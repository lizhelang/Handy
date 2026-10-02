use inputia_updater::archive::{extract_zip, ArchiveDigest, ArchiveLimits};
use rawzip::{
    extra_fields::ExtraFieldId, path::EntryPath, CompressionMethod, Header, ZipArchiveWriter,
};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Cursor, Write},
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::PathBuf,
    sync::atomic::AtomicBool,
};

struct Item<'a> {
    path: &'a str,
    body: &'a [u8],
    mode: u32,
    deflate: bool,
}
fn item<'a>(path: &'a str, body: &'static [u8]) -> Item<'a> {
    Item {
        path,
        body,
        mode: 0o100644,
        deflate: false,
    }
}
fn zip_raw(items: &[Item<'_>], extra: Option<(u16, &[u8])>) -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    let mut writer = ZipArchiveWriter::new(&mut bytes);
    for item in items {
        if item.mode & 0o170000 == 0o040000 {
            writer
                .new_dir(EntryPath::verbatim(item.path.as_bytes()))
                .unix_permissions(item.mode)
                .create()
                .unwrap();
            continue;
        }
        let method = if item.deflate {
            CompressionMethod::DEFLATE
        } else {
            CompressionMethod::STORE
        };
        let mut builder = writer
            .new_file(EntryPath::verbatim(item.path.as_bytes()))
            .unix_permissions(item.mode)
            .compression_method(method);
        if let Some((id, data)) = extra {
            builder = builder
                .extra_field(ExtraFieldId::new(id), data, Header::default())
                .unwrap();
        }
        let (mut entry, config) = builder.start().unwrap();
        if item.deflate {
            let encoder =
                flate2::write::DeflateEncoder::new(&mut entry, flate2::Compression::default());
            let mut payload = config.wrap(encoder);
            payload.write_all(item.body).unwrap();
            let (encoder, descriptor) = payload.finish().unwrap();
            encoder.finish().unwrap();
            entry.finish(descriptor).unwrap();
        } else {
            let mut payload = config.wrap(&mut entry);
            payload.write_all(item.body).unwrap();
            let (_, descriptor) = payload.finish().unwrap();
            entry.finish(descriptor).unwrap();
        }
    }
    writer.finish().unwrap();
    bytes.into_inner()
}
fn zip(items: &[Item<'_>], extra: Option<(u16, &[u8])>) -> Vec<u8> {
    let names: Vec<_> = items
        .iter()
        .map(|entry| format!("Inputia.app/{}", entry.path))
        .collect();
    let mut entries: Vec<_> = items
        .iter()
        .zip(&names)
        .map(|(entry, name)| Item {
            path: name,
            body: entry.body,
            mode: entry.mode,
            deflate: entry.deflate,
        })
        .collect();
    entries.push(Item {
        path: "Inputia.app/",
        body: b"",
        mode: 0o040755,
        deflate: false,
    });
    zip_raw(&entries, extra)
}
struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    source: PathBuf,
    destination: PathBuf,
    expected: ArchiveDigest,
}
impl Fixture {
    fn new(bytes: &[u8]) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let source = root.join("source.zip");
        fs::write(&source, bytes).unwrap();
        fs::set_permissions(&source, fs::Permissions::from_mode(0o600)).unwrap();
        let destination = root.join("extracted");
        let expected = ArchiveDigest {
            sha256: format!("{:x}", Sha256::digest(bytes)),
            size: bytes.len() as u64,
        };
        Self {
            _temp: temp,
            root,
            source,
            destination,
            expected,
        }
    }
    fn extract(
        &self,
        limits: &ArchiveLimits,
    ) -> inputia_updater::Result<inputia_updater::archive::ExtractedArchive> {
        extract_zip(
            File::open(&self.source).unwrap(),
            &self.expected,
            "Inputia.app",
            &self.destination,
            unsafe { libc::geteuid() },
            limits,
            &AtomicBool::new(false),
        )
    }
}
fn central(bytes: &[u8]) -> usize {
    bytes.windows(4).position(|b| b == b"PK\x01\x02").unwrap()
}
fn reject_before_create(bytes: &[u8]) {
    let fixture = Fixture::new(bytes);
    assert!(fixture.extract(&ArchiveLimits::default()).is_err());
    assert!(!fixture.destination.exists());
}

