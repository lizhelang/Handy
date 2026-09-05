//! 已标记候选安装的数据域；环境变量不能把日常安装切入候选模式。

use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
};

const CANDIDATE_BUNDLE: &str = "com.pais.handy.UnifiedCandidate";
static PROFILE: OnceLock<Result<Option<CandidateProfile>, String>> = OnceLock::new();

#[derive(Clone, Debug, PartialEq, Eq)]
/// 已通过安装标记解析的 Handy/Inputia 配对数据域。
pub struct CandidateProfile {
    pub handy_root: PathBuf,
    pub inputia_root: PathBuf,
    pub profile_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
/// 保留 Info.plist 值类型，避免将数字 1 或字符串 "true" 当作布尔授权。
pub enum MetadataValue {
    Missing,
    Boolean(bool),
    String(String),
    Invalid,
}

/// 必须在 portable 和 Tauri 初始化之前调用；失败必须终止启动。
pub fn initialize(compiled_identifier: &str) -> Result<(), String> {
    PROFILE
        .get_or_init(|| {
            #[cfg(target_os = "macos")]
            {
                let native = objc2_foundation::NSBundle::mainBundle()
                    .bundleIdentifier()
                    .map(|value| value.to_string());
                validate_installation_identity(compiled_identifier, native.as_deref())?;
            }
            #[cfg(not(target_os = "macos"))]
            validate_installation_identity(compiled_identifier, None)?;
            let profile = platform_profile()?;
            if let Some(profile) = &profile {
                profile.audit_and_prepare()?;
            }
            Ok(profile)
        })
        .as_ref()
        .map(|_| ())
        .map_err(Clone::clone)
}

/// 候选的编译身份与包身份必须双向一致；裸候选二进制不能回退日常数据域。
pub fn validate_installation_identity(compiled: &str, native: Option<&str>) -> Result<(), String> {
    if (compiled == CANDIDATE_BUNDLE && native != Some(CANDIDATE_BUNDLE))
        || native.is_some_and(|native| native != compiled)
    {
        Err("编译身份与安装包身份不一致，拒绝初始化数据".into())
    } else {
        Ok(())
    }
}

/// 返回已初始化的候选数据域；调用方必须先处理 initialize 的失败。
pub fn current() -> Option<&'static CandidateProfile> {
    PROFILE
        .get()
        .and_then(|result| result.as_ref().ok())
        .and_then(Option::as_ref)
}

/// macOS 的 data_directory 不控制 WK 存储；按运行域选用专属存储。
pub fn configure_webview<'a, R: tauri::Runtime, M: tauri::Manager<R>>(
    builder: tauri::WebviewWindowBuilder<'a, R, M>,
) -> tauri::WebviewWindowBuilder<'a, R, M> {
    #[cfg(target_os = "macos")]
    if let Some(profile) = current() {
        let major = objc2_foundation::NSProcessInfo::processInfo()
            .operatingSystemVersion()
            .majorVersion;
        return match webview_store(&profile.profile_id, major) {
            CandidateWebviewStore::Persistent(identifier) => {
                builder.data_store_identifier(identifier)
            }
            CandidateWebviewStore::Ephemeral => builder.incognito(true),
        };
    }
    builder
}

#[cfg(any(target_os = "macos", test))]
#[derive(Debug, PartialEq, Eq)]
enum CandidateWebviewStore {
    Persistent([u8; 16]),
    Ephemeral,
}

#[cfg(any(target_os = "macos", test))]
fn webview_store(profile_id: &str, os_major: isize) -> CandidateWebviewStore {
    if os_major < 14 {
        return CandidateWebviewStore::Ephemeral;
    }
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"handy-unified-candidate-webview-v1\0");
    hasher.update(profile_id.as_bytes());
    let digest = hasher.finalize();
    let mut identifier = [0; 16];
    identifier.copy_from_slice(&digest[..16]);
    CandidateWebviewStore::Persistent(identifier)
}

