//! 固定学习域的合作单写者文件租约。不是旧进程退出/FD清空或SQLite连接身份的证明。
//! runtime接入时须额外持有原生origin授权并核SQLite实际handle；HAS_MOVED不能独自排除ABA。
//! Connection必须成功关闭后才能释放本租约。
use serde::{Deserialize, Serialize};
use std::{
    ffi::CString,
    fs::File,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Component, Path, PathBuf},
};

pub const DOMAIN_DIRECTORY: &str = "managed-memory-v1";
pub const DATABASE_NAME: &str = "memory.sqlite";
pub const OLD_DATABASE_NAME: &str = "inputia_memory.db";
pub const FENCE_RECORD: &str = "handoff.json";

#[derive(Debug)]
pub enum LeaseError {
    UnsafePath,
    Busy,
    IdentityChanged,
    ProcessChanged,
    Io(std::io::Error),
}
impl std::fmt::Display for LeaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for LeaseError {}
impl From<std::io::Error> for LeaseError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
type Result<T> = std::result::Result<T, LeaseError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileIdentity {
    pub device: u64,
    pub inode: u64,
}
impl FileIdentity {
    pub fn of(file: &File) -> Result<Self> {
        let value = file.metadata()?;
        Ok(Self {
            device: value.dev(),
            inode: value.ino(),
        })
    }
}
/// 仅文件事实，不是可反序列化的交接授权；任何使用者仍须核原生origin和域UUID/key/epoch。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryFileBinding {
    pub database: FileIdentity,
    pub fence: FileIdentity,
    pub fence_record: FileIdentity,
    pub service_lock: FileIdentity,
}