#[test]
fn extracts_real_stored_deflate_and_framework_links_preserving_modes() {
    let bytes = zip(
        &[
            Item {
                path: "App.app/Contents/MacOS/run",
                body: b"executable",
                mode: 0o100755,
                deflate: true,
            },
            item(
                "App.app/Contents/Frameworks/F.framework/Versions/A/F",
                b"framework",
            ),
            Item {
                path: "App.app/Contents/Frameworks/F.framework/Versions/Current",
                body: b"A",
                mode: 0o120777,
                deflate: false,
            },
            Item {
                path: "App.app/Contents/Frameworks/F.framework/F",
                body: b"Versions/Current/F",
                mode: 0o120777,
                deflate: false,
            },
        ],
        None,
    );
    let fixture = Fixture::new(&bytes);
    let proof = fixture.extract(&ArchiveLimits::default()).unwrap();
    assert_eq!(proof.source(), &fixture.expected);
    assert_eq!(proof.destination(), fixture.destination);
    assert_eq!(
        fs::metadata(&fixture.destination).unwrap().mode() & 0o777,
        0o700
    );
    let executable = fixture
        .destination
        .join("Inputia.app/App.app/Contents/MacOS/run");
    assert_eq!(fs::metadata(&executable).unwrap().mode() & 0o777, 0o755);
    assert_eq!(fs::read(&executable).unwrap(), b"executable");
    let link = fixture
        .destination
        .join("Inputia.app/App.app/Contents/Frameworks/F.framework/F");
    assert_eq!(
        fs::read_link(&link).unwrap(),
        PathBuf::from("Versions/Current/F")
    );
    assert_eq!(fs::read(link).unwrap(), b"framework");
    assert_eq!(
        proof.tree(),
        &inputia_updater::fingerprint(&fixture.destination, unsafe { libc::geteuid() }).unwrap()
    );
    assert_eq!(
        proof.bundle_tree(),
        &inputia_updater::fingerprint(&fixture.destination.join("Inputia.app"), unsafe {
            libc::geteuid()
        })
        .unwrap()
    );
    assert_eq!(proof.bundle_path(), fixture.destination.join("Inputia.app"));
    proof
        .verify_bound_bundle(unsafe { libc::geteuid() }, &|| Ok(()))
        .unwrap();
    fs::write(&executable, b"tampered").unwrap();
    assert!(proof
        .verify_bound_bundle(unsafe { libc::geteuid() }, &|| Ok(()))
        .is_err());
    assert!(proof.source_read_bytes() >= fixture.expected.size * 2);
}
#[test]
fn unsafe_names_and_metadata_are_rejected_before_creating_target() {
    for name in [
        "../escape",
        "/absolute",
        "a/../escape",
        "a\\b",
        "a\nb",
        "a//b",
        "C:drive",
        "._fork",
        "__MACOSX/data",
        "a/._fork",
    ] {
        reject_before_create(&zip(&[item(name, b"test")], None));
    }
    for extra in [
        (0x000d, b"hardlink".as_slice()),
        (0x07c8, b"mac metadata".as_slice()),
        (0x9999, b"unknown".as_slice()),
        (0x0001, b"zip64".as_slice()),
    ] {
        reject_before_create(&zip(&[item("safe", b"test")], Some(extra)));
    }
}
#[test]
fn duplicate_case_unicode_and_implicit_directory_aliases_rejected() {
    for (a, b) in [
        ("same", "same"),
        ("A", "a"),
        ("É", "E\u{301}"),
        ("straße", "STRASSE"),
        ("A/one", "a/two"),
        ("Ａ", "A"),
    ] {
        reject_before_create(&zip(&[item(a, b"one"), item(b, b"two")], None));
    }
}
#[test]
fn file_parent_and_escaping_missing_cyclic_links_rejected() {
    reject_before_create(&zip(
        &[item("parent", b"file"), item("parent/child", b"no")],
        None,
    ));
    for link in ["../outside", "/outside", "missing", "link", "a\\b"] {
        reject_before_create(&zip(
            &[Item {
                path: "link",
                body: link.as_bytes(),
                mode: 0o120777,
                deflate: false,
            }],
            None,
        ));
    }
    reject_before_create(&zip(
        &[
            Item {
                path: "link",
                body: b"target",
                mode: 0o120777,
                deflate: false,
            },
            item("target", b"data"),
            item("link/child", b"unsafe"),
        ],
        None,
    ));
}
#[test]
fn devices_hardlink_types_and_unsafe_permissions_rejected() {
    for mode in [
        0o020600, 0o060600, 0o010600, 0o140600, 0o106755, 0o100666, 0o100000,
    ] {
        reject_before_create(&zip(
            &[Item {
                path: "entry",
                body: b"test",
                mode,
                deflate: false,
            }],
            None,
        ));
    }
}
#[test]
fn headers_cannot_hide_encryption_method_or_name_mismatch() {
    let source = zip(&[item("safe", b"test")], None);
    let cd = central(&source);
    for offset in [6, cd + 8] {
        let mut bytes = source.clone();
        bytes[offset] |= 1;
        reject_before_create(&bytes);
    }
    let mut bytes = source.clone();
    bytes[8..10].copy_from_slice(&99u16.to_le_bytes());
    bytes[cd + 10..cd + 12].copy_from_slice(&99u16.to_le_bytes());
    reject_before_create(&bytes);
    let mut bytes = source.clone();
    bytes[30] = b'X';
    reject_before_create(&bytes);
    let mut bytes = source.clone();
    bytes.push(0);
    reject_before_create(&bytes);
    let mut bytes = source;
    bytes[cd + 16] ^= 1;
    reject_before_create(&bytes);
}
#[test]
fn forged_size_or_descriptor_and_overlapping_entries_rejected() {
    let source = zip(&[item("one", b"data"), item("two", b"data")], None);
    let cd = central(&source);
    let mut bytes = source.clone();
    bytes[cd + 24..cd + 28].copy_from_slice(&1000u32.to_le_bytes());
    reject_before_create(&bytes);
    let mut bytes = source.clone();
    let second = cd + 46 + "Inputia.app/one".len();
    bytes[second + 42..second + 46].copy_from_slice(&0u32.to_le_bytes());
    reject_before_create(&bytes);
    let mut bytes = source.clone();
    let end = bytes.len();
    bytes[end - 12..end - 10].copy_from_slice(&1u16.to_le_bytes());
    reject_before_create(&bytes);
}
#[test]
fn damaged_payload_never_returns_proof_and_existing_target_is_untouched() {
    let mut bytes = zip(&[item("file", b"payload")], None);
    let payload = 30 + "Inputia.app/file".len();
    bytes[payload] ^= 1;
    let fixture = Fixture::new(&bytes);
    assert!(fixture.extract(&ArchiveLimits::default()).is_err());
    let fixture = Fixture::new(&zip(&[item("file", b"valid")], None));
    fs::create_dir(&fixture.destination).unwrap();
    fs::write(fixture.destination.join("keep"), b"new user file").unwrap();
    assert!(fixture.extract(&ArchiveLimits::default()).is_err());
    assert_eq!(
        fs::read(fixture.destination.join("keep")).unwrap(),
        b"new user file"
    );
    assert!(!fixture.destination.join("Inputia.app/file").exists());
}
#[test]
fn all_budgets_and_cancellation_fail_closed() {
    let bytes = zip(
        &[item("a/b/c", b"ten bytes!"), item("second", b"data")],
        None,
    );
    let variants = [
        ArchiveLimits {
            max_entries: 1,
            ..Default::default()
        },
        ArchiveLimits {
            max_depth: 2,
            ..Default::default()
        },
        ArchiveLimits {
            max_file_bytes: 3,
            ..Default::default()
        },
        ArchiveLimits {
            max_unpacked_bytes: 12,
            ..Default::default()
        },
        ArchiveLimits {
            max_source_bytes: 10,
            ..Default::default()
        },
        ArchiveLimits {
            max_source_read_bytes: 1,
            ..Default::default()
        },
    ];
    for limits in variants {
        let f = Fixture::new(&bytes);
        assert!(f.extract(&limits).is_err());
        assert!(!f.destination.exists());
    }
    let f = Fixture::new(&bytes);
    assert!(extract_zip(
        File::open(&f.source).unwrap(),
        &f.expected,
        "Inputia.app",
        &f.destination,
        unsafe { libc::geteuid() },
        &ArchiveLimits::default(),
        &AtomicBool::new(true)
    )
    .is_err());
    assert!(!f.destination.exists());
    let body = vec![0; 100_000];
    let f = Fixture::new(&zip(
        &[Item {
            path: "bomb",
            body: &body,
            mode: 0o100644,
            deflate: true,
        }],
        None,
    ));
    assert!(f.extract(&ArchiveLimits::default()).is_err());
    assert!(!f.destination.exists());
}
#[test]
fn source_digest_hardlink_and_destination_symlink_cannot_bypass_checks() {
    let bytes = zip(&[item("file", b"valid")], None);
    let mut fixture = Fixture::new(&bytes);
    fixture.expected.sha256 = "0".repeat(64);
    assert!(fixture.extract(&ArchiveLimits::default()).is_err());
    assert!(!fixture.destination.exists());
    let fixture = Fixture::new(&bytes);
    fs::hard_link(&fixture.source, fixture.root.join("alias")).unwrap();
    assert!(fixture.extract(&ArchiveLimits::default()).is_err());
    let fixture = Fixture::new(&bytes);
    let outside = fixture.root.join("outside");
    fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, &fixture.destination).unwrap();
    assert!(fixture.extract(&ArchiveLimits::default()).is_err());
    assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
}
#[cfg(target_os = "macos")]
#[test]
fn actual_ditto_plain_profile_succeeds_and_default_appledouble_is_rejected() {
    use std::process::Command;
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let app = root.join("Inputia.app");
    fs::create_dir_all(app.join("Versions/A")).unwrap();
    let run = app.join("Versions/A/run");
    fs::write(&run, b"synthetic binary").unwrap();
    fs::set_permissions(&run, fs::Permissions::from_mode(0o755)).unwrap();
    std::os::unix::fs::symlink("A", app.join("Versions/Current")).unwrap();
    std::os::unix::fs::symlink("Versions/Current/run", app.join("run")).unwrap();
    assert!(Command::new("/usr/bin/xattr")
        .args(["-w", "com.inputia.synthetic", "fixture"])
        .arg(&run)
        .status()
        .unwrap()
        .success());
    for plain in [false, true] {
        let output = root.join(if plain { "plain.zip" } else { "default.zip" });
        let mut cmd = Command::new("/usr/bin/ditto");
        cmd.args(["-c", "-k", "--keepParent"]);
        if plain {
            cmd.args(["--norsrc", "--noextattr", "--noacl", "--noqtn"]);
        }
        assert!(cmd.arg(&app).arg(&output).status().unwrap().success());
        let fixture = Fixture::new(&fs::read(output).unwrap());
        let result = fixture.extract(&ArchiveLimits::default());
        assert_eq!(result.is_ok(), plain, "{result:?}");
        if !plain {
            assert!(!fixture.destination.exists());
        }
    }
}

