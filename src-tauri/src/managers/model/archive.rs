//! 有界模型归档解压：先核物理条目和扩展元数据，再解释最终路径。
use super::storage::{self, ModelStorageBudget};
use anyhow::Result;
use flate2::read::GzDecoder;
use std::{fs::File, io::Seek, path::Path};
use tar::Archive;

pub(super) fn unpack(
    archive_path: &Path,
    destination: &Path,
    budget: &ModelStorageBudget,
) -> Result<()> {
    budget.validate().map_err(anyhow::Error::msg)?;
    let mut tar_gz = File::open(archive_path)?;
    preflight(&mut tar_gz, budget)?;
    tar_gz.rewind()?;
    let tar = GzDecoder::new(tar_gz);
    let mut archive = Archive::new(tar);

    let mut expanded = 0_u64;
    let mut count = 0_u32;
    for entry in archive.entries()? {
        let mut entry = entry?;
        count = count
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("model_archive_count_overflow"))?;
        if count > budget.max_archive_entries {
            return Err(storage::ModelStorageFailure {
                code: storage::StorageFailureCode::ArchiveEntryLimit,
                required_bytes: u64::from(count),
                limit_bytes: u64::from(budget.max_archive_entries),
                resumable: true,
            }
            .into());
        }
        let entry_path = entry.path()?.into_owned();
        validate_archive_entry_path(&entry_path)?;
        let entry_type = entry.header().entry_type();
        if !entry_type.is_file() && !entry_type.is_dir() {
            return Err(anyhow::anyhow!(
                "model archive special entries are not permitted: {}",
                entry_path.display()
            ));
        }
        let size = entry.size();
        expanded = expanded
            .checked_add(size)
            .ok_or_else(|| anyhow::anyhow!("model_archive_size_overflow"))?;
        if expanded > budget.max_extracted_bytes {
            return Err(storage::ModelStorageFailure {
                code: storage::StorageFailureCode::ArchiveSizeLimit,
                required_bytes: expanded,
                limit_bytes: budget.max_extracted_bytes,
                resumable: true,
            }
            .into());
        }
        if !entry.unpack_in(destination)? {
            return Err(anyhow::anyhow!(
                "model archive entry escaped extraction directory: {}",
                entry_path.display()
            ));
        }
    }
    Ok(())
}

fn preflight(file: &mut File, budget: &ModelStorageBudget) -> Result<()> {
    const MAX_METADATA_ENTRY: u64 = 1024 * 1024;
    const MAX_METADATA_TOTAL: u64 = 16 * 1024 * 1024;
    let mut archive = Archive::new(GzDecoder::new(file));
    let mut metadata_bytes = 0_u64;
    let mut expanded = 0_u64;
    for (index, entry) in archive.entries()?.raw(true).enumerate() {
        if index >= budget.max_archive_entries as usize {
            return Err(storage::ModelStorageFailure {
                code: storage::StorageFailureCode::ArchiveEntryLimit,
                required_bytes: index as u64 + 1,
                limit_bytes: u64::from(budget.max_archive_entries),
                resumable: true,
            }
            .into());
        }
        let mut entry = entry?;
        let kind = entry.header().entry_type();
        let size = entry.size();
        if kind.is_gnu_longname() || kind.is_pax_local_extensions() {
            metadata_bytes = metadata_bytes
                .checked_add(size)
                .ok_or_else(|| anyhow::anyhow!("model_archive_metadata_overflow"))?;
            if size > MAX_METADATA_ENTRY || metadata_bytes > MAX_METADATA_TOTAL {
                return Err(storage::ModelStorageFailure {
                    code: storage::StorageFailureCode::ArchiveMetadataLimit,
                    required_bytes: size.max(metadata_bytes),
                    limit_bytes: if size > MAX_METADATA_ENTRY {
                        MAX_METADATA_ENTRY
                    } else {
                        MAX_METADATA_TOTAL
                    },
                    resumable: true,
                }
                .into());
            }
            if kind.is_pax_local_extensions() {
                for field in entry
                    .pax_extensions()?
                    .ok_or_else(|| anyhow::anyhow!("model_archive_pax_invalid"))?
                {
                    let field = field?;
                    // size/sparse/link会改变两个遍历的流边界或引入链接，不能继承预检结论。
                    if !matches!(
                        field.key()?,
                        "path"
                            | "mtime"
                            | "atime"
                            | "ctime"
                            | "uid"
                            | "gid"
                            | "uname"
                            | "gname"
                            | "charset"
                            | "comment"
                    ) {
                        anyhow::bail!("model_archive_pax_field_unsupported");
                    }
                }
            }
        } else if kind.is_file() || kind.is_dir() {
            expanded = expanded
                .checked_add(size)
                .ok_or_else(|| anyhow::anyhow!("model_archive_size_overflow"))?;
            if expanded > budget.max_extracted_bytes {
                return Err(storage::ModelStorageFailure {
                    code: storage::StorageFailureCode::ArchiveSizeLimit,
                    required_bytes: expanded,
                    limit_bytes: budget.max_extracted_bytes,
                    resumable: true,
                }
                .into());
            }
        } else {
            anyhow::bail!("model_archive_entry_unsupported");
        }
    }
    Ok(())
}

