//! 菜单与界面共用的配套更新入口。没有可信发布配置时明确不可检查。
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::AppHandle;

static CHECKING: AtomicBool = AtomicBool::new(false);
struct CheckGuard;
impl Drop for CheckGuard {
    fn drop(&mut self) {
        CHECKING.store(false, Ordering::Release);
    }
}

#[derive(Clone, Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum UpdateStatus {
    Disabled,
    Checking,
    Current,
    Available,
    Unavailable,
}
#[derive(Clone, Serialize, specta::Type)]
pub struct UpdateCheckReply {
    pub status: UpdateStatus,
    pub reason: Option<String>,
    pub version: Option<String>,
    pub release_id: Option<String>,
    pub installable: bool,
}
impl UpdateCheckReply {
    fn state(status: UpdateStatus, reason: Option<&str>) -> Self {
        Self {
            status,
            reason: reason.map(str::to_owned),
            version: None,
            release_id: None,
            installable: false,
        }
    }
    fn unavailable(reason: &str) -> Self {
        Self::state(UpdateStatus::Unavailable, Some(reason))
    }
}

#[tauri::command]
#[specta::specta]
pub async fn check_product_update(app: AppHandle) -> UpdateCheckReply {
    if !crate::settings::update_checks_effectively_enabled(&crate::settings::get_settings(&app)) {
        return UpdateCheckReply::state(UpdateStatus::Disabled, None);
    }
    if CHECKING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return UpdateCheckReply::state(UpdateStatus::Checking, None);
    }
    let _guard = CheckGuard;
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    let reply =
        match tokio::time::timeout(std::time::Duration::from_secs(120), macos::check()).await {
            Ok(reply) => reply,
            Err(_) => UpdateCheckReply::unavailable("network"),
        };
    #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
    let reply = UpdateCheckReply::unavailable("unsupported_platform");
    if !crate::settings::update_checks_effectively_enabled(&crate::settings::get_settings(&app)) {
        return UpdateCheckReply::state(UpdateStatus::Disabled, None);
    }
    reply
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod macos {
    use super::*;
    use crate::update_download::HttpsCatalogSource;
    use inputia_release::{
        catalog::{self, CatalogStatus, CheckContext, CheckError, Clock, InstalledCatalog},
        trust::Track,
    };
    use serde::Deserialize;
    struct SystemClock;
    impl Clock for SystemClock {
        fn now(&self) -> i64 {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_secs().min(i64::MAX as u64) as i64)
                .unwrap_or(-1)
        }
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct CatalogReceipt {
        schema_version: u32,
        release_id: String,
        archive_policy_id: String,
        manifest_digest: String,
    }

    pub async fn check() -> UpdateCheckReply {
        let result = check_inner().await;
        match result {
            Ok(value) => value,
            Err(reason) => UpdateCheckReply::unavailable(reason),
        }
    }
    async fn check_inner() -> Result<UpdateCheckReply, &'static str> {
        let config =
            catalog::embedded_config(include_bytes!("../../../release/update-source.json"))
                .map_err(|_| "invalid_build_configuration")?
                .ok_or("source_unconfigured")?;
        let context = inputia_settings::maintenance::current_user_context()
            .map_err(|_| "installation_repair")?;
        inputia_settings::maintenance::ensure_normal_start(&context.home, context.uid)
            .map_err(|_| "maintenance")?;
        let startup = crate::candidate_profile::current()
            .and_then(|profile| profile.installation.as_ref())
            .ok_or("installation_repair")?;
        let binding = crate::native_pair_auth::release_build_trust()
            .map_err(|_| "installation_repair")?
            .ok_or("installation_repair")?
            .release_binding();
        let locator = inputia_settings::installation::LocatorContext {
            product_id: binding.product_id.into(),
            release_id: binding.release_id.into(),
            home: context.home.clone(),
            uid: context.uid,
        };
        let installation = crate::update_installation::inspect(&locator, startup)?;
        let root = installation
            .pair_manifest
            .parent()
            .ok_or("installation_repair")?;
        let read = |file: &str, limit| {
            inputia_settings::installation::read_owned_file(
                &root.join(file),
                context.uid,
                limit,
                true,
            )
            .map_err(|_| "installation_repair")
        };
        let receipt: CatalogReceipt = serde_json::from_value(
            inputia_release::canonical::parse(&read("catalog-receipt.json", 4096)?)
                .map_err(|_| "installation_repair")?,
        )
        .map_err(|_| "installation_repair")?;
        if receipt.schema_version != 1 || receipt.release_id != binding.release_id {
            return Err("installation_repair");
        }
        let manifest = read("release-manifest.json", catalog::DOCUMENT_LIMIT)?;
        let source = HttpsCatalogSource::new(&config).map_err(|_| "invalid_build_configuration")?;
        let track = Track {
            channel: match installation.receipt.channel {
                inputia_settings::installation::UpdateChannel::Candidate => "candidate",
                inputia_settings::installation::UpdateChannel::Stable => "stable",
            }
            .into(),
            platform: "macos".into(),
            architecture: "arm64".into(),
        };
        let os = objc2_foundation::NSProcessInfo::processInfo().operatingSystemVersion();
        let os_version = format!(
            "{}.{}.{}",
            os.majorVersion, os.minorVersion, os.patchVersion
        );
        let checked = catalog::check_catalog(
            &context.home,
            config
                .trust_root()
                .map_err(|_| "invalid_build_configuration")?,
            &source,
            &CheckContext {
                track: &track,
                os_version: &os_version,
                updater_version: "1.0.0",
                transaction_schema: 1,
                installed: InstalledCatalog {
                    release_id: binding.release_id,
                    manifest_digest: &receipt.manifest_digest,
                    archive_policy_id: &receipt.archive_policy_id,
                    signed_manifest: &manifest,
                },
            },
            &SystemClock,
        )
        .await
        .map_err(|error| match error {
            CheckError::Network => "network",
            CheckError::BudgetExceeded => "download_limit",
            CheckError::TrustRefreshRequired => "trust_refresh_required",
            CheckError::InstalledMetadataMissing | CheckError::InvalidInstalledMetadata => {
                "installation_repair"
            }
            CheckError::Release(inputia_release::ReleaseError::Incompatible) => "incompatible",
            CheckError::Release(_) => "verification_failed",
        })?;
        crate::update_installation::revalidate(&locator, &installation)?;
        // 这里只交付经过验签的目录结果；配套安装器接入后才开放安装动作。
        Ok(UpdateCheckReply {
            status: match checked.status {
                CatalogStatus::Current => UpdateStatus::Current,
                CatalogStatus::Available => UpdateStatus::Available,
            },
            reason: (checked.status == CatalogStatus::Available)
                .then(|| "paired_installer_pending".into()),
            version: checked.metadata.manifest()["version"]
                .as_str()
                .map(str::to_owned),
            release_id: checked.metadata.manifest()["release_id"]
                .as_str()
                .map(str::to_owned),
            installable: false,
        })
    }
}