/// 纯元数据解析，不访问或创建目录，供候选打包检查复用。
pub fn resolve(
    bundle_id: Option<&str>,
    marker: &MetadataValue,
    plist_run: &MetadataValue,
    environment_run: Option<&str>,
    application_support: &Path,
) -> Result<Option<CandidateProfile>, String> {
    let candidate = bundle_id == Some(CANDIDATE_BUNDLE);
    let has_metadata = marker != &MetadataValue::Missing || plist_run != &MetadataValue::Missing;
    if !candidate {
        if has_metadata || environment_run.is_some() {
            return Err("未授权的 Handy 候选 profile 标记".into());
        }
        return Ok(None);
    }
    if marker != &MetadataValue::Boolean(true) {
        return Err("候选安装必须包含严格布尔值 HandyDevelopmentCandidate=true".into());
    }
    let MetadataValue::String(run_id) = plist_run else {
        return Err("候选安装缺少有效 HandyProfileRunID".into());
    };
    if !(1..=64).contains(&run_id.len())
        || !run_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err("候选 profile ID 只能包含 1–64 个 ASCII 字母、数字、横线和下划线".into());
    }
    if environment_run.is_some_and(|value| value != run_id) {
        return Err("候选环境变量与安装标记的 profile 不匹配".into());
    }
    if !application_support.is_absolute() {
        return Err("Application Support 路径不是绝对路径".into());
    }
    let root = application_support
        .join("HandyUnifiedCandidate")
        .join(run_id);
    Ok(Some(CandidateProfile {
        handy_root: root.join("Handy"),
        inputia_root: root.join("Inputia"),
        profile_id: format!("unified-candidate:{run_id}"),
    }))
}

impl CandidateProfile {
    fn audit_and_prepare(&self) -> Result<(), String> {
        let run_root = self.handy_root.parent().ok_or("候选根缺少父目录")?;
        audit_ancestors(run_root)?;
        // 在创建或打开任何数据文件前，审计两端现有目录（包括 SQLite sidecar）。
        audit_tree(run_root)?;
        for path in [&self.handy_root, &self.inputia_root] {
            create_private_directories(path)?;
        }
        audit_tree(run_root)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // 只收紧当前候选运行域；不修改 Application Support 等用户祖先。
            std::fs::set_permissions(run_root, std::fs::Permissions::from_mode(0o700))
                .map_err(|error| format!("候选运行域权限设置失败: {error}"))?;
        }
        Ok(())
    }
}

fn audit_ancestors(path: &Path) -> Result<(), String> {
    let ancestors: Vec<_> = path.ancestors().collect();
    for ancestor in ancestors.into_iter().rev() {
        match std::fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => return Err("候选路径包含符号链接或非目录父节点".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("候选路径元数据审计失败: {error}")),
        }
    }
    Ok(())
}

fn audit_tree(path: &Path) -> Result<(), String> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("候选目录元数据读取失败: {error}")),
    };
    if metadata.file_type().is_symlink() || !(metadata.is_dir() || metadata.is_file()) {
        return Err("候选数据包含符号链接或特殊文件".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        unsafe extern "C" {
            fn geteuid() -> u32;
        }
        if metadata.uid() != unsafe { geteuid() } || (metadata.is_file() && metadata.nlink() != 1) {
            return Err("候选数据归属不匹配或包含硬链接".into());
        }
        if metadata.mode() & 0o022 != 0 {
            return Err("候选数据允许其他用户写入".into());
        }
    }
    if metadata.is_dir() {
        for entry in
            std::fs::read_dir(path).map_err(|error| format!("候选目录枚举失败: {error}"))?
        {
            audit_tree(&entry.map_err(|error| error.to_string())?.path())?;
        }
    }
    Ok(())
}

fn create_private_directories(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            create_private_directories(parent)?;
        }
    }
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => audit_ancestors(path),
        Err(error) => Err(format!("候选目录创建失败: {error}")),
    }
}

#[cfg(target_os = "macos")]
fn metadata_value(value: &objc2::runtime::AnyObject) -> MetadataValue {
    use objc2::runtime::AnyObject;
    use objc2_foundation::NSString;
    use std::ffi::c_void;
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFGetTypeID(value: *const c_void) -> usize;
        fn CFBooleanGetTypeID() -> usize;
        fn CFBooleanGetValue(value: *const c_void) -> u8;
    }
    let pointer = (value as *const AnyObject).cast::<c_void>();
    if unsafe { CFGetTypeID(pointer) == CFBooleanGetTypeID() } {
        MetadataValue::Boolean(unsafe { CFBooleanGetValue(pointer) } != 0)
    } else if let Some(value) = value.downcast_ref::<NSString>() {
        MetadataValue::String(value.to_string())
    } else {
        MetadataValue::Invalid
    }
}

