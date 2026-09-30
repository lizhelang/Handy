//! 更新维护的共享只读门禁。正常启动不依赖收据或配对版本，旧 v1 同样受限。

use crate::installation::{self, InstallationError, LocatedInstallation, LocatorContext};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

pub const MAX_MARKER_BYTES: usize = 4096;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaintenanceMarker {
    pub schema_version: u32,
    pub transaction_id: String,
    pub installation_id: String,
    #[serde(deserialize_with = "required_optional_release")]
    pub old_release_id: Option<String>,
    pub new_release_id: String,
    pub epoch: String,
    pub plan_sha256: String,
}

fn required_optional_release<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error> {
    Option::<String>::deserialize(deserializer)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserContext {
    pub home: PathBuf,
    pub uid: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceError {
    MaintenanceActive,
    InvalidMarker,
    UnsafePath,
    Unavailable,
    IdentityMismatch,
    MissingMarker,
}

impl std::fmt::Display for MaintenanceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::MaintenanceActive => "Inputia 正在维护，普通启动已暂停",
            Self::InvalidMarker => "Inputia 维护记录无效，需要修复后启动",
            Self::UnsafePath => "Inputia 维护路径或文件属性不安全",
            Self::Unavailable => "无法确认 Inputia 维护状态",
            Self::IdentityMismatch => "维护检查与本次事务或安装身份不匹配",
            Self::MissingMarker => "受限检查缺少有效维护记录",
        })
    }
}
impl std::error::Error for MaintenanceError {}

pub type Result<T> = std::result::Result<T, MaintenanceError>;

fn canonical_home(home: &Path) -> bool {
    home.is_absolute()
        && home != Path::new("/")
        && home.to_str().is_some_and(|text| {
            !text.contains("//")
                && !text.ends_with('/')
                && !text.chars().any(char::is_control)
                && !text.split('/').any(|part| part == "." || part == "..")
        })
        && home
            .components()
            .all(|part| matches!(part, Component::RootDir | Component::Normal(_)))
}

pub fn marker_path(home: &Path) -> PathBuf {
    home.join("Library/Application Support/Inputia/Updater/maintenance.json")
}

impl MaintenanceMarker {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.is_empty() || bytes.len() > MAX_MARKER_BYTES {
            return Err(MaintenanceError::InvalidMarker);
        }
        let marker: Self =
            serde_json::from_slice(bytes).map_err(|_| MaintenanceError::InvalidMarker)?;
        if marker.schema_version != 1
            || !installation::valid_uuid(&marker.transaction_id)
            || !installation::valid_uuid(&marker.installation_id)
            || !installation::valid_uuid(&marker.epoch)
            || !installation::valid_release_id(&marker.new_release_id)
            || marker.old_release_id.as_ref().is_some_and(|old| {
                !installation::valid_release_id(old) || old == &marker.new_release_id
            })
            || marker.plan_sha256.len() != 64
            || !marker
                .plan_sha256
                .bytes()
                .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
        {
            return Err(MaintenanceError::InvalidMarker);
        }
        Ok(marker)
    }
}

/// 只读取固定 marker；任何格式、归属、权限和链接异常均返回错误，绝不当作不存在。
#[cfg(unix)]
pub fn inspect(home: &Path, uid: u32) -> Result<Option<MaintenanceMarker>> {
    if !canonical_home(home) || uid != unsafe { libc::geteuid() } {
        return Err(MaintenanceError::UnsafePath);
    }
    match installation::read_owned_file_with_mode(
        &marker_path(home),
        uid,
        MAX_MARKER_BYTES,
        true,
        Some(0o600),
    ) {
        Ok(bytes) => MaintenanceMarker::parse(&bytes).map(Some),
        Err(InstallationError::MissingReceipt) => Ok(None),
        Err(InstallationError::UnsafePath) => Err(MaintenanceError::UnsafePath),
        Err(_) => Err(MaintenanceError::Unavailable),
    }
}