#[test]
fn multi_volume_and_unsupported_versions_are_rejected() {
    let source = zip(&[item("safe", b"bytes")], None);
    let cd = central(&source);
    for offset in [
        4,
        cd + 6,
        cd + 34,
        source.len() - 18,
        source.len() - 16,
        source.len() - 14,
    ] {
        let mut bytes = source.clone();
        bytes[offset..offset + 2].copy_from_slice(&99u16.to_le_bytes());
        reject_before_create(&bytes);
    }
}
#[test]
fn malformed_extra_and_actual_expansion_crc_mismatches_are_rejected() {
    let mut bytes = zip(&[item("safe", b"bytes")], Some((0x5855, &[0; 12])));
    let cd = central(&bytes);
    bytes[cd + 46 + "Inputia.app/safe".len() + 2..cd + 46 + "Inputia.app/safe".len() + 4]
        .copy_from_slice(&65535u16.to_le_bytes());
    reject_before_create(&bytes);
    let payload = b"abcdabcdabcdabcd";
    let mut bytes = zip(
        &[Item {
            path: "safe",
            body: payload,
            mode: 0o100644,
            deflate: true,
        }],
        None,
    );
    let cd = central(&bytes);
    let dd = bytes.windows(4).position(|b| b == b"PK\x07\x08").unwrap();
    bytes[cd + 24..cd + 28].copy_from_slice(&4u32.to_le_bytes());
    bytes[dd + 12..dd + 16].copy_from_slice(&4u32.to_le_bytes());
    let fixture = Fixture::new(&bytes);
    assert!(fixture.extract(&ArchiveLimits::default()).is_err());
}

