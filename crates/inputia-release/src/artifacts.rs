//! 校验清单指定的落盘归档。证明范围为所读字节；原生安装必须在实际暂存副本再次校验。
use crate::{feed::AuthorizedReleaseMetadata, safe_relative, ReleaseError, Result};
use serde::Serialize;
use std::{
    collections::BTreeSet,
    ffi::CString,
    fs::File,
    io::Read,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Component, Path},
};

#[derive(Clone, Debug, Serialize)]
pub struct ArtifactDigest {
    pub role: String,
    pub relative_path: String,
    pub sha256: String,
    pub size: u64,
}
#[derive(Debug)]
pub struct VerifiedArtifactDigests {
    entries: Vec<ArtifactDigest>,
}
impl VerifiedArtifactDigests {
    pub fn entries(&self) -> &[ArtifactDigest] {
        &self.entries
    }
}

impl AuthorizedReleaseMetadata {
    /// 不跟随路径链接，不接受重复 inode，大小/总量受调用方固定预算约束。
    /// 该结果不是 Apple 代码签名、公证、解包安全或实际安装状态证明。
    pub fn verify_artifact_files(
        &self,
        directory: &Path,
        max_total_bytes: u64,
    ) -> Result<VerifiedArtifactDigests> {
        verify_manifest_files(self.manifest(), directory, max_total_bytes)
    }
}
fn verify_manifest_files(
    manifest: &serde_json::Value,
    directory: &Path,
    max_total_bytes: u64,
) -> Result<VerifiedArtifactDigests> {
    crate::manifest::validate_manifest(manifest)?;
    let root = open_directory(directory)?;
    let mut entries = Vec::new();
    let mut identities = BTreeSet::new();
    let mut total = 0u64;
    for item in manifest["components"]
        .as_array()
        .ok_or(ReleaseError::InvalidDocument)?
        .iter()
        .chain(
            manifest["distribution_artifacts"]
                .as_array()
                .ok_or(ReleaseError::InvalidDocument)?,
        )
        .chain(std::iter::once(&manifest["pair_manifest"]))
    {
        let path = item["artifact"]
            .as_str()
            .ok_or(ReleaseError::InvalidDocument)?;
        safe_relative(path)?;
        let mut file = open_relative(&root, path)?;
        let before = file
            .metadata()
            .map_err(|_| ReleaseError::StateUnavailable)?;
        if !before.is_file()
            || before.nlink() != 1
            || before.mode() & 0o022 != 0
            || (before.uid() != 0 && before.uid() != unsafe { libc::geteuid() })
            || !identities.insert((before.dev(), before.ino()))
        {
            return Err(ReleaseError::UnsafePath);
        }
        if item
            .get("size")
            .is_some_and(|expected| expected.as_u64() != Some(before.len()))
        {
            return Err(ReleaseError::DigestMismatch);
        }
        total = total
            .checked_add(before.len())
            .ok_or(ReleaseError::InvalidDocument)?;
        if before.len() == 0 || total > max_total_bytes {
            return Err(ReleaseError::InvalidDocument);
        }
        if item.get("schema").is_some()
            && before.len() > crate::canonical::MAX_DOCUMENT_BYTES as u64
        {
            return Err(ReleaseError::InvalidDocument);
        }
        let mut sha = ring::digest::Context::new(&ring::digest::SHA256);
        let mut buffer = [0u8; 64 * 1024];
        let mut size = 0u64;
        loop {
            let count = file
                .read(&mut buffer)
                .map_err(|_| ReleaseError::StateUnavailable)?;
            if count == 0 {
                break;
            }
            size = size
                .checked_add(count as u64)
                .ok_or(ReleaseError::InvalidDocument)?;
            if size > before.len() {
                return Err(ReleaseError::DigestMismatch);
            }
            sha.update(&buffer[..count]);
        }
        let after = file
            .metadata()
            .map_err(|_| ReleaseError::StateUnavailable)?;
        let sha256: String = sha
            .finish()
            .as_ref()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if size != before.len()
            || sha256 != item["sha256"]
            || before.len() != after.len()
            || before.mtime() != after.mtime()
            || before.mtime_nsec() != after.mtime_nsec()
            || before.ctime() != after.ctime()
            || before.ctime_nsec() != after.ctime_nsec()
            || before.nlink() != after.nlink()
        {
            return Err(ReleaseError::DigestMismatch);
        }
        entries.push(ArtifactDigest {
            role: item["role"].as_str().unwrap_or("pair_manifest").into(),
            relative_path: path.into(),
            sha256,
            size,
        });
    }
    Ok(VerifiedArtifactDigests { entries })
}