#[cfg(not(unix))]
pub fn inspect(_: &Path, _: u32) -> Result<Option<MaintenanceMarker>> {
    Err(MaintenanceError::Unavailable)
}

pub fn ensure_normal_start(home: &Path, uid: u32) -> Result<()> {
    match inspect(home, uid)? {
        None => Ok(()),
        Some(_) => Err(MaintenanceError::MaintenanceActive),
    }
}

pub fn ensure_current_normal_start() -> Result<()> {
    let context = current_user_context()?;
    ensure_normal_start(&context.home, context.uid)
}

/// 使用内核 euid 和系统账户目录；不接受 HOME 或 profile 环境变量重定向门禁。
#[cfg(unix)]
pub fn current_user_context() -> Result<UserContext> {
    use std::{ffi::CStr, os::unix::ffi::OsStrExt};
    let uid = unsafe { libc::geteuid() };
    let mut capacity = 16_384;
    loop {
        let mut buffer = vec![0u8; capacity];
        let mut user = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut result = std::ptr::null_mut();
        // SAFETY: 所有输出内存均有效；pw_dir 在 buffer 存活期间复制为独立 PathBuf。
        let code = unsafe {
            libc::getpwuid_r(
                uid,
                user.as_mut_ptr(),
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &mut result,
            )
        };
        if code == libc::ERANGE && capacity < 1_048_576 {
            capacity *= 2;
            continue;
        }
        if code != 0 || result.is_null() {
            return Err(MaintenanceError::Unavailable);
        }
        let user = unsafe { user.assume_init() };
        if user.pw_dir.is_null() || user.pw_uid != uid {
            return Err(MaintenanceError::Unavailable);
        }
        let home = PathBuf::from(std::ffi::OsStr::from_bytes(
            unsafe { CStr::from_ptr(user.pw_dir) }.to_bytes(),
        ));
        if !canonical_home(&home) {
            return Err(MaintenanceError::UnsafePath);
        }
        return Ok(UserContext { home, uid });
    }
}

#[cfg(not(unix))]
pub fn current_user_context() -> Result<UserContext> {
    Err(MaintenanceError::Unavailable)
}

/// 此匹配只允许读取本次安装的元数据；不是写入许可、代码验签或 IMK 握手证明。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadOnlyPostcheckRequest {
    pub transaction_id: String,
    pub installation_id: String,
    pub epoch: String,
    pub plan_sha256: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ReadOnlyPostcheck {
    pub marker: MaintenanceMarker,
    pub installation: LocatedInstallation,
    pub user_databases_opened: bool,
    pub runtime_handshake_verified: bool,
    pub code_signature_verified: bool,
}

/// context 只能来自编译发布常量与系统用户。只读取收据和 marker，不初始化业务组件。
#[cfg(unix)]
pub fn inspect_read_only_postcheck(
    context: &LocatorContext,
    request: &ReadOnlyPostcheckRequest,
) -> Result<ReadOnlyPostcheck> {
    let marker = inspect(&context.home, context.uid)?.ok_or(MaintenanceError::MissingMarker)?;
    if marker.transaction_id != request.transaction_id
        || marker.installation_id != request.installation_id
        || marker.epoch != request.epoch
        || marker.plan_sha256 != request.plan_sha256
        || marker.new_release_id != context.release_id
    {
        return Err(MaintenanceError::IdentityMismatch);
    }
    let installation =
        installation::load(context).map_err(|_| MaintenanceError::IdentityMismatch)?;
    if installation.receipt.installation_id != marker.installation_id
        || inspect(&context.home, context.uid)?.as_ref() != Some(&marker)
    {
        return Err(MaintenanceError::IdentityMismatch);
    }
    Ok(ReadOnlyPostcheck {
        marker,
        installation,
        user_databases_opened: false,
        runtime_handshake_verified: false,
        code_signature_verified: false,
    })
}