fn open(path: &Path, uid: u32, writable: bool, directory: bool) -> Result<File> {
    let text = path.to_str().ok_or(LeaseError::UnsafePath)?;
    if !path.is_absolute()
        || text.contains("//")
        || text.split('/').any(|p| p == "." || p == "..")
        || text.chars().any(char::is_control)
    {
        return Err(LeaseError::UnsafePath);
    }
    let parts: Vec<_> = path
        .components()
        .filter_map(|part| match part {
            Component::Normal(value) => Some(value),
            _ => None,
        })
        .collect();
    if parts.is_empty() {
        return Err(LeaseError::UnsafePath);
    }
    let mut parent = File::open("/")?;
    for (index, part) in parts.iter().enumerate() {
        let last = index + 1 == parts.len();
        let name = CString::new(part.as_bytes()).map_err(|_| LeaseError::UnsafePath)?;
        let flags = libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | libc::O_NONBLOCK
            | if last && writable {
                libc::O_RDWR
            } else {
                libc::O_RDONLY
            }
            | if !last || directory {
                libc::O_DIRECTORY
            } else {
                0
            };
        let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let next = unsafe { File::from_raw_fd(fd) };
        let metadata = next.metadata()?;
        if last {
            if metadata.uid() != uid
                || metadata.mode() & 0o077 != 0
                || (!directory && (!metadata.is_file() || metadata.nlink() != 1))
                || (directory && !metadata.is_dir())
            {
                return Err(LeaseError::UnsafePath);
            }
        } else if !metadata.is_dir()
            || (metadata.uid() != uid && metadata.uid() != 0)
            || (metadata.mode() & 0o022 != 0
                && !(metadata.uid() == 0 && metadata.mode() & 0o1000 != 0))
        {
            return Err(LeaseError::UnsafePath);
        }
        parent = next;
    }
    Ok(parent)
}
fn lock(file: &File) -> Result<()> {
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::EWOULDBLOCK) {
        Err(LeaseError::Busy)
    } else {
        Err(error.into())
    }
}
/// 固定目录和service.lock的合作排他；目标数据库尚未存在时也可持有。
/// 不接受任意File，不提供Clone/dup/LOCK_UN；登记身份仅是文件事实，不发行来源授权。
pub struct OwnedMemoryServiceLock {
    root: PathBuf,
    uid: u32,
    pid: u32,
    managed_identity: FileIdentity,
    lock_identity: FileIdentity,
    managed_directory: File,
    service_lock: File,
}
impl OwnedMemoryServiceLock {
    /// 只打开已登记的目录和锁；初次创建及其耐久登记由交接协议负责。
    pub fn acquire(
        root: &Path,
        uid: u32,
        managed_identity: FileIdentity,
        lock_identity: FileIdentity,
    ) -> Result<Self> {
        let managed_directory = open(&root.join(DOMAIN_DIRECTORY), uid, false, true)?;
        if FileIdentity::of(&managed_directory)? != managed_identity {
            return Err(LeaseError::IdentityChanged);
        }
        let service_lock = open(
            &root.join(DOMAIN_DIRECTORY).join("service.lock"),
            uid,
            true,
            false,
        )?;
        if FileIdentity::of(&service_lock)? != lock_identity {
            return Err(LeaseError::IdentityChanged);
        }
        lock(&service_lock)?;
        let value = Self {
            root: root.to_owned(),
            uid,
            pid: std::process::id(),
            managed_identity,
            lock_identity,
            managed_directory,
            service_lock,
        };
        value.assert_current()?;
        Ok(value)
    }
    /// 父目录本身也必须保持同一实例，不能仅把原子文件搬到新目录后冒充同一命名空间。
    pub fn assert_current(&self) -> Result<()> {
        if self.pid != std::process::id() {
            return Err(LeaseError::ProcessChanged);
        }
        for (held, path, identity, directory) in [
            (
                &self.managed_directory,
                self.root.join(DOMAIN_DIRECTORY),
                self.managed_identity,
                true,
            ),
            (
                &self.service_lock,
                self.root.join(DOMAIN_DIRECTORY).join("service.lock"),
                self.lock_identity,
                false,
            ),
        ] {
            let current = open(&path, self.uid, false, directory)?;
            if FileIdentity::of(held)? != identity || FileIdentity::of(&current)? != identity {
                return Err(LeaseError::IdentityChanged);
            }
        }
        Ok(())
    }
    /// 包括恢复后的重复同步：锁内容、锁所在目录，以及该目录的父目录都耐久后才可推进。
    pub fn sync_creation(&self) -> Result<()> {
        self.assert_current()?;
        self.service_lock.sync_all()?;
        self.managed_directory.sync_all()?;
        open(&self.root, self.uid, false, true)?.sync_all()?;
        self.assert_current()
    }
    /// 消费完整锁对象，直接交给数据库租约；没有重新flock、dup或先释放再获取的窗口。
    pub fn bind_database(self, binding: &MemoryFileBinding) -> Result<OwnedMemoryDomainLease> {
        self.assert_current()?;
        if binding.service_lock != self.lock_identity {
            return Err(LeaseError::IdentityChanged);
        }
        let database = open(
            &self.root.join(DOMAIN_DIRECTORY).join(DATABASE_NAME),
            self.uid,
            true,
            false,
        )?;
        if FileIdentity::of(&database)? != binding.database {
            return Err(LeaseError::IdentityChanged);
        }
        // macOS的flock与SQLite的fcntl锁会相互影响；目标FD只绑定实例，排他锁留在service.lock。
        let fence = open(&self.root.join(OLD_DATABASE_NAME), self.uid, false, true)?;
        let fence_record = open(
            &self.root.join(OLD_DATABASE_NAME).join(FENCE_RECORD),
            self.uid,
            false,
            false,
        )?;
        let value = OwnedMemoryDomainLease {
            root: self.root.clone(),
            uid: self.uid,
            pid: self.pid,
            binding: binding.clone(),
            database,
            fence,
            fence_record,
            service: self,
        };
        value.assert_current()?;
        Ok(value)
    }
}
/// 持有固定service.lock的flock及目标inode私有FD；只能close释放，不公开dup/LOCK_UN入口。
/// 只证明合作锁排他；不声称阻挡任意同UID进程open，也不把JSON binding当origin授权。
pub struct OwnedMemoryDomainLease {
    root: PathBuf,
    uid: u32,
    pid: u32,
    binding: MemoryFileBinding,
    database: File,
    fence: File,
    fence_record: File,
    service: OwnedMemoryServiceLock,
}
impl OwnedMemoryDomainLease {
    /// root只能由调用者已经验证的installation/profile resolver提供，不应接受wire任意路径。
    /// 本方法不创建文件；不存在源/目标绝不能推导成新profile或成功迁移。
    pub fn acquire(root: &Path, uid: u32, binding: &MemoryFileBinding) -> Result<Self> {
        let managed = open(&root.join(DOMAIN_DIRECTORY), uid, false, true)?;
        OwnedMemoryServiceLock::acquire(
            root,
            uid,
            FileIdentity::of(&managed)?,
            binding.service_lock,
        )?
        .bind_database(binding)
    }
    /// 逐次确认持有的实例仍绑定固定名字。fork继承的句柄不产生子进程使用权限。
    pub fn assert_current(&self) -> Result<()> {
        if self.pid != std::process::id() {
            return Err(LeaseError::ProcessChanged);
        }
        self.service.assert_current()?;
        self.assert_database_current()?;
        for (held, path, identity, directory) in [
            (
                &self.fence,
                self.root.join(OLD_DATABASE_NAME),
                self.binding.fence,
                true,
            ),
            (
                &self.fence_record,
                self.root.join(OLD_DATABASE_NAME).join(FENCE_RECORD),
                self.binding.fence_record,
                false,
            ),
        ] {
            let current = open(&path, self.uid, false, directory)?;
            if FileIdentity::of(held)? != identity || FileIdentity::of(&current)? != identity {
                return Err(LeaseError::IdentityChanged);
            }
        }
        Ok(())
    }
    fn assert_database_current(&self) -> Result<()> {
        // 不能为身份检查另开关数据库FD：这会释放同进程SQLite已持有的POSIX锁。
        // held FD只fstat；固定名字通过安全父目录FD做fstatat，绝不open主文件。
        let held = self.database.metadata()?;
        if held.uid() != self.uid
            || held.mode() & 0o077 != 0
            || !held.is_file()
            || held.nlink() != 1
        {
            return Err(LeaseError::UnsafePath);
        }
        let parent = open(&self.root.join(DOMAIN_DIRECTORY), self.uid, false, true)?;
        let mut named = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                parent.as_raw_fd(),
                c"memory.sqlite".as_ptr(),
                named.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        // fstatat成功后stat已完整初始化；NOFOLLOW使符号链接本身无法通过regular-file检查。
        let named = unsafe { named.assume_init() };
        if named.st_uid != self.uid
            || named.st_mode & 0o077 != 0
            || named.st_mode & libc::S_IFMT != libc::S_IFREG
            || named.st_nlink != 1
        {
            return Err(LeaseError::UnsafePath);
        }
        let held_identity = FileIdentity {
            device: held.dev(),
            inode: held.ino(),
        };
        // Unix各平台dev_t/ino_t宽度不同；统一为现有持久身份字段，不解析SQLite私有布局。
        #[allow(clippy::unnecessary_cast)]
        let named_identity = FileIdentity {
            device: named.st_dev as u64,
            inode: named.st_ino as u64,
        };
        if held_identity != self.binding.database || named_identity != self.binding.database {
            return Err(LeaseError::IdentityChanged);
        }
        Ok(())
    }
    pub fn database_path(&self) -> PathBuf {
        self.root.join(DOMAIN_DIRECTORY).join(DATABASE_NAME)
    }
    /// 对外显式说明本租约不是迁移来源或SQLite连接身份授权。
    pub fn requires_origin_and_connection_validation(&self) -> bool {
        true
    }
    /// 只绑定析构顺序，不验证身份或资源关闭成功；SQLite须另核实际handle并处理close失败。
    pub fn bind_resource<T>(self, resource: T) -> LeaseBoundMemoryResource<T> {
        LeaseBoundMemoryResource {
            resource,
            lease: self,
        }
    }
}

/// resource字段先析构，随后才close租约FD；不提供拆出Connection或提前释放lease的入口。
pub struct LeaseBoundMemoryResource<T> {
    resource: T,
    lease: OwnedMemoryDomainLease,
}
impl<T> LeaseBoundMemoryResource<T> {
    pub fn resource(&self) -> &T {
        &self.resource
    }
    pub fn assert_current(&self) -> Result<()> {
        self.lease.assert_current()
    }
}