fn validate_archive_entry_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() {
        return Err(anyhow::anyhow!("model archive contains an empty path"));
    }
    for component in path.components() {
        if matches!(
            component,
            std::path::Component::ParentDir
                | std::path::Component::RootDir
                | std::path::Component::Prefix(_)
        ) {
            return Err(anyhow::anyhow!("model archive contains an unsafe path"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{write::GzEncoder, Compression};
    use std::{fs, io::Write};
    type Builder = tar::Builder<GzEncoder<File>>;
    fn fixture(
        write: impl FnOnce(&mut Builder),
    ) -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("model.tar.gz");
        let mut builder = tar::Builder::new(GzEncoder::new(
            File::create(&path).unwrap(),
            Compression::default(),
        ));
        write(&mut builder);
        builder.into_inner().unwrap().finish().unwrap();
        let destination = temp.path().join("unpacked");
        fs::create_dir(&destination).unwrap();
        (temp, path, destination)
    }
    fn append(builder: &mut Builder, name: &str, content: &[u8]) {
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o600);
        header.set_cksum();
        builder.append_data(&mut header, name, content).unwrap();
    }
    #[test]
    fn python_style_pax_and_gnu_long_paths_remain_supported() {
        let long = format!("nested/{}", "a".repeat(140));
        let (_temp, path, destination) = fixture(|builder| {
            builder
                .append_pax_extensions([
                    ("mtime", b"1234.567".as_slice()),
                    ("path", b"nested/pax-model".as_slice()),
                ])
                .unwrap();
            append(builder, "model", b"pax");
            append(builder, &long, b"gnu");
        });
        unpack(&path, &destination, &ModelStorageBudget::default()).unwrap();
        assert_eq!(
            fs::read(destination.join("nested/pax-model")).unwrap(),
            b"pax"
        );
        assert_eq!(fs::read(destination.join(long)).unwrap(), b"gnu");
    }
    #[test]
    fn cumulative_size_and_entry_limits_stop_before_any_extraction() {
        for count_limit in [false, true] {
            let (_temp, path, destination) = fixture(|builder| {
                append(builder, "one", b"12345");
                append(builder, "two", b"67890");
            });
            let mut budget = ModelStorageBudget::default();
            if count_limit {
                budget.max_archive_entries = 1;
            } else {
                budget.max_extracted_bytes = 9;
            }
            let error = unpack(&path, &destination, &budget).unwrap_err();
            let actual = error
                .downcast_ref::<storage::ModelStorageFailure>()
                .unwrap();
            assert_eq!(
                actual.code,
                if count_limit {
                    storage::StorageFailureCode::ArchiveEntryLimit
                } else {
                    storage::StorageFailureCode::ArchiveSizeLimit
                }
            );
            assert!(actual.resumable);
            assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
            assert!(path.is_file());
        }
    }
    #[test]
    fn oversized_extension_is_rejected_before_reading_its_body() {
        let (_temp, path, destination) = fixture(|builder| {
            let mut header = tar::Header::new_gnu();
            header.set_path("extension").unwrap();
            header.set_entry_type(tar::EntryType::GNULongName);
            header.set_mode(0o600);
            header.set_size(1024 * 1024 + 1);
            header.set_cksum();
            builder.get_mut().write_all(header.as_bytes()).unwrap();
        });
        let error = unpack(&path, &destination, &ModelStorageBudget::default()).unwrap_err();
        assert_eq!(
            error
                .downcast_ref::<storage::ModelStorageFailure>()
                .unwrap()
                .code,
            storage::StorageFailureCode::ArchiveMetadataLimit
        );
        assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
    }
    #[test]
    fn pax_size_cannot_hide_an_unchecked_gnu_extension_behind_a_zero_block() {
        let (_temp, path, destination) = fixture(|builder| {
            builder
                .append_pax_extensions([("size", b"512".as_slice())])
                .unwrap();
            append(builder, "zero-header-size", b"");
            builder.get_mut().write_all(&[0; 512]).unwrap();
            let mut hidden = tar::Header::new_gnu();
            hidden.set_path("hidden-extension").unwrap();
            hidden.set_entry_type(tar::EntryType::GNULongName);
            hidden.set_size(1024 * 1024 * 1024);
            hidden.set_cksum();
            builder.get_mut().write_all(hidden.as_bytes()).unwrap();
        });
        let error = unpack(&path, &destination, &ModelStorageBudget::default()).unwrap_err();
        assert_eq!(error.to_string(), "model_archive_pax_field_unsupported");
        assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
    }
}
