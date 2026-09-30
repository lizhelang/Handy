//! 统一历史服务的应用适配：先备份源 schema，再启动后台唯一写入者。

use anyhow::Result;
use inputia_handy_runtime::{
    service::HistoryService,
    source::{SourceOutbox, SourceTable},
};
use rusqlite::Connection;
use serde::Serialize;
use specta::Type;
use std::{path::Path, sync::Arc};
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

#[derive(Clone, Debug, Serialize, Type, tauri_specta::Event)]
pub struct UnifiedHistoryUpdate {
    pub generation: u64,
}

pub struct IntegrationManager {
    pub service: Arc<HistoryService>,
}

/// 不等待 IPC/数据库；在唯一服务初始化前没有可撤销的输出许可。
pub(crate) fn begin_source_write(
    app: &AppHandle,
) -> Option<inputia_handy_runtime::service::SourceWriteGuard> {
    app.try_state::<Arc<IntegrationManager>>()
        .map(|manager| manager.service.begin_source_write())
}

impl IntegrationManager {
    pub fn new(app: &AppHandle) -> Result<Self> {
        let root = crate::portable::app_data_dir(app)?;
        for (file, source) in [
            ("history.db", SourceTable::History),
            ("clipboard.db", SourceTable::Clipboard),
        ] {
            prepare_source(&root.join(file), source)?;
        }
        let app = app.clone();
        let profile_id = crate::candidate_profile::current()
            .map(|profile| profile.profile_id.clone())
            .unwrap_or_else(|| "handy-local".into());
        let memory_context = crate::candidate_profile::current()
            .map(|profile| {
                inputia_handy_runtime::legacy_memory::LegacyMemoryContext::handoff_required(
                    profile.inputia_root.join("inputia_memory.db"),
                    profile.profile_id.clone(),
                )
            })
            .unwrap_or_else(
                inputia_handy_runtime::legacy_memory::LegacyMemoryContext::unconfigured,
            );
        let service = HistoryService::start_with_memory(
            root,
            profile_id,
            memory_context,
            move |generation| {
                if (UnifiedHistoryUpdate { generation }).emit(&app).is_err() {
                    log::warn!("Unable to notify unified history change");
                }
            },
        )
        .map_err(anyhow::Error::msg)?;
        // start 只创建后台线程；确认实际数据库/密钥初始化成功后，外层才能提交启动迁移标记。
        service.policy_epoch().map_err(anyhow::Error::msg)?;
        Ok(Self {
            service: Arc::new(service),
        })
    }
}

fn prepare_source(path: &Path, source: SourceTable) -> Result<()> {
    let installed = {
        let conn = Connection::open(path)?;
        let exists=conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='unified_source_meta')",[],|row|row.get::<_,bool>(0))?;
        exists
            && conn.query_row(
                "SELECT schema_version>=?1 FROM unified_source_meta",
                [inputia_handy_runtime::source::SOURCE_SCHEMA_VERSION],
                |row| row.get::<_, bool>(0),
            )?
    };
    if !installed {
        let backup_root = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("source has no parent"))?
            .join("migration_backups")
            .join("unified-input-v1")
            .join(source.logical_name());
        crate::data_migration::run_sqlite_migration_with_backup(path, &backup_root, |conn| {
            SourceOutbox::install(conn, source)?;
            Ok(())
        })?;
    }
    Ok(())
}
