//! 启动设置校验先于任何业务模块；恢复页面只有固定的两项操作。
use crate::data_migration::{self, StartupMigration};
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

/// 只有此函数成功返回后，调用者才可构造业务 manager。
pub(crate) fn prepare(app: &AppHandle) -> anyhow::Result<Option<StartupMigration>> {
    #[cfg(unix)]
    {
        let paths = (|| -> anyhow::Result<_> {
            Ok((
                crate::portable::app_data_dir(app)?.join(crate::settings::SETTINGS_STORE_PATH),
                app.path().home_dir()?,
                unsafe { libc::geteuid() },
            ))
        })();
        let (path, home, uid) = match paths {
            Ok(paths) => paths,
            Err(_) => {
                enter_recovery(app, RecoveryReason::StorageUnavailable, false);
                anyhow::bail!("startup_storage_unavailable");
            }
        };
        let prepared = data_migration::prepare_startup_backup_with_preflight(app, || {
            RuntimeSettings::preflight(&path, &home, uid).map_err(Into::into)
        });
        let mut migration = match prepared {
            Ok(migration) => migration,
            Err(error) => {
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
                anyhow::bail!("startup_preflight_failed");
            }
        };
        if let Some(guard) = migration.as_mut() {
            if guard.begin_mutations().is_err() {
                enter_recovery(app, RecoveryReason::MigrationRecoveryRequired, false);
                anyhow::bail!("startup_mutation_authorization_failed");
            }
        }
        let result = app
            .state::<RuntimeSettings>()
            .initialize(&path, &home, uid, &mut |intent| {
                // 既有 Completed 不授权重建丢失文件；必须由新恢复流程明确处理。
                migration
                    .as_mut()
                    .ok_or(inputia_settings::store::Error::RepairRequired)?
                    .record_settings_initialization(intent)
                    .map_err(|_| inputia_settings::store::Error::CommitUncertain)
            });
        if result.is_err() {
            enter_recovery(app, RecoveryReason::MigrationRecoveryRequired, false);
            anyhow::bail!("startup_settings_initialization_failed");
        }
        Ok(migration)
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
        serde_json::to_value(app.state::<RuntimeSettings>().status())
            .unwrap_or_else(|_| serde_json::json!({ "phase": "starting" }))
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
