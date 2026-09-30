//! 启动阶段与配置快照分离：业务入口只能在全部初始化完成后开放。
use super::{session::Coordinator, AppSettings};
use inputia_settings::store::{Error, InitializationIntent};
use serde::Serialize;
use std::{
    path::Path,
    sync::{Arc, Mutex, OnceLock, RwLock},
};

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RecoveryReason {
    SettingsInvalid,
    StorageUnavailable,
    MigrationRecoveryRequired,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub(crate) enum StartupStatus {
    Starting,
    Ready,
    Recovery {
        reason: RecoveryReason,
        settings_editable: bool,
    },
}

pub(crate) struct RuntimeSettings {
    initialization: Mutex<()>,
    coordinator: OnceLock<Arc<Coordinator>>,
    phase: RwLock<StartupStatus>,
}
impl Default for RuntimeSettings {
    fn default() -> Self {
        Self {
            initialization: Mutex::new(()),
            coordinator: OnceLock::new(),
            phase: RwLock::new(StartupStatus::Starting),
        }
    }
}
impl RuntimeSettings {
    pub fn preflight(path: &Path, home: &Path, uid: u32) -> Result<(), Error> {
        super::document::preflight(path, home, uid)
    }
    pub fn status(&self) -> StartupStatus {
        self.phase
            .read()
            .map(|phase| phase.clone())
            .unwrap_or(StartupStatus::Recovery {
                reason: RecoveryReason::StorageUnavailable,
                settings_editable: false,
            })
    }
    pub fn ready(&self) -> bool {
        self.status() == StartupStatus::Ready
    }

    /// 迁移 guard 已授权 Mutating 后调用；observer 必须先耐久登记初始化写入。
    pub fn initialize(
        &self,
        path: &Path,
        home: &Path,
        uid: u32,
        observer: &mut dyn FnMut(&InitializationIntent) -> Result<(), Error>,
    ) -> Result<(), String> {
        let _initialization = self
            .initialization
            .lock()
            .map_err(|_| "settings_state_unavailable".to_owned())?;
        if self.status() != StartupStatus::Starting || self.coordinator.get().is_some() {
            return Err("settings_startup_state".into());
        }
        let coordinator =
            Coordinator::open_observed(path, home, uid, observer).map_err(failure_code)?;
        self.coordinator
            .set(Arc::new(coordinator))
            .map_err(|_| "settings_already_initialized".to_owned())
    }

    /// 仅内部初始化可在 Starting 读取。IPC 的统一入口必须先检查 ready。
    pub fn read_for_component(&self) -> Result<AppSettings, String> {
        let phase = self
            .phase
            .read()
            .map_err(|_| "settings_state_unavailable")?;
        if matches!(*phase, StartupStatus::Recovery { .. }) {
            return Err("settings_recovery_required".into());
        }
        self.coordinator
            .get()
            .ok_or_else(|| "settings_not_initialized".to_owned())?
            .read()
            .map_err(failure_code)
    }

    pub fn save_for_component(&self, settings: &AppSettings) -> Result<(), String> {
        // 持有准入读锁直到操作结束；发布 Recovery 必须等已获准操作退出。
        let phase = self
            .phase
            .read()
            .map_err(|_| "settings_state_unavailable")?;
        if matches!(*phase, StartupStatus::Recovery { .. }) {
            return Err("settings_recovery_required".into());
        }
        self.coordinator
            .get()
            .ok_or_else(|| "settings_not_initialized".to_owned())?
            .save(settings)
            .map(|_| ())
            .map_err(failure_code)
    }

    pub fn finish_startup(&self) -> Result<(), String> {
        let _initialization = self
            .initialization
            .lock()
            .map_err(|_| "settings_state_unavailable".to_owned())?;
        let mut phase = self
            .phase
            .write()
            .map_err(|_| "settings_state_unavailable".to_owned())?;
        if *phase != StartupStatus::Starting || self.coordinator.get().is_none() {
            return Err("settings_startup_state".into());
        }
        *phase = StartupStatus::Ready;
        Ok(())
    }

    /// 只有迁移协调器证明没有待恢复源写入，才允许 settings_editable=true。
    /// 进入恢复后本进程不重新开放业务；修复后的下一次启动重新走整个初始化流程。
    pub fn enter_recovery(
        &self,
        reason: RecoveryReason,
        settings_editable: bool,
    ) -> Result<(), String> {
        let _initialization = self
            .initialization
            .lock()
            .map_err(|_| "settings_state_unavailable".to_owned())?;
        let mut phase = self
            .phase
            .write()
            .map_err(|_| "settings_state_unavailable".to_owned())?;
        if *phase == StartupStatus::Ready {
            return Err("settings_startup_already_completed".into());
        }
        *phase = StartupStatus::Recovery {
            reason,
            settings_editable,
        };
        Ok(())
    }
}

fn failure_code(error: super::session::Failure) -> String {
    // Failure 只有固定错误类别和操作 ID，不含配置值、路径或凭据。
    serde_json::to_string(&error).unwrap_or_else(|_| "settings_state_unavailable".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{get_default_settings, SETTINGS_STORE_PATH};

    #[test]
    fn business_admission_requires_initialized_settings_and_completed_startup() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().canonicalize().unwrap();
        let runtime = RuntimeSettings::default();
        assert!(!runtime.ready());
        assert!(runtime.read_for_component().is_err());
        assert!(runtime.finish_startup().is_err());
        runtime
            .initialize(
                &home.join(SETTINGS_STORE_PATH),
                &home,
                unsafe { libc::geteuid() },
                &mut |_| Ok(()),
            )
            .unwrap();
        assert!(!runtime.ready());
        assert!(runtime.read_for_component().is_ok());
        assert!(runtime.save_for_component(&get_default_settings()).is_err());
        runtime
            .enter_recovery(RecoveryReason::MigrationRecoveryRequired, false)
            .unwrap();
        assert!(!runtime.ready());
        assert!(runtime.read_for_component().is_err());
        assert!(runtime.finish_startup().is_err());
        assert!(runtime
            .initialize(
                &home.join(SETTINGS_STORE_PATH),
                &home,
                unsafe { libc::geteuid() },
                &mut |_| Ok(())
            )
            .is_err());
        let ready = RuntimeSettings::default();
        ready
            .initialize(
                &home.join(SETTINGS_STORE_PATH),
                &home,
                unsafe { libc::geteuid() },
                &mut |_| Ok(()),
            )
            .unwrap();
        ready.finish_startup().unwrap();
        assert!(ready.ready());
        assert!(ready
            .enter_recovery(RecoveryReason::SettingsInvalid, true)
            .is_err());
    }

    #[test]
    fn failed_creation_journal_never_installs_a_default_runtime() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().canonicalize().unwrap();
        let path = home.join(SETTINGS_STORE_PATH);
        let runtime = RuntimeSettings::default();
        assert!(runtime
            .initialize(&path, &home, unsafe { libc::geteuid() }, &mut |_| Err(
                Error::CommitUncertain
            ))
            .is_err());
        assert!(!path.exists());
        assert!(runtime.read_for_component().is_err());
        assert!(!runtime.ready());
    }
}
