//! 合作单写者命名空间中的SQLite连接生命周期。
//!
//! 所有受控publish/restore/repair必须先持同一稳定service锁，且不得替换其祖先目录。
//! 本层只核该合作边界下的路径一致性，不把HAS_MOVED称为抗恶意同UID ABA的实际FD证明。
//! 来源发行者、安装/profile授权与旧路径fence仍是调用方的独立前置门禁；本模块不发行它们。

use inputia_settings::memory_domain::{LeaseError, OwnedMemoryDomainLease};
use rusqlite::{Connection, OpenFlags};

#[derive(Debug)]
pub(crate) enum VerificationError {
    Closed,
    Lease(LeaseError),
    Sqlite(rusqlite::Error),
    ReadOnly,
    Moved,
    FileControlUnavailable(i32),
}
impl std::fmt::Display for VerificationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Closed => f.write_str("学习库连接已关闭"),
            Self::Lease(e) => write!(f, "学习库文件租约失效：{e}"),
            Self::Sqlite(e) => write!(f, "学习库连接检查失败：{e}"),
            Self::ReadOnly => f.write_str("学习库连接意外只读"),
            Self::Moved => f.write_str("SQLite观察到学习库路径被替换"),
            Self::FileControlUnavailable(code) => write!(f, "SQLite路径检查不可用：{code}"),
        }
    }
}
impl std::error::Error for VerificationError {}

#[derive(Debug)]
pub(crate) enum OpenError {
    Verification(VerificationError),
    Sqlite(rusqlite::Error),
    CleanupPending {
        verification: VerificationError,
        close_error: Box<rusqlite::Error>,
    },
}
impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Verification(e) => write!(f, "{e}"),
            Self::Sqlite(e) => write!(f, "无法打开现有学习库：{e}"),
            Self::CleanupPending {
                verification,
                close_error,
            } => {
                write!(f, "{verification}；连接尚未关闭，租约保留：{close_error}")
            }
        }
    }
}
impl std::error::Error for OpenError {}

#[derive(Debug)]
pub(crate) enum AccessError<E> {
    BeforeOperation(VerificationError),
    Operation(E),
    /// 闭包可能已经提交；调用方必须按原operation查询，不能当成“未执行”重新派发。
    Uncertain {
        verification: VerificationError,
        operation_error: Option<E>,
    },
}
impl<E: std::fmt::Display> std::fmt::Display for AccessError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BeforeOperation(e) => write!(f, "操作前核验失败：{e}"),
            Self::Operation(e) => write!(f, "学习库操作失败：{e}"),
            Self::Uncertain {
                verification,
                operation_error,
            } => {
                write!(f, "操作后的文件绑定不确定：{verification}")?;
                if let Some(error) = operation_error {
                    write!(f, "；原操作错误：{error}")?;
                }
                Ok(())
            }
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for AccessError<E> {}

/// 不可Clone，不提供拥有所有权的Connection或可替换它的可变引用。
/// 只在sqlite3_close真正成功后释放租约；BUSY保留连接和锁供显式重试。
pub(crate) struct OwnedSqliteConnection {
    connection: Option<Connection>,
    lease: Option<OwnedMemoryDomainLease>,
}

impl OwnedSqliteConnection {
    /// 不创建库，不运行PRAGMA/schema迁移；先完成合作身份检查，才允许调用方执行SQL。
    pub(crate) fn open(lease: OwnedMemoryDomainLease) -> Result<Self, OpenError> {
        lease
            .assert_current()
            .map_err(|e| OpenError::Verification(VerificationError::Lease(e)))?;
        let connection = Connection::open_with_flags(
            lease.database_path(),
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(OpenError::Sqlite)?;
        let mut value = Self {
            connection: Some(connection),
            lease: Some(lease),
        };
        if let Err(verification) = value.verify() {
            return match value.try_close() {
                Ok(()) => Err(OpenError::Verification(verification)),
                // value的Drop还会兜底；仍未关闭时不得把文件租约提前释放。
                Err(close_error) => Err(OpenError::CleanupPending {
                    verification,
                    close_error: Box::new(close_error),
                }),
            };
        }
        Ok(value)
    }

    fn verify(&self) -> Result<(), VerificationError> {
        let connection = self.connection.as_ref().ok_or(VerificationError::Closed)?;
        let lease = self.lease.as_ref().ok_or(VerificationError::Closed)?;
        lease.assert_current().map_err(VerificationError::Lease)?;
        if connection
            .is_readonly(rusqlite::MAIN_DB)
            .map_err(VerificationError::Sqlite)?
        {
            return Err(VerificationError::ReadOnly);
        }
        let mut moved: libc::c_int = 0;
        // 此borrow期间本包装保持连接与租约存活，file_control不取走SQLite内部FD。
        let result = unsafe {
            rusqlite::ffi::sqlite3_file_control(
                connection.handle(),
                c"main".as_ptr(),
                rusqlite::ffi::SQLITE_FCNTL_HAS_MOVED,
                (&mut moved as *mut libc::c_int).cast(),
            )
        };
        if result != rusqlite::ffi::SQLITE_OK {
            return Err(VerificationError::FileControlUnavailable(result));
        }
        if moved != 0 {
            return Err(VerificationError::Moved);
        }
        lease.assert_current().map_err(VerificationError::Lease)
    }

    /// 单worker同步借用；不能返回借用中的Statement/Transaction或取走Connection。
    /// 事务可用Transaction::new_unchecked(&Connection, behavior)，运行期拒绝嵌套事务。
    pub(crate) fn with_connection<T, E>(
        &self,
        operation: impl FnOnce(&Connection) -> Result<T, E>,
    ) -> Result<T, AccessError<E>> {
        self.verify().map_err(AccessError::BeforeOperation)?;
        let connection = self
            .connection
            .as_ref()
            .ok_or(AccessError::BeforeOperation(VerificationError::Closed))?;
        let result = operation(connection);
        if let Err(verification) = self.verify() {
            return Err(AccessError::Uncertain {
                verification,
                operation_error: result.err(),
            });
        }
        result.map_err(AccessError::Operation)
    }

    /// 重复关闭幂等。失败时仍持原连接和锁；禁止把close_v2的延迟zombie释放当成功关闭。
    pub(crate) fn try_close(&mut self) -> Result<(), rusqlite::Error> {
        if let Some(connection) = self.connection.take() {
            if let Err((connection, error)) = connection.close() {
                self.connection = Some(connection);
                return Err(error);
            }
        }
        self.lease.take();
        Ok(())
    }
}

impl Drop for OwnedSqliteConnection {
    fn drop(&mut self) {
        if self.try_close().is_err() {
            // rusqlite的Drop忽略sqlite3_close(BUSY)，故不能依赖字段析构次序。
            // 遗留Statement/Blob/Backup时保守保留两者到进程退出；同域再次获取会Busy。
            // 正常服务关闭应显式try_close并报告错误，不能把本兜底称作关闭成功。
            if let Some(connection) = self.connection.take() {
                std::mem::forget(connection);
            }
            if let Some(lease) = self.lease.take() {
                std::mem::forget(lease);
            }
        }
    }
}

#[cfg(test)]
mod tests;