fn open_directory(path: &Path) -> Result<File> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
    {
        return Err(ReleaseError::UnsafePath);
    }
    let mut parent = File::open("/").map_err(|_| ReleaseError::StateUnavailable)?;
    for part in path.components().filter_map(|part| {
        if let Component::Normal(p) = part {
            Some(p)
        } else {
            None
        }
    }) {
        parent = open_child(&parent, part.as_bytes(), true)?;
    }
    Ok(parent)
}
fn open_child(parent: &File, name: &[u8], directory: bool) -> Result<File> {
    let name = CString::new(name).map_err(|_| ReleaseError::UnsafePath)?;
    // SAFETY: 单段名称与有效目录 fd；拒绝链接，FIFO 使用 NONBLOCK 后按类型拒绝。
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY
                | libc::O_CLOEXEC
                | libc::O_NOFOLLOW
                | libc::O_NONBLOCK
                | if directory { libc::O_DIRECTORY } else { 0 },
        )
    };
    if fd < 0 {
        return Err(ReleaseError::UnsafePath);
    }
    let file = unsafe { File::from_raw_fd(fd) };
    if directory {
        let meta = file
            .metadata()
            .map_err(|_| ReleaseError::StateUnavailable)?;
        if !meta.is_dir()
            || (meta.uid() != 0 && meta.uid() != unsafe { libc::geteuid() })
            || (meta.mode() & 0o022 != 0 && !(meta.uid() == 0 && meta.mode() & 0o1000 != 0))
        {
            return Err(ReleaseError::UnsafePath);
        }
    }
    Ok(file)
}
fn open_relative(root: &File, path: &str) -> Result<File> {
    let mut parent = root
        .try_clone()
        .map_err(|_| ReleaseError::StateUnavailable)?;
    let parts: Vec<_> = path.split('/').collect();
    for (index, part) in parts.iter().enumerate() {
        parent = open_child(&parent, part.as_bytes(), index + 1 < parts.len())?;
    }
    Ok(parent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    fn fixture(root: &Path) -> serde_json::Value {
        let mut value =
            crate::canonical::parse(include_bytes!("../tests/fixtures/manifest.json")).unwrap();
        for key in ["components", "distribution_artifacts"] {
            for entry in value[key].as_array_mut().unwrap() {
                let path = root.join(entry["artifact"].as_str().unwrap());
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, b"test").unwrap();
                entry["sha256"] = serde_json::json!(crate::digest(b"test"));
            }
        }
        std::fs::write(root.join("pair-manifest.json"), b"pair").unwrap();
        value["pair_manifest"]["sha256"] = serde_json::json!(crate::digest(b"pair"));
        value
    }
    #[test]
    fn complete_artifacts_are_bound_and_budget_or_modified_bytes_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let value = fixture(&root);
        assert_eq!(
            verify_manifest_files(&value, &root, 1024)
                .unwrap()
                .entries
                .len(),
            7
        );
        assert!(verify_manifest_files(&value, &root, 27).is_err());
        std::fs::write(root.join("components/ime.zip"), b"evil").unwrap();
        assert!(verify_manifest_files(&value, &root, 1024).is_err());
    }
    #[test]
    fn file_and_ancestor_links_and_duplicate_inodes_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let value = fixture(&root);
        let control = root.join("components/control.zip");
        std::fs::remove_file(&control).unwrap();
        symlink(root.join("components/ime.zip"), &control).unwrap();
        assert!(verify_manifest_files(&value, &root, 1024).is_err());
        std::fs::remove_file(&control).unwrap();
        std::fs::hard_link(root.join("components/ime.zip"), &control).unwrap();
        assert!(verify_manifest_files(&value, &root, 1024).is_err());
        std::fs::remove_file(&control).unwrap();
        std::fs::write(&control, b"test").unwrap();
        std::fs::rename(root.join("components"), root.join("actual")).unwrap();
        symlink(root.join("actual"), root.join("components")).unwrap();
        assert!(verify_manifest_files(&value, &root, 1024).is_err());
    }
}
