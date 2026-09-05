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
use tauri::AppHandle;
use tauri_specta::Event;

#[derive(Clone, Debug, Serialize, Type, tauri_specta::Event)]
pub struct UnifiedHistoryUpdate {
    pub generation: u64,
}

pub struct IntegrationManager {
    pub service: Arc<HistoryService>,
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
        let service = HistoryService::start(root, "handy-local".into(), move |generation| {
            if (UnifiedHistoryUpdate { generation }).emit(&app).is_err() {
                log::warn!("Unable to notify unified history change");
            }
        })
        .map_err(anyhow::Error::msg)?;
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
                "SELECT schema_version>=3 FROM unified_source_meta",
                [],
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
