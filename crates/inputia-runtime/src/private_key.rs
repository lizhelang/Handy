//! 使用系统随机源生成本地词身份密钥；已有账本缺密钥时拒绝重建。

use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    path::Path,
};

pub fn load_or_create(path: &Path, allow_create: bool) -> io::Result<[u8; 32]> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::other("invalid key filename"))?;
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("invalid key parent"))?
        .canonicalize()?;
    let path = parent.join(name);
    let mut read = OpenOptions::new();
    read.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        read.custom_flags(libc::O_NOFOLLOW);
    }
    match read.open(&path) {
        Ok(file) => read_key(file),
        Err(error) if error.kind() == io::ErrorKind::NotFound && allow_create => {
            let mut key = [0; 32];
            getrandom::getrandom(&mut key)
                .map_err(|_| io::Error::other("system random source failed"))?;
            let mut create = OpenOptions::new();
            create.read(true).write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                create.mode(0o600).custom_flags(libc::O_NOFOLLOW);
            }
            match create.open(&path) {
                Ok(mut file) => {
                    file.write_all(&key)?;
                    file.sync_all()?;
                    #[cfg(unix)]
                    File::open(parent)?.sync_all()?;
                    Ok(key)
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    read_key(read.open(path)?)
                }
                Err(error) => Err(error),
            }
        }
        Err(error) => Err(error),
    }
}

fn read_key(mut file: File) -> io::Result<[u8; 32]> {
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() != 32 {
        return Err(io::Error::other("invalid learning key file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // SAFETY: geteuid 无参数且无调用前置条件。
        let uid = unsafe { libc::geteuid() };
        if metadata.mode() & 0o077 != 0 || metadata.uid() != uid || metadata.nlink() != 1 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "learning key is not private",
            ));
        }
    }
    let mut key = [0; 32];
    file.read_exact(&mut key)?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn key_survives_reopen_and_missing_ledger_key_is_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key");
        assert!(load_or_create(&path, false).is_err());
        assert!(!path.exists());
        let first = load_or_create(&path, true).unwrap();
        assert_eq!(first, load_or_create(&path, false).unwrap());
        assert_ne!(
            first,
            load_or_create(&dir.path().join("another"), true).unwrap()
        );
    }
    #[cfg(unix)]
    #[test]
    fn public_or_linked_keys_are_rejected() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key");
        load_or_create(&path, true).unwrap();
        let link = dir.path().join("redirect");
        symlink(&path, &link).unwrap();
        assert!(load_or_create(&link, true).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(load_or_create(&path, false).is_err());
    }
}
