//! 模型存储预算。降低预算只限制新的增长，不授权删除现有模型或续传文件。
use serde::{Deserialize, Serialize};
use specta::Type;
use std::{
    fs::{File, OpenOptions},
    path::Path,
};

const GIB: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Type, PartialEq, Eq)]
pub struct ModelStorageBudget {
    pub max_download_bytes: u64,
    pub max_extracted_bytes: u64,
    pub max_archive_entries: u32,
}
impl Default for ModelStorageBudget {
    fn default() -> Self {
        Self {
            max_download_bytes: 16 * GIB,
            max_extracted_bytes: 32 * GIB,
            max_archive_entries: 100_000,
        }
    }
}
impl ModelStorageBudget {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.max_download_bytes == 0
            || self.max_download_bytes > 1024 * 1024 * GIB
            || self.max_extracted_bytes == 0
            || self.max_extracted_bytes > 1024 * 1024 * GIB
            || self.max_archive_entries == 0
            || self.max_archive_entries > 1_000_000
        {
            return Err("model_storage_budget_invalid");
        }
        Ok(())
    }
}
impl<'de> Deserialize<'de> for ModelStorageBudget {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Values {
            max_download_bytes: u64,
            max_extracted_bytes: u64,
            max_archive_entries: u32,
        }
        let value = Values::deserialize(deserializer)?;
        let budget = Self {
            max_download_bytes: value.max_download_bytes,
            max_extracted_bytes: value.max_extracted_bytes,
            max_archive_entries: value.max_archive_entries,
        };
        budget.validate().map_err(serde::de::Error::custom)?;
        Ok(budget)
    }
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StorageFailureCode {
    Busy,
    DownloadLimit,
    ArchiveSizeLimit,
    ArchiveEntryLimit,
    ArchiveMetadataLimit,
}
impl StorageFailureCode {
    pub fn code(self) -> &'static str {
        match self {
            Self::Busy => "model_storage_busy",
            Self::DownloadLimit => "model_storage_download_limit",
            Self::ArchiveSizeLimit => "model_storage_archive_size_limit",
            Self::ArchiveEntryLimit => "model_storage_archive_entry_limit",
            Self::ArchiveMetadataLimit => "model_storage_archive_metadata_limit",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ModelStorageFailure {
    pub code: StorageFailureCode,
    pub required_bytes: u64,
    pub limit_bytes: u64,
    pub resumable: bool,
}
impl std::fmt::Display for ModelStorageFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code.code())
    }
}
impl std::error::Error for ModelStorageFailure {}

pub(super) fn is_space_error(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        let Some(error) = cause.downcast_ref::<std::io::Error>() else {
            return false;
        };
        if error.kind() == std::io::ErrorKind::StorageFull {
            return true;
        }
        #[cfg(unix)]
        {
            matches!(error.raw_os_error(), Some(libc::ENOSPC | libc::EDQUOT))
        }
        #[cfg(not(unix))]
        {
            false
        }
    })
}

/// 一个受管模型目录同时只允许一个下载/解压/删除，取消令牌不会覆盖另一请求。
/// 固定独立锁跨进程持有；不锁模型文件本身，也不阻止已有模型被读取。
pub(super) struct ModelWriteLease {
    _file: File,
}
impl ModelWriteLease {
    pub fn acquire(root: &Path) -> anyhow::Result<Self> {
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        }
        let file = options.open(root.join(".inputia-model-storage.lock"))?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            anyhow::bail!("model_storage_unsafe_lock");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.nlink() != 1
                || metadata.uid() != unsafe { libc::geteuid() }
                || metadata.mode() & 0o777 != 0o600
            {
                anyhow::bail!("model_storage_unsafe_lock");
            }
        }
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(std::fs::TryLockError::WouldBlock) => Err(ModelStorageFailure {
                code: StorageFailureCode::Busy,
                required_bytes: 0,
                limit_bytes: 0,
                resumable: true,
            }
            .into()),
            Err(std::fs::TryLockError::Error(error)) => Err(error.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_limits_are_rejected_without_silent_defaults() {
        let value = serde_json::to_value(ModelStorageBudget::default()).unwrap();
        for (field, invalid) in [
            ("max_download_bytes", u64::MAX),
            ("max_extracted_bytes", 0),
            ("max_archive_entries", 1_000_001),
        ] {
            let mut changed = value.clone();
            changed[field] = invalid.into();
            assert!(serde_json::from_value::<ModelStorageBudget>(changed).is_err());
        }
        assert_eq!(
            serde_json::from_value::<ModelStorageBudget>(value).unwrap(),
            ModelStorageBudget::default()
        );
    }
    #[test]
    fn existing_settings_default_the_budget_but_invalid_explicit_values_fail() {
        let old: crate::settings::AppSettings =
            serde_json::from_value(serde_json::json!({"selected_model":"keep-model"})).unwrap();
        assert_eq!(old.selected_model, "keep-model");
        assert_eq!(old.model_storage, ModelStorageBudget::default());
        let malformed = serde_json::json!({"selected_model":"keep-model", "model_storage": {"max_download_bytes":0,"max_extracted_bytes":10,"max_archive_entries":10}});
        assert!(serde_json::from_value::<crate::settings::AppSettings>(malformed).is_err());
    }
    #[test]
    fn model_writes_are_exclusive_until_the_original_lease_is_dropped() {
        let temp = tempfile::tempdir().unwrap();
        let first = ModelWriteLease::acquire(temp.path()).unwrap();
        let error = ModelWriteLease::acquire(temp.path()).err().unwrap();
        assert_eq!(
            error.downcast_ref::<ModelStorageFailure>().unwrap().code,
            StorageFailureCode::Busy
        );
        drop(first);
        assert!(ModelWriteLease::acquire(temp.path()).is_ok());
    }
    #[test]
    #[cfg(unix)]
    fn wrapped_disk_and_quota_errors_remain_recoverable() {
        for errno in [libc::ENOSPC, libc::EDQUOT] {
            let error = anyhow::Error::new(std::io::Error::from_raw_os_error(errno))
                .context("unpack failed");
            assert!(is_space_error(&error));
            assert_eq!(
                super::super::download_failure_code(&error),
                Some("model_storage_insufficient_space")
            );
        }
        assert!(!is_space_error(&anyhow::anyhow!("bad archive")));
    }
}