#[test]
fn caller_root_is_explicit_and_links_cannot_reach_sibling_components() {
    let missing = zip_raw(&[item("Inputia.app/file", b"file")], None);
    reject_before_create(&missing);
    let multi = zip_raw(
        &[
            Item {
                path: "Inputia.app/",
                body: b"",
                mode: 0o040755,
                deflate: false,
            },
            Item {
                path: "Other.app/",
                body: b"",
                mode: 0o040755,
                deflate: false,
            },
            Item {
                path: "Inputia.app/link",
                body: b"../Other.app",
                mode: 0o120777,
                deflate: false,
            },
            item("Other.app/file", b"data"),
        ],
        None,
    );
    reject_before_create(&multi);
    reject_before_create(&zip(
        &[
            item("data", b"bytes"),
            Item {
                path: "link",
                body: b"..",
                mode: 0o120777,
                deflate: false,
            },
        ],
        None,
    ));
    let f = Fixture::new(&zip(&[item("data", b"bytes")], None));
    assert!(extract_zip(
        File::open(&f.source).unwrap(),
        &f.expected,
        "Wrong.app",
        &f.destination,
        unsafe { libc::geteuid() },
        &ArchiveLimits::default(),
        &AtomicBool::new(false)
    )
    .is_err());
    assert!(!f.destination.exists());
}
#[test]
fn crc_valid_deflate_without_final_block_is_rejected() {
    let mut bytes = zip(
        &[Item {
            path: "safe",
            body: b"text payload",
            mode: 0o100644,
            deflate: true,
        }],
        None,
    );
    let archive = rawzip::ZipArchive::from_slice(bytes.as_slice()).unwrap();
    let entry = archive.entries().next_entry().unwrap().unwrap().wayfinder();
    let (start, end) = archive.get_entry(entry).unwrap().compressed_data_range();
    let data = (end - 2) as usize;
    assert_eq!(
        &bytes[data..end as usize],
        &[3, 0],
        "{:02x?}",
        &bytes[start as usize..end as usize]
    );
    bytes[data] &= !1; // 保留正文/CRC/长度，只使完整流缺失最后块标志。
    let mut legacy = flate2::read::DeflateDecoder::new(&bytes[start as usize..end as usize]);
    let mut decoded = Vec::new();
    std::io::Read::read_to_end(&mut legacy, &mut decoded).unwrap();
    assert_eq!(&decoded, b"text payload"); // 高层 EOF 不足以证明终止标记。
    let fixture = Fixture::new(&bytes);
    assert!(fixture.extract(&ArchiveLimits::default()).is_err());
}

