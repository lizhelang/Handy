//! 两端共用的安装收据与数据定位合同。渠道不参与目录或配对身份计算。
//! 本模块不创建、移动或迁移任何用户数据；写入收据只由安装事务负责。

use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

pub const PRODUCT_ID: &str = "com.inputia";
pub const MAX_RECEIPT_BYTES: usize = 16_384;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallationScope {
    User,
    LegacySingleUser,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DataLocation {
    Managed,
    LegacyCandidate { run_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentPaths {
    pub control: PathBuf,
    pub ime: PathBuf,
    pub settings: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateChannel {
    Candidate,
    Stable,
}

/// release_id 仅在完整安装事务提交时切换；installation/profile ID 跨升级保持不变。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallationReceipt {
    pub schema_version: u32,
    pub product_id: String,
    pub installation_id: String,
    pub profile_id: String,
    pub uid: u32,
    pub scope: InstallationScope,
    pub data: DataLocation,
    pub components: ComponentPaths,
    pub release_id: String,
    pub channel: UpdateChannel,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallationBinding {
    pub product_id: String,
    pub installation_id: String,
    pub pair_release_id: String,
}

/// 纯解析上下文来自已验签构建常量和系统用户身份，不能来自收据自身。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocatorContext {
    pub product_id: String,
    pub release_id: String,
    pub uid: u32,
    pub home: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocatedInstallation {
    pub receipt: InstallationReceipt,
    pub handy_root: PathBuf,
    pub inputia_root: PathBuf,
    pub pair_manifest: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallationError {
    InvalidReceipt,
    IdentityMismatch,
    UnsafePath,
    MissingReceipt,
    Unavailable,
}

impl std::fmt::Display for InstallationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidReceipt => "安装收据格式无效",
            Self::IdentityMismatch => "安装收据与当前构建或用户不匹配",
            Self::UnsafePath => "安装收据包含不安全路径或文件属性",
            Self::MissingReceipt => "安装收据缺失，需要修复安装",
            Self::Unavailable => "无法读取安装收据",
        })
    }
}
impl std::error::Error for InstallationError {}

pub fn valid_release_id(value: &str) -> bool {
    value.strip_prefix("inputia-").is_some_and(|suffix| {
        (1..=180).contains(&suffix.len())
            && suffix
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_' || c == b'.')
    })
}

pub fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) {
                c == b'-'
            } else {
                c.is_ascii_digit() || (b'a'..=b'f').contains(&c)
            }
        })
        && value != "00000000-0000-0000-0000-000000000000"
}

fn canonical_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path.to_str().is_some_and(|text| {
            !text.chars().any(char::is_control)
                && !text.contains("//")
                && !text.ends_with('/')
                && !text.split('/').any(|part| part == "." || part == "..")
        })
        && path
            .components()
            .all(|c| matches!(c, Component::RootDir | Component::Normal(_)))
}

impl LocatorContext {
    pub fn receipt_path(&self) -> PathBuf {
        self.home
            .join("Library/Application Support/Inputia/installation.json")
    }

    fn validate(&self) -> Result<(), InstallationError> {
        if self.product_id != PRODUCT_ID || !valid_release_id(&self.release_id) {
            return Err(InstallationError::IdentityMismatch);
        }
        if !canonical_absolute(&self.home) || self.home == Path::new("/") {
            return Err(InstallationError::UnsafePath);
        }
        Ok(())
    }
}

impl InstallationReceipt {
    pub fn parse(bytes: &[u8]) -> Result<Self, InstallationError> {
        if bytes.is_empty() || bytes.len() > MAX_RECEIPT_BYTES {
            return Err(InstallationError::InvalidReceipt);
        }
        serde_json::from_slice(bytes).map_err(|_| InstallationError::InvalidReceipt)
    }

    pub fn resolve(
        self,
        context: &LocatorContext,
    ) -> Result<LocatedInstallation, InstallationError> {
        context.validate()?;
        if self.schema_version != 1 || !valid_uuid(&self.installation_id) {
            return Err(InstallationError::InvalidReceipt);
        }
        if self.product_id != context.product_id
            || self.release_id != context.release_id
            || self.uid != context.uid
        {
            return Err(InstallationError::IdentityMismatch);
        }
        let home = &context.home;
        let (control, settings) = match self.scope {
            InstallationScope::User => (
                home.join("Applications/Inputia.app"),
                home.join("Applications/Inputia 设置.app"),
            ),
            // 旧版安装的三个公开组件都位于系统 Applications / Input Methods 路径；
            // 数据仍然按当前用户的 legacy profile 定位，不把程序路径当作数据迁移。
            InstallationScope::LegacySingleUser => (
                PathBuf::from("/Applications/Inputia.app"),
                PathBuf::from("/Applications/Inputia 设置.app"),
            ),
        };
        let expected = ComponentPaths {
            control,
            ime: home.join("Library/Input Methods/InputiaUnifiedCandidate.app"),
            settings,
        };
        for path in [
            &self.components.control,
            &self.components.ime,
            &self.components.settings,
        ] {
            if !canonical_absolute(path) {
                return Err(InstallationError::UnsafePath);
            }
        }
        if self.components != expected {
            return Err(InstallationError::UnsafePath);
        }
        let support = home.join("Library/Application Support");
        let data_root = match &self.data {
            DataLocation::Managed => {
                if !valid_uuid(&self.profile_id) {
                    return Err(InstallationError::InvalidReceipt);
                }
                support.join("Inputia/Profiles").join(&self.profile_id)
            }
            DataLocation::LegacyCandidate { run_id } => {
                if !(1..=64).contains(&run_id.len())
                    || !run_id
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
                    || self.profile_id != format!("unified-candidate:{run_id}")
                {
                    return Err(InstallationError::InvalidReceipt);
                }
                support.join("HandyUnifiedCandidate").join(run_id)
            }
        };
        let pair_manifest = support
            .join("Inputia/Releases")
            .join(&self.release_id)
            .join("pair-manifest.json");
        Ok(LocatedInstallation {
            receipt: self,
            handy_root: data_root.join("Handy"),
            inputia_root: data_root.join("Inputia"),
            pair_manifest,
        })
    }
}

