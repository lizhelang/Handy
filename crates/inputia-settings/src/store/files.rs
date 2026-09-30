//! 固定目录句柄和独立锁文件；rename 不会替换锁住的 inode。
use super::{Error, Result};
use std::{
    ffi::CString,
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Component, Path},
};

pub(super) struct Files {
    directory: File,
    _lock: File,
    uid: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Boundary {
    TempWritten,
    FileSynced,
    Renamed,
    DirectorySynced,
    ReplayFileSynced,
    ReplayDirectorySynced,
}
fn io<T>(_: T) -> Error {
    Error::StorageUnavailable
}
fn cstring(value: &[u8]) -> Result<CString> {
    CString::new(value).map_err(|_| Error::UnsafePath)
}
fn fd_file(fd: libc::c_int) -> Result<File> {
    if fd < 0 {
        Err(Error::StorageUnavailable)
    } else {
        // SAFETY: 成功 open 返回本函数唯一拥有的 fd。
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}
fn validate_directory(file: &File, uid: u32, root: bool) -> Result<()> {
    let metadata = file.metadata().map_err(io)?;
    let root_sticky = metadata.uid() == 0 && metadata.mode() & 0o1000 != 0;
    if !metadata.is_dir()
        || (metadata.uid() != uid && metadata.uid() != 0)
        || (metadata.mode() & 0o022 != 0 && !root_sticky)
        || (!root && metadata.uid() != uid)
    {
        return Err(Error::UnsafePath);
    }
    Ok(())
}
impl Files {
    pub(super) fn open(parent: &Path, uid: u32) -> Result<Self> {
        if !parent.is_absolute() {
            return Err(Error::UnsafePath);
        }
        let mut directory = fd_file(unsafe {
            libc::open(
                c"/".as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        })?;
        validate_directory(&directory, uid, true)?;
        for component in parent.components() {
            let name = match component {
                Component::RootDir => continue,
                Component::Normal(name) => cstring(name.as_bytes())?,
                _ => return Err(Error::UnsafePath),
            };
            let mut fd = unsafe {
                libc::openat(
                    directory.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                )
            };
            if fd < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
                if unsafe { libc::mkdirat(directory.as_raw_fd(), name.as_ptr(), 0o700) } != 0
                    && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists
                {
                    return Err(Error::StorageUnavailable);
                }
                directory.sync_all().map_err(io)?;
                fd = unsafe {
                    libc::openat(
                        directory.as_raw_fd(),
                        name.as_ptr(),
                        libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                    )
                };
            }
            directory = fd_file(fd)?;
            validate_directory(&directory, uid, true)?;
        }
        validate_directory(&directory, uid, false)?;
        let lock = fd_file(unsafe {
            libc::openat(
                directory.as_raw_fd(),
                c".inputia-settings.lock".as_ptr(),
                libc::O_RDWR
                    | libc::O_CREAT
                    | libc::O_CLOEXEC
                    | libc::O_NOFOLLOW
                    | libc::O_NONBLOCK,
                0o600,
            )
        })?;
        let metadata = lock.metadata().map_err(io)?;
        if !metadata.is_file()
            || metadata.uid() != uid
            || metadata.nlink() != 1
            || metadata.mode() & 0o7777 != 0o600
        {
            return Err(Error::UnsafePath);
        }
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(Error::Busy);
        }
        Ok(Self {
            directory,
            _lock: lock,
            uid,
        })
    }
    pub(super) fn read(&self, name: &str, limit: usize, private: bool) -> Result<Option<Vec<u8>>> {
        let name = cstring(name.as_bytes())?;
        let fd = unsafe {
            libc::openat(
                self.directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
            )
        };
        if fd < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
            return Ok(None);
        }
        let mut file = fd_file(fd)?;
        let before = file.metadata().map_err(io)?;
        if !before.is_file()
            || before.uid() != self.uid
            || before.nlink() != 1
            || (if private {
                before.mode() & 0o7777 != 0o600
            } else {
                before.mode() & 0o022 != 0
            })
            || before.len() > limit as u64
        {
            return Err(Error::UnsafePath);
        }
        let mut raw = vec![];
        (&mut file)
            .take(limit as u64 + 1)
            .read_to_end(&mut raw)
            .map_err(io)?;
        let after = file.metadata().map_err(io)?;
        if raw.len() > limit
            || before.len() != after.len()
            || before.mtime() != after.mtime()
            || before.mtime_nsec() != after.mtime_nsec()
            || before.ctime() != after.ctime()
            || before.ctime_nsec() != after.ctime_nsec()
            || after.nlink() != 1
        {
            return Err(Error::ExternalEdit);
        }
        Ok(Some(raw))
    }
    pub(super) fn confirm_durable(
        &self,
        hook: &mut impl FnMut(Boundary) -> Result<()>,
    ) -> Result<()> {
        let confirm = || -> Result<()> {
            for name in [c"settings.json", c".inputia-settings-initialized.json"] {
                let file = fd_file(unsafe {
                    libc::openat(
                        self.directory.as_raw_fd(),
                        name.as_ptr(),
                        libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
                    )
                })?;
                let metadata = file.metadata().map_err(io)?;
                if !metadata.is_file()
                    || metadata.uid() != self.uid
                    || metadata.nlink() != 1
                    || metadata.mode() & 0o7777 != 0o600
                {
                    return Err(Error::UnsafePath);
                }
                file.sync_all().map_err(io)?;
            }
            Ok(())
        };
        confirm().map_err(|_| Error::CommitUncertain)?;
        hook(Boundary::ReplayFileSynced).map_err(|_| Error::CommitUncertain)?;
        self.directory
            .sync_all()
            .map_err(|_| Error::CommitUncertain)?;
        hook(Boundary::ReplayDirectorySynced).map_err(|_| Error::CommitUncertain)?;
        Ok(())
    }
    pub(super) fn replace(
        &self,
        name: &str,
        bytes: &[u8],
        hook: &mut impl FnMut(Boundary) -> Result<()>,
    ) -> Result<()> {
        self.replace_inner(name, bytes, hook, true)
    }
    /// 生效观察带短租约，不是耐久业务回执；崩溃后过期，避免每秒为观察信息 fsync。
    pub(super) fn replace_observation(&self, name: &str, bytes: &[u8]) -> Result<()> {
        self.replace_inner(name, bytes, &mut |_| Ok(()), false)
    }
    fn replace_inner(
        &self,
        name: &str,
        bytes: &[u8],
        hook: &mut impl FnMut(Boundary) -> Result<()>,
        durable: bool,
    ) -> Result<()> {
        let temp = cstring(format!(".settings-{}.tmp", uuid::Uuid::new_v4()).as_bytes())?;
        let target = cstring(name.as_bytes())?;
        let mut file = fd_file(unsafe {
            libc::openat(
                self.directory.as_raw_fd(),
                temp.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                0o600,
            )
        })?;
        let result = (|| {
            file.write_all(bytes).map_err(io)?;
            hook(Boundary::TempWritten)?;
            if durable {
                file.sync_all().map_err(io)?;
            }
            hook(Boundary::FileSynced)?;
            if unsafe {
                libc::renameat(
                    self.directory.as_raw_fd(),
                    temp.as_ptr(),
                    self.directory.as_raw_fd(),
                    target.as_ptr(),
                )
            } != 0
            {
                return Err(Error::StorageUnavailable);
            }
            hook(Boundary::Renamed).map_err(|_| Error::CommitUncertain)?;
            if durable {
                self.directory
                    .sync_all()
                    .map_err(|_| Error::CommitUncertain)?;
            }
            hook(Boundary::DirectorySynced).map_err(|_| Error::CommitUncertain)?;
            Ok(())
        })();
        // 只删除本次 create_new 的临时名；rename 后目标存在时此名已不存在。
        unsafe { libc::unlinkat(self.directory.as_raw_fd(), temp.as_ptr(), 0) };
        result
    }
}

impl Drop for Files {
    fn drop(&mut self) {
        // fork→exec 窗口可能继承同一 open-file-description；显式解锁不等子进程 exec 关 fd。
        unsafe { libc::flock(self._lock.as_raw_fd(), libc::LOCK_UN) };
    }
}
