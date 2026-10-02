//! 单边进程崩溃下的暂停恢复库。当前没有正式Updater入口/签名正向验收，不授数据交接权。
#[cfg(all(target_os = "macos", feature = "native-code-verification"))]
mod native;
#[cfg(all(target_os = "macos", feature = "native-code-verification"))]
mod protocol;
#[cfg(all(target_os = "macos", feature = "native-code-verification"))]
mod recovery;
#[cfg(all(target_os = "macos", feature = "native-code-verification"))]
mod runtime;
#[cfg(all(target_os = "macos", feature = "native-code-verification"))]
mod session;
#[cfg(all(target_os = "macos", feature = "native-code-verification"))]
mod transport;

const INTERNAL_MODE: &str = "--inputia-internal-suspension-guardian";

use crate::{MaintenanceMarker, Subject};
use std::{
    fs::File,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

#[derive(Debug)]
pub enum GuardianError {
    NativeUnavailable,
    NativeReply,
    InvalidAuthority,
    Cancelled,
    PeerStillAlive,
    RecoveryPending,
    Native { code: String, status: i32 },
    Protocol(String),
    Io(std::io::Error),
    Quiescence(crate::native_quiescence::NativeQuiescenceError),
}
impl std::fmt::Display for GuardianError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for GuardianError {}
impl From<std::io::Error> for GuardianError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<crate::native_quiescence::NativeQuiescenceError> for GuardianError {
    fn from(e: crate::native_quiescence::NativeQuiescenceError) -> Self {
        Self::Quiescence(e)
    }
}
#[cfg(all(target_os = "macos", feature = "native-code-verification"))]
impl From<protocol::ProtocolError> for GuardianError {
    fn from(e: protocol::ProtocolError) -> Self {
        Self::Protocol(e.to_string())
    }
}
#[cfg(all(target_os = "macos", feature = "native-code-verification"))]
impl From<transport::TransportError> for GuardianError {
    fn from(e: transport::TransportError) -> Self {
        match e {
            transport::TransportError::Io(e) => Self::Io(e),
            transport::TransportError::Protocol(e) => e.into(),
            transport::TransportError::Closed | transport::TransportError::Timeout => {
                Self::RecoveryPending
            }
        }
    }
}

/// 只由真正Transaction的已持flock描述符建立；没有JSON/路径/裸FD公开构造。
pub struct GuardianTransactionAuthority {
    pub(crate) lock: File,
    pub(crate) subject: Subject,
    pub(crate) marker: MaintenanceMarker,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuardianStatus {
    Starting,
    Holding,
    Recovering,
    RecoveryRequired,
    Resumed,
}
/// 后台恢复线程持有锁/清单到真实终态；丢弃UI句柄不会取消其恢复责任。
pub struct GuardedWriterLease {
    id: String,
    state: Arc<Mutex<GuardianStatus>>,
    cancelled: Arc<AtomicBool>,
    #[cfg(all(target_os = "macos", feature = "native-code-verification"))]
    requests: std::sync::mpsc::SyncSender<LeaseRequest>,
}
#[cfg(all(target_os = "macos", feature = "native-code-verification"))]
enum LeaseRequest {
    Inspect(std::sync::mpsc::SyncSender<Result<(), GuardianError>>),
}
impl GuardedWriterLease {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn status(&self) -> Result<GuardianStatus, GuardianError> {
        self.state
            .lock()
            .map(|s| *s)
            .map_err(|_| GuardianError::RecoveryPending)
    }
    /// 同步重核真实guardian/目标状态；不是完整NativeAdapter QuiescenceReceipt。
    pub fn assert_suspended(&self) -> Result<(), GuardianError> {
        #[cfg(not(all(target_os = "macos", feature = "native-code-verification")))]
        {
            Err(GuardianError::NativeUnavailable)
        }
        #[cfg(all(target_os = "macos", feature = "native-code-verification"))]
        {
            if self.cancelled.load(Ordering::Acquire) {
                return Err(GuardianError::Cancelled);
            }
            let (send, receive) = std::sync::mpsc::sync_channel(1);
            self.requests
                .try_send(LeaseRequest::Inspect(send))
                .map_err(|_| GuardianError::RecoveryPending)?;
            receive
                .recv_timeout(Duration::from_secs(2))
                .map_err(|_| GuardianError::RecoveryPending)?
        }
    }
    /// 不会用guardian活着但超时作为接管依据；失败保留后台恢复责任，可重试查询。
    pub fn resume(&mut self) -> Result<(), GuardianError> {
        self.cancelled.store(true, Ordering::Release);
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            if self.status()? == GuardianStatus::Resumed {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Err(GuardianError::RecoveryPending)
    }
}
impl Drop for GuardedWriterLease {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

/// 开始异步受监督暂停；调用方必须等Holding并重验。默认最长30秒，不允许无限续租。
/// 当前没有生产调用；正式签名同入口reexec、TIS/fence/独占FD仍需单独验收。
pub fn begin_guarded_suspension(
    authority: GuardianTransactionAuthority,
    updater: crate::native_code::VerifiedCodeEvidence,
    roles: &[crate::native_code::VerifiedCodeEvidence],
) -> Result<GuardedWriterLease, GuardianError> {
    #[cfg(all(target_os = "macos", feature = "native-code-verification"))]
    {
        runtime::begin(authority, updater, roles)
    }
    #[cfg(not(all(target_os = "macos", feature = "native-code-verification")))]
    {
        let _ = (
            authority.lock,
            authority.subject,
            authority.marker,
            updater,
            roles,
        );
        Err(GuardianError::NativeUnavailable)
    }
}
/// 供未来已验Updater的main在初始化GUI/服务前调用。普通启动返回None；不从PATH启动helper。
/// 单独存在库入口不等于产品已接线。
pub fn guardian_entry() -> Result<Option<i32>, GuardianError> {
    let args: Vec<_> = std::env::args_os().collect();
    let requested = args
        .iter()
        .skip(1)
        .any(|arg| arg == std::ffi::OsStr::new(INTERNAL_MODE));
    if !requested {
        return Ok(None);
    }
    // 内部标记出现在任意其它位置、重复或混入公开参数时都不能回落公开 CLI。
    if args.len() != 3 || args.get(1).is_none_or(|arg| arg != INTERNAL_MODE) {
        return Err(GuardianError::InvalidAuthority);
    }
    #[cfg(all(target_os = "macos", feature = "native-code-verification"))]
    {
        runtime::entry()
    }
    #[cfg(not(all(target_os = "macos", feature = "native-code-verification")))]
    {
        Err(GuardianError::NativeUnavailable)
    }
}

#[cfg(all(test, target_os = "macos", feature = "native-code-verification"))]
mod tests;