#[test]
fn implicit_nodes_and_path_memory_are_bounded_before_creation() {
    let f = Fixture::new(&zip(&[item("a/b/c/d/e", b"bytes")], None));
    assert!(f
        .extract(&ArchiveLimits {
            max_entries: 4,
            ..Default::default()
        })
        .is_err());
    assert!(!f.destination.exists());
    let f = Fixture::new(&zip(&[item("longer/path/file", b"bytes")], None));
    assert!(f
        .extract(&ArchiveLimits {
            max_path_bytes: 30,
            ..Default::default()
        })
        .is_err());
    assert!(!f.destination.exists());
}
#[cfg(target_os = "macos")]
#[test]
fn inherited_acl_cannot_make_the_new_root_non_private() {
    let f = Fixture::new(&zip(&[item("file", b"bytes")], None));
    assert!(std::process::Command::new("/bin/chmod")
        .args([
            "+a",
            "everyone allow read,write,add_file,add_subdirectory,file_inherit,directory_inherit"
        ])
        .arg(&f.root)
        .status()
        .unwrap()
        .success());
    let result = f.extract(&ArchiveLimits::default());
    // 仅清除本测试自行添加的临时 ACL，以便 TempDir 能正常释放。
    assert!(std::process::Command::new("/bin/chmod")
        .args(["-RN"])
        .arg(&f.root)
        .status()
        .unwrap()
        .success());
    assert!(result.is_err());
    assert!(!f.destination.join("Inputia.app/file").exists());
}