#[cfg(target_os = "macos")]
fn platform_profile() -> Result<Option<CandidateProfile>, String> {
    use objc2_foundation::{
        NSBundle, NSFileManager, NSSearchPathDirectory, NSSearchPathDomainMask, NSString,
    };
    let bundle = NSBundle::mainBundle();
    let metadata = |key: &str| {
        let Some(value) = bundle.objectForInfoDictionaryKey(&NSString::from_str(key)) else {
            return MetadataValue::Missing;
        };
        metadata_value(&value)
    };
    let marker = metadata("HandyDevelopmentCandidate");
    let run = metadata("HandyProfileRunID");
    let bundle_id = bundle.bundleIdentifier().map(|value| value.to_string());
    let environment = match std::env::var("HANDY_PROFILE_RUN_ID") {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(_) => return Err("候选环境 profile ID 不是有效 Unicode".into()),
    };
    let directories = NSFileManager::defaultManager().URLsForDirectory_inDomains(
        NSSearchPathDirectory::ApplicationSupportDirectory,
        NSSearchPathDomainMask::UserDomainMask,
    );
    let support = directories
        .firstObject()
        .and_then(|url| url.path())
        .map(|value| PathBuf::from(value.to_string()))
        .ok_or("系统没有返回用户 Application Support 目录")?;
    resolve(
        bundle_id.as_deref(),
        &marker,
        &run,
        environment.as_deref(),
        &support,
    )
}