impl LocatedInstallation {
    pub fn binding(&self) -> InstallationBinding {
        InstallationBinding {
            product_id: self.receipt.product_id.clone(),
            installation_id: self.receipt.installation_id.clone(),
            pair_release_id: self.receipt.release_id.clone(),
        }
    }
}

/// 固定路径逐段 openat/O_NOFOLLOW；打开后核验归属、权限和硬链接，不跟随被换走的祖先。
#[cfg(unix)]
pub fn read_private_file(
    path: &Path,
    uid: u32,
    limit: usize,
) -> Result<Vec<u8>, InstallationError> {
    read_owned_file(path, uid, limit, true)
}

/// 旧公开配对清单可保留 0644；安装收据和发现端点必须使用 private_file=true。
#[cfg(unix)]
pub fn read_owned_file(
    path: &Path,
    uid: u32,
    limit: usize,
    private_file: bool,
) -> Result<Vec<u8>, InstallationError> {
    read_owned_file_with_mode(path, uid, limit, private_file, None)
}

#[cfg(unix)]
pub(crate) fn read_owned_file_with_mode(
    path: &Path,
    uid: u32,
    limit: usize,
    private_file: bool,
    exact_mode: Option<u32>,
) -> Result<Vec<u8>, InstallationError> {
    use std::{
        ffi::CString,
        fs::File,
        io::Read,
        os::{
            fd::{AsRawFd, FromRawFd},
            unix::fs::MetadataExt,
        },
    };
    if !canonical_absolute(path) {
        return Err(InstallationError::UnsafePath);
    }
    let mut parent = File::open("/").map_err(|_| InstallationError::Unavailable)?;
    let parts: Vec<_> = path
        .components()
        .filter_map(|part| match part {
            Component::Normal(v) => Some(v),
            _ => None,
        })
        .collect();
    for (index, part) in parts.iter().enumerate() {
        use std::os::unix::ffi::OsStrExt;
        let name = CString::new(part.as_bytes()).map_err(|_| InstallationError::UnsafePath)?;
        let final_part = index + 1 == parts.len();
        let flags = libc::O_RDONLY
            | libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | if final_part {
                libc::O_NONBLOCK
            } else {
                libc::O_DIRECTORY
            };
        // SAFETY: parent 是本函数拥有的目录 fd，name 为有效 NUL 字符串；新 fd 立即交给 File。
        let fd = unsafe { libc::openat(parent.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(match std::io::Error::last_os_error().raw_os_error() {
                Some(libc::ENOENT) => InstallationError::MissingReceipt,
                Some(libc::ELOOP | libc::ENOTDIR) => InstallationError::UnsafePath,
                _ => InstallationError::Unavailable,
            });
        }
        let file = unsafe { File::from_raw_fd(fd) };
        let meta = file
            .metadata()
            .map_err(|_| InstallationError::Unavailable)?;
        if final_part {
            if !meta.is_file()
                || meta.uid() != uid
                || meta.nlink() != 1
                || meta.mode() & if private_file { 0o077 } else { 0o022 } != 0
                || exact_mode.is_some_and(|mode| meta.mode() & 0o7777 != mode)
                || meta.len() > limit as u64
            {
                return Err(InstallationError::UnsafePath);
            }
            let mut bytes = Vec::new();
            file.take(limit as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| InstallationError::Unavailable)?;
            if bytes.len() > limit {
                return Err(InstallationError::UnsafePath);
            }
            return Ok(bytes);
        }
        if !meta.is_dir() {
            return Err(InstallationError::UnsafePath);
        }
        if (meta.uid() != 0 && meta.uid() != uid)
            || (meta.mode() & 0o022 != 0 && !(meta.uid() == 0 && meta.mode() & 0o1000 != 0))
        {
            return Err(InstallationError::UnsafePath);
        }
        parent = file;
    }
    Err(InstallationError::UnsafePath)
}

#[cfg(unix)]
pub fn load(context: &LocatorContext) -> Result<LocatedInstallation, InstallationError> {
    context.validate()?;
    let bytes = read_private_file(&context.receipt_path(), context.uid, MAX_RECEIPT_BYTES)?;
    InstallationReceipt::parse(&bytes)?.resolve(context)
}
