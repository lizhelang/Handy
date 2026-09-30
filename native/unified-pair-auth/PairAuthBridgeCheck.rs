//! 独立 Rust→Swift 桥实验。只使用脚本创建的临时密钥、manifest 和 socket。
#[path = "../../src-tauri/src/native_pair_auth.rs"]
mod native_pair_auth;

use native_pair_auth::{
    EmbeddedPairTrust, EmbeddedReleasePairTrust, PairAuthError, PairManifest, PeerRole,
};
use std::io::{Read, Write};
use std::os::fd::AsFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

static PUBLIC_KEY: &[u8; 65] = include_bytes!(env!("UIPA_FIXTURE_PUBLIC_KEY"));
#[cfg(pair_host_fixture)]
const LOCAL_ROLE: PeerRole = PeerRole::Inputia;
#[cfg(not(pair_host_fixture))]
const LOCAL_ROLE: PeerRole = PeerRole::Handy;
const PEER_ROLE: PeerRole = match LOCAL_ROLE {
    PeerRole::Handy => PeerRole::Inputia,
    PeerRole::Inputia => PeerRole::Handy,
};

fn trust(public_key: &'static [u8; 65], profile: &'static str) -> EmbeddedPairTrust {
    EmbeddedPairTrust::from_build_constants(
        public_key,
        "bridge-fixture-key",
        "trial-20260905",
        profile,
        LOCAL_ROLE,
    )
}
fn release_trust(
    public_key: &'static [u8; 65],
    product: &'static str,
    release: &'static str,
) -> EmbeddedReleasePairTrust {
    EmbeddedReleasePairTrust::from_build_constants(
        public_key,
        "bridge-fixture-key",
        product,
        release,
        1,
        LOCAL_ROLE,
    )
}
fn manifest(path: &str, release: bool) -> PairManifest {
    if release {
        return PairManifest::load_release(
            &std::fs::read(path).unwrap(),
            &release_trust(PUBLIC_KEY, "com.inputia", "inputia-bridge-fixture"),
        )
        .expect("valid release bridge manifest");
    }
    PairManifest::load(
        &std::fs::read(path).unwrap(),
        &trust(PUBLIC_KEY, "unified-candidate:trial-20260905"),
    )
    .expect("valid signed bridge manifest")
}
fn component_path(executable: &Path) -> std::path::PathBuf {
    let canonical = executable.canonicalize().unwrap();
    canonical
        .ancestors()
        .find(|path| path.extension().is_some_and(|value| value == "app"))
        .unwrap_or(&canonical)
        .to_path_buf()
}
fn copy_component(source: &Path, destination: &Path) {
    if source.is_dir() {
        std::fs::create_dir(destination).unwrap();
        for entry in std::fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            copy_component(&entry.path(), &destination.join(entry.file_name()));
        }
    } else {
        std::fs::copy(source, destination).unwrap();
    }
}
fn configure(stream: &UnixStream) {
    // Darwin accept 继承监听 socket 的 O_NONBLOCK；实验收发采用有界阻塞 I/O。
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
}
fn authenticate(stream: &UnixStream, manifest: &PairManifest, expected: &Path) -> bool {
    let started = Instant::now();
    eprintln!("bridge_fixture_authentication_begin local_role={LOCAL_ROLE:?}");
    let result = if manifest.release_binding().is_some() {
        assert!(matches!(
            manifest.authenticate(stream.as_fd(), PEER_ROLE),
            Err(PairAuthError::InvalidArgument)
        ));
        manifest.authenticate_at(stream.as_fd(), PEER_ROLE, expected)
    } else {
        manifest.authenticate(stream.as_fd(), PEER_ROLE)
    };
    eprintln!(
        "bridge_fixture_authentication_end local_role={LOCAL_ROLE:?} elapsed_ms={} accepted={}",
        started.elapsed().as_millis(),
        result.is_ok()
    );
    match result {
        Ok(peer) => {
            assert_eq!(peer.role(), PEER_ROLE);
            assert!(peer.audit_token().iter().any(|value| *value != 0));
            unsafe extern "C" {
                fn geteuid() -> u32;
            }
            // SAFETY: geteuid 无参数，只读当前进程有效 UID。
            assert_eq!(peer.uid(), unsafe { geteuid() });
            true
        }
        Err(PairAuthError::PeerRejected) => false,
        Err(error) => panic!("unexpected bridge error: {error}"),
    }
}
fn main() {
    assert!(
        native_pair_auth::candidate_build_trust("unified-candidate:trial-20260905")
            .unwrap()
            .is_none()
    );
    assert!(native_pair_auth::release_build_trust().unwrap().is_none());
    let args: Vec<_> = std::env::args().collect();
    if args.len() == 5 && ["client", "release-client"].contains(&args[1].as_str()) {
        let mut stream = UnixStream::connect(&args[2]).unwrap();
        configure(&stream);
        let accepted = authenticate(
            &stream,
            &manifest(&args[3], args[1] == "release-client"),
            Path::new(&args[4]),
        );
        stream.write_all(&[u8::from(accepted)]).unwrap();
        let mut reply = [0];
        stream.read_exact(&mut reply).unwrap();
        std::process::exit(if accepted && reply[0] == 1 { 0 } else { 3 });
    }
    assert!(
        args.len() == 5 && ["self-check", "release-self-check"].contains(&args[1].as_str()),
        "bridge fixture arguments"
    );
    let release = args[1] == "release-self-check";
    let bytes = std::fs::read(&args[2]).unwrap();
    let loaded = manifest(&args[2], release);
    assert_eq!(loaded.release_binding().is_some(), release);
    let release_trust = release_trust(PUBLIC_KEY, "com.inputia", "inputia-bridge-fixture");
    assert_eq!(release_trust.release_binding().product_id, "com.inputia");
    if release {
        for wrong in [
            self::release_trust(&[4; 65], "com.inputia", "inputia-bridge-fixture"),
            self::release_trust(PUBLIC_KEY, "other", "inputia-bridge-fixture"),
            self::release_trust(PUBLIC_KEY, "com.inputia", "inputia-other"),
        ] {
            assert!(matches!(
                PairManifest::load_release(&bytes, &wrong),
                Err(PairAuthError::ManifestRejected)
            ));
        }
        assert!(PairManifest::load_release(b"{}", &release_trust).is_err());
        assert!(PairManifest::load_release(&[], &release_trust).is_err());
    } else {
        assert!(matches!(
            PairManifest::load_release(&bytes, &release_trust),
            Err(PairAuthError::ManifestRejected)
        ));
    }
    assert!(matches!(
        PairManifest::load(&bytes, &trust(&[4; 65], "unified-candidate:trial-20260905")),
        Err(PairAuthError::ManifestRejected)
    ));
    assert!(matches!(
        PairManifest::load(&bytes, &trust(PUBLIC_KEY, "unified-candidate:other")),
        Err(PairAuthError::ManifestRejected)
    ));
    assert!(matches!(
        PairManifest::load(
            b"{}",
            &trust(PUBLIC_KEY, "unified-candidate:trial-20260905")
        ),
        Err(PairAuthError::ManifestRejected)
    ));
    assert!(matches!(
        PairManifest::load(&[], &trust(PUBLIC_KEY, "unified-candidate:trial-20260905")),
        Err(PairAuthError::InvalidArgument)
    ));
    let file = std::fs::File::open(&args[2]).unwrap();
    if release {
        assert!(matches!(
            loaded.authenticate_at(file.as_fd(), PEER_ROLE, Path::new(&args[3])),
            Err(PairAuthError::PeerRejected)
        ));
        assert!(matches!(
            loaded.authenticate_at(file.as_fd(), PEER_ROLE, Path::new("relative")),
            Err(PairAuthError::InvalidArgument)
        ));
    } else {
        assert!(matches!(
            loaded.authenticate(file.as_fd(), PEER_ROLE),
            Err(PairAuthError::PeerRejected)
        ));
        assert!(matches!(
            loaded.authenticate_at(file.as_fd(), PEER_ROLE, Path::new(&args[3])),
            Err(PairAuthError::InvalidArgument)
        ));
    }
    assert!(matches!(
        loaded.authenticate(file.as_fd(), LOCAL_ROLE),
        Err(PairAuthError::InvalidArgument)
    ));
    for _ in 0..50 {
        drop(manifest(&args[2], release));
    }
    let root = Path::new(&args[2]).parent().unwrap();
    let own_path = component_path(&std::env::current_exe().unwrap());
    let host_path = component_path(Path::new(&args[3]));
    let alias = root.join("host-alias");
    let copy = root.join(if host_path.is_dir() {
        "HostCopy.app"
    } else {
        "HostCopy"
    });
    let copied_executable = if host_path.is_dir() {
        copy.join("Contents/MacOS/HostFixture")
    } else {
        copy.clone()
    };
    if release {
        std::os::unix::fs::symlink(&host_path, &alias).unwrap();
        copy_component(&host_path, &copy);
    }
    let mut cases = vec![
        (
            "strict",
            Path::new(&args[3]),
            host_path.as_path(),
            true,
            true,
        ),
        (
            "weak",
            Path::new(&args[4]),
            Path::new(&args[4]),
            false,
            false,
        ),
    ];
    if release {
        cases.extend([
            (
                "copied",
                copied_executable.as_path(),
                host_path.as_path(),
                false,
                true,
            ),
            ("symlink", Path::new(&args[3]), alias.as_path(), false, true),
            (
                "wrongpath",
                Path::new(&args[3]),
                own_path.as_path(),
                false,
                true,
            ),
        ]);
    }
    for (label, executable, expected_path, expected, client_expected) in cases {
        let socket = root.join(format!("{label}.sock"));
        let listener = UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut child = Command::new(executable)
            .args([
                if release { "release-client" } else { "client" },
                socket.to_str().unwrap(),
                &args[2],
                own_path.to_str().unwrap(),
            ])
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("fixture accept: {error}");
                }
            }
        };
        configure(&stream);
        let accepted = authenticate(&stream, &loaded, expected_path);
        let mut peer = [0];
        stream.read_exact(&mut peer).unwrap();
        stream.write_all(&[u8::from(accepted)]).unwrap();
        assert_eq!(accepted, expected);
        assert_eq!(peer[0], u8::from(client_expected));
        assert_eq!(
            child.wait().unwrap().code(),
            Some(if expected { 0 } else { 3 })
        );
        drop(stream);
        drop(listener);
        std::fs::remove_file(socket).unwrap();
    }
    if release {
        std::fs::remove_file(alias).unwrap();
        if copy.is_dir() {
            std::fs::remove_dir_all(copy).unwrap();
        } else {
            std::fs::remove_file(copy).unwrap();
        }
    }
    println!("rust_swift_pair_bridge=pass version={} path_binding={} manifest_valid=true wrong_key_rejected=true invalid_fd_rejected=true raii_load_drop=50 strict_runtime_both_directions=true allowlisted_weak_runtime_rejected=true fixture_only=true", if release { 2 } else { 1 }, release);
}
