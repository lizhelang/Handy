//! 启动设置校验先于任何业务模块；恢复页面只有固定的两项操作。
use crate::data_migration;
#[cfg(not(unix))]
use crate::data_migration::StartupMigration;
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;

#[cfg(unix)]
use crate::settings::runtime::{RecoveryReason, RuntimeSettings, StartupStatus};

pub(crate) fn business_ready(app: &AppHandle) -> bool {
    #[cfg(unix)]
    {
        app.try_state::<RuntimeSettings>()
            .is_some_and(|state| state.ready())
    }
    #[cfg(not(unix))]
    {
        let _ = app;
        true
    }
}

#[cfg(unix)]
fn enter_recovery(app: &AppHandle, reason: RecoveryReason, editable: bool) {
    if app
        .state::<RuntimeSettings>()
        .enter_recovery(reason, editable)
        .is_err()
    {
        log::error!("startup_recovery_state_unavailable");
    }
}

#[cfg(unix)]
pub(crate) struct StartupGuard {
    app: AppHandle,
    completed: bool,
}
#[cfg(not(unix))]
pub(crate) type StartupGuard = StartupMigration;
#[cfg(unix)]
impl StartupGuard {
    pub fn complete(&mut self) -> anyhow::Result<()> {
        self.app
            .state::<RuntimeSettings>()
            .confirm_startup()
            .map_err(anyhow::Error::msg)?;
        self.completed = true;
        Ok(())
    }
}
#[cfg(unix)]
impl Drop for StartupGuard {
    fn drop(&mut self) {
        if !self.completed {
            // manager 可能已打开业务资源；这里只关闭准入，不能在线恢复 DB。
            enter_recovery(&self.app, RecoveryReason::MigrationRecoveryRequired, false);
        }
    }
}

/// 只有此函数成功返回后，调用者才可构造业务 manager。
pub(crate) fn prepare(app: &AppHandle) -> anyhow::Result<Option<StartupGuard>> {
    #[cfg(unix)]
    {
        let home = match app.path().home_dir() {
            Ok(home) => home,
            Err(_) => {
                enter_recovery(app, RecoveryReason::StorageUnavailable, false);
                anyhow::bail!("startup_storage_unavailable");
            }
        };
        if let Err(error) = app
            .state::<RuntimeSettings>()
            .prepare_app(app, &home, unsafe { libc::geteuid() })
        {
            let editable = error
                .downcast_ref::<data_migration::StartupFailure>()
                .is_some_and(|failure| failure.allows_configuration_repair());
            enter_recovery(
                app,
                if editable {
                    RecoveryReason::SettingsInvalid
                } else {
                    RecoveryReason::MigrationRecoveryRequired
                },
                editable,
            );
            anyhow::bail!("startup_settings_preparation_failed");
        }
        Ok(Some(StartupGuard {
            app: app.clone(),
            completed: false,
        }))
    }
    #[cfg(not(unix))]
    {
        let mut migration = data_migration::prepare_startup_backup(app)?;
        if let Some(guard) = migration.as_mut() {
            guard.begin_mutations()?;
        }
        Ok(migration)
    }
}

pub(crate) fn finish(app: &AppHandle) -> Result<(), String> {
    #[cfg(unix)]
    {
        app.state::<RuntimeSettings>().finish_startup()
    }
    #[cfg(not(unix))]
    {
        let _ = app;
        Ok(())
    }
}

#[tauri::command]
pub(crate) fn control_settings_status(app: AppHandle) -> serde_json::Value {
    #[cfg(unix)]
    {
        let runtime = app.state::<RuntimeSettings>();
        let mut status = serde_json::to_value(runtime.status())
            .unwrap_or_else(|_| serde_json::json!({ "phase": "starting" }));
        if let Some(reconciliation) = runtime.reconciliation() {
            status["settings_reconciliation"] =
                serde_json::to_value(reconciliation).unwrap_or(serde_json::Value::Null);
        }
        status
    }
    #[cfg(not(unix))]
    {
        let _ = app;
        serde_json::json!({ "phase": "ready" })
    }
}

#[tauri::command]
pub(crate) fn control_settings_recovery(app: AppHandle, action: String) -> Result<(), String> {
    #[cfg(unix)]
    if !matches!(
        app.state::<RuntimeSettings>().status(),
        StartupStatus::Recovery { .. }
    ) {
        return Err("startup_recovery_not_active".into());
    }
    #[cfg(not(unix))]
    return Err("startup_recovery_not_active".into());

    #[cfg(unix)]
    match action.as_str() {
        "restart" => app.restart(),
        "open_data_folder" => {
            // 不接收页面传来的路径，且不读取/显示配置正文或凭据。
            let path =
                crate::portable::app_data_dir(&app).map_err(|_| "startup_directory_unavailable")?;
            app.opener()
                .open_path(path.to_string_lossy(), None::<String>)
                .map_err(|_| "startup_directory_unavailable".into())
        }
        _ => Err("startup_action_unsupported".into()),
    }
}