#[cfg(not(target_os = "macos"))]
fn platform_profile() -> Result<Option<CandidateProfile>, String> {
    if std::env::var_os("HANDY_PROFILE_RUN_ID").is_some() {
        return Err("此平台不支持已签名 macOS 候选 profile".into());
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wk_storage_is_profile_specific_or_explicitly_ephemeral() {
        assert_eq!(webview_store("run-a", 13), CandidateWebviewStore::Ephemeral);
        assert_eq!(webview_store("run-a", 14), webview_store("run-a", 26));
        assert_ne!(webview_store("run-a", 14), webview_store("run-b", 14));
    }

    #[test]
    fn compiled_candidate_cannot_run_bare_or_inside_a_daily_bundle() {
        assert!(validate_installation_identity(CANDIDATE_BUNDLE, None).is_err());
        assert!(validate_installation_identity(CANDIDATE_BUNDLE, Some("com.pais.handy")).is_err());
        assert!(validate_installation_identity("com.pais.handy", Some(CANDIDATE_BUNDLE)).is_err());
        assert!(validate_installation_identity(CANDIDATE_BUNDLE, Some(CANDIDATE_BUNDLE)).is_ok());
        assert!(validate_installation_identity("com.pais.handy", None).is_ok());
    }
    fn candidate(run: &str, support: &Path) -> Result<Option<CandidateProfile>, String> {
        resolve(
            Some(CANDIDATE_BUNDLE),
            &MetadataValue::Boolean(true),
            &MetadataValue::String(run.into()),
            None,
            support,
        )
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn native_metadata_distinguishes_boolean_from_number_and_string() {
        use objc2_foundation::{NSNumber, NSString};
        assert_eq!(
            metadata_value(&NSNumber::numberWithBool(true)),
            MetadataValue::Boolean(true)
        );
        assert_eq!(
            metadata_value(&NSNumber::numberWithBool(false)),
            MetadataValue::Boolean(false)
        );
        assert_eq!(
            metadata_value(&NSNumber::numberWithInt(1)),
            MetadataValue::Invalid
        );
        assert_eq!(
            metadata_value(&NSString::from_str("true")),
            MetadataValue::String("true".into())
        );
    }
    #[test]
    fn daily_cannot_be_promoted_by_environment_or_partial_markers() {
        let base = Path::new("/synthetic/support");
        assert_eq!(
            resolve(
                Some("com.pais.handy"),
                &MetadataValue::Missing,
                &MetadataValue::Missing,
                None,
                base
            )
            .unwrap(),
            None
        );
        for marker in [
            MetadataValue::Missing,
            MetadataValue::Boolean(true),
            MetadataValue::Boolean(false),
            MetadataValue::Invalid,
        ] {
            assert!(resolve(
                Some("com.pais.handy"),
                &marker,
                &MetadataValue::Missing,
                Some("trial"),
                base
            )
            .is_err());
        }
        assert!(resolve(
            Some("other.UnifiedCandidate"),
            &MetadataValue::Boolean(true),
            &MetadataValue::String("trial".into()),
            None,
            base
        )
        .is_err());
    }
    #[test]
    fn strict_boolean_run_id_and_environment_pairing() {
        let base = Path::new("/synthetic/support");
        for marker in [
            MetadataValue::Missing,
            MetadataValue::Boolean(false),
            MetadataValue::String("true".into()),
            MetadataValue::Invalid,
        ] {
            assert!(resolve(
                Some(CANDIDATE_BUNDLE),
                &marker,
                &MetadataValue::String("trial".into()),
                None,
                base
            )
            .is_err());
        }
        for run in [
            "",
            ".",
            "../daily",
            "a/b",
            "a b",
            "中文",
            "a\n",
            &"a".repeat(65),
        ] {
            assert!(candidate(run, base).is_err());
        }
        let profile = candidate("A-1_trial", base).unwrap().unwrap();
        assert_eq!(
            profile.handy_root,
            base.join("HandyUnifiedCandidate/A-1_trial/Handy")
        );
        assert_eq!(
            profile.inputia_root,
            base.join("HandyUnifiedCandidate/A-1_trial/Inputia")
        );
        assert_eq!(profile.profile_id, "unified-candidate:A-1_trial");
        assert!(resolve(
            Some(CANDIDATE_BUNDLE),
            &MetadataValue::Boolean(true),
            &MetadataValue::String("trial".into()),
            Some("other"),
            base
        )
        .is_err());
        assert!(resolve(
            Some(CANDIDATE_BUNDLE),
            &MetadataValue::Boolean(true),
            &MetadataValue::String("trial".into()),
            Some("trial"),
            base
        )
        .is_ok());
    }
    #[test]
    fn prepares_only_synthetic_candidate_directories() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let profile = candidate("trial", &base).unwrap().unwrap();
        profile.audit_and_prepare().unwrap();
        assert!(profile.handy_root.is_dir());
        assert!(profile.inputia_root.is_dir());
        assert!(!base.join("com.pais.handy").exists());
        assert!(!base.join("Inputia").exists());
        std::fs::write(profile.handy_root.join("history.db-wal"), b"synthetic").unwrap();
        profile.audit_and_prepare().unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn rejects_linked_sidecars_and_inputia_subtrees_before_creating_handy() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let profile = candidate("trial", &base).unwrap().unwrap();
        std::fs::create_dir_all(&profile.inputia_root).unwrap();
        let outside = base.join("outside");
        std::fs::write(&outside, b"untouched").unwrap();
        symlink(&outside, profile.inputia_root.join("memory.db-shm")).unwrap();
        assert!(profile.audit_and_prepare().is_err());
        assert!(!profile.handy_root.exists());
        std::fs::remove_file(profile.inputia_root.join("memory.db-shm")).unwrap();
        std::fs::hard_link(&outside, profile.inputia_root.join("memory.db-wal")).unwrap();
        assert!(profile.audit_and_prepare().is_err());
        assert!(!profile.handy_root.exists());
        assert_eq!(std::fs::read(outside).unwrap(), b"untouched");
    }
    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_parent() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        std::fs::create_dir(base.join("actual")).unwrap();
        symlink(base.join("actual"), base.join("HandyUnifiedCandidate")).unwrap();
        assert!(candidate("trial", &base)
            .unwrap()
            .unwrap()
            .audit_and_prepare()
            .is_err());
        assert!(!base.join("actual/trial").exists());
    }

    #[cfg(unix)]
    #[test]
    fn makes_only_owned_run_root_private_and_preserves_regular_file_modes() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o755)).unwrap();
        let profile = candidate("existing", &base).unwrap().unwrap();
        std::fs::create_dir_all(&profile.handy_root).unwrap();
        let run_root = profile.handy_root.parent().unwrap();
        std::fs::set_permissions(run_root, std::fs::Permissions::from_mode(0o755)).unwrap();
        let file = profile.handy_root.join("settings.json");
        std::fs::write(&file, b"synthetic").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        profile.audit_and_prepare().unwrap();
        assert_eq!(std::fs::metadata(run_root).unwrap().mode() & 0o777, 0o700);
        assert_eq!(std::fs::metadata(&base).unwrap().mode() & 0o777, 0o755);
        assert_eq!(std::fs::metadata(file).unwrap().mode() & 0o777, 0o644);
    }
}
