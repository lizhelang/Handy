use anyhow::{anyhow, Result};
use chrono::{DateTime, Local, Utc};
use log::{debug, error, info};
use rusqlite::{params, Connection, OptionalExtension};
use rusqlite_migration::{Migrations, M};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::fs;
use std::path::PathBuf;
use tauri::AppHandle;
use tauri_specta::Event;

/// Database migrations for transcription history.
/// Each migration is applied in order. The library tracks which migrations
/// have been applied using SQLite's user_version pragma.
///
/// Note: For users upgrading from tauri-plugin-sql, migrate_from_tauri_plugin_sql()
/// converts the old _sqlx_migrations table tracking to the user_version pragma,
/// ensuring migrations don't re-run on existing databases.
static MIGRATIONS: &[M] = &[
    M::up(
        "CREATE TABLE IF NOT EXISTS transcription_history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            file_name TEXT NOT NULL,
            timestamp INTEGER NOT NULL,
            saved BOOLEAN NOT NULL DEFAULT 0,
            title TEXT NOT NULL,
            transcription_text TEXT NOT NULL
        );",
    ),
    M::up("ALTER TABLE transcription_history ADD COLUMN post_processed_text TEXT;"),
    M::up("ALTER TABLE transcription_history ADD COLUMN post_process_prompt TEXT;"),
    M::up("ALTER TABLE transcription_history ADD COLUMN post_process_requested BOOLEAN NOT NULL DEFAULT 0;"),
];

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct PaginatedHistory {
    pub entries: Vec<HistoryEntry>,
    pub has_more: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type, tauri_specta::Event)]
#[serde(tag = "action")]
pub enum HistoryUpdatePayload {
    #[serde(rename = "added")]
    Added { entry: HistoryEntry },
    #[serde(rename = "updated")]
    Updated { entry: HistoryEntry },
    #[serde(rename = "deleted")]
    Deleted { id: i64 },
    #[serde(rename = "toggled")]
    Toggled { id: i64 },
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct HistoryEntry {
    pub id: i64,
    pub file_name: String,
    pub timestamp: i64,
    pub saved: bool,
    pub title: String,
    pub transcription_text: String,
    pub post_processed_text: Option<String>,
    pub post_process_prompt: Option<String>,
    pub post_process_requested: bool,
}

/// 仅供录音管线从协调器已接纳的认证会话构造；不是前端可提交的信任标记。
pub(crate) struct VerifiedVoiceSource {
    app: String,
}

impl VerifiedVoiceSource {
    pub(crate) fn from_owned_request(
        request: &inputia_handy_runtime::voice_protocol::VoiceRequest,
    ) -> Option<Self> {
        use inputia_handy_runtime::voice_protocol::VoiceCommand;
        let target = match &request.command {
            VoiceCommand::Start { target, .. } => target,
            VoiceCommand::HostShortcut { target, edge, .. } if edge.starts_session => target,
            _ => return None,
        };
        let app = target.source_app.as_deref()?;
        if target.field_id.as_deref().is_none_or(str::is_empty)
            || app.is_empty()
            || app.len() > 256
            || app.chars().any(char::is_control)
            || inputia_core::AppPolicy::default().excludes(&inputia_core::AppContext::new(app))
        {
            return None;
        }
        Some(Self {
            app: app.to_owned(),
        })
    }
}

fn insert_entry_with_voice_source(
    conn: &Connection,
    entry: HistoryEntry,
    source: Option<&VerifiedVoiceSource>,
) -> Result<HistoryEntry> {
    let Some(source) = source else {
        return insert_entry_with_conn(conn, entry);
    };
    // 同一INSERT同时写正文及来源，outbox触发器只产生一个完整事件。
    let mut entry = entry;
    conn.execute(
        "INSERT INTO transcription_history (file_name,timestamp,saved,title,transcription_text,
         post_processed_text,post_process_prompt,post_process_requested,inputia_source_app,inputia_source_trust)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,'verified')",
        params![entry.file_name,entry.timestamp,entry.saved,entry.title,entry.transcription_text,
                entry.post_processed_text,entry.post_process_prompt,entry.post_process_requested,source.app],
    )?;
    entry.id = conn.last_insert_rowid();
    Ok(entry)
}

/// 生产与故障回归共用的 SQLite 插入边界；不依赖窗口或录音文件写入成功。
pub(crate) fn insert_entry_with_conn(
    conn: &Connection,
    mut entry: HistoryEntry,
) -> Result<HistoryEntry> {
    conn.execute(
        "INSERT INTO transcription_history (
            file_name, timestamp, saved, title, transcription_text,
            post_processed_text, post_process_prompt, post_process_requested
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            &entry.file_name,
            entry.timestamp,
            entry.saved,
            &entry.title,
            &entry.transcription_text,
            &entry.post_processed_text,
            &entry.post_process_prompt,
            entry.post_process_requested
        ],
    )?;
    entry.id = conn.last_insert_rowid();
    Ok(entry)
}

/// 空文件名表示无附件，不能把 recordings 目录交给文件删除操作。
fn remove_recording_attachment(
    recordings: &std::path::Path,
    file_name: &str,
    remove_file: impl FnOnce(&std::path::Path) -> std::io::Result<()>,
) -> std::io::Result<bool> {
    if file_name.is_empty() {
        return Ok(false);
    }
    let path = recordings.join(file_name);
    if !path.exists() {
        return Ok(false);
    }
    remove_file(&path)?;
    Ok(true)
}

pub struct HistoryManager {
    app_handle: AppHandle,
    recordings_dir: PathBuf,
    db_path: PathBuf,
}

impl HistoryManager {
    pub fn new(app_handle: &AppHandle) -> Result<Self> {
        // Create recordings directory in app data dir
        let app_data_dir = crate::portable::app_data_dir(app_handle)?;
        let recordings_dir = app_data_dir.join("recordings");
        let db_path = app_data_dir.join("history.db");

        // Ensure recordings directory exists
        if !recordings_dir.exists() {
            fs::create_dir_all(&recordings_dir)?;
            debug!("Created recordings directory: {:?}", recordings_dir);
        }

        let manager = Self {
            app_handle: app_handle.clone(),
            recordings_dir,
            db_path,
        };

        // Initialize database and run migrations synchronously
        manager.init_database()?;

        Ok(manager)
    }

    fn init_database(&self) -> Result<()> {
        info!("Initializing database at {:?}", self.db_path);

        let mut conn = Connection::open(&self.db_path)?;

        // Handle migration from tauri-plugin-sql to rusqlite_migration
        // tauri-plugin-sql used _sqlx_migrations table, rusqlite_migration uses user_version pragma
        self.migrate_from_tauri_plugin_sql(&conn)?;

        // Create migrations object and run to latest version
        let migrations = Migrations::new(MIGRATIONS.to_vec());

        // Validate migrations in debug builds
        #[cfg(debug_assertions)]
        migrations.validate().expect("Invalid migrations");

        // Get current version before migration
        let version_before: i32 =
            conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        debug!("Database version before migration: {}", version_before);

        // Apply any pending migrations
        migrations.to_latest(&mut conn)?;

        // Get version after migration
        let version_after: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;

        if version_after > version_before {
            info!(
                "Database migrated from version {} to {}",
                version_before, version_after
            );
        } else {
            debug!("Database already at latest version {}", version_after);
        }

        Ok(())
    }

    /// Migrate from tauri-plugin-sql's migration tracking to rusqlite_migration's.
    /// tauri-plugin-sql used a _sqlx_migrations table, while rusqlite_migration uses
    /// SQLite's user_version pragma. This function checks if the old system was in use
    /// and sets the user_version accordingly so migrations don't re-run.
    fn migrate_from_tauri_plugin_sql(&self, conn: &Connection) -> Result<()> {
        // Check if the old _sqlx_migrations table exists
        let has_sqlx_migrations: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='_sqlx_migrations'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(false);

        if !has_sqlx_migrations {
            return Ok(());
        }

        // Check current user_version
        let current_version: i32 =
            conn.pragma_query_value(None, "user_version", |row| row.get(0))?;

        if current_version > 0 {
            // Already migrated to rusqlite_migration system
            return Ok(());
        }

        // Get the highest version from the old migrations table
        let old_version: i32 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM _sqlx_migrations WHERE success = 1",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if old_version > 0 {
            info!(
                "Migrating from tauri-plugin-sql (version {}) to rusqlite_migration",
                old_version
            );

            // Set user_version to match the old migration state
            conn.pragma_update(None, "user_version", old_version)?;

            // Optionally drop the old migrations table (keeping it doesn't hurt)
            // conn.execute("DROP TABLE IF EXISTS _sqlx_migrations", [])?;

            info!(
                "Migration tracking converted: user_version set to {}",
                old_version
            );
        }

        Ok(())
    }

    fn get_connection(&self) -> Result<Connection> {
        Ok(Connection::open(&self.db_path)?)
    }

    fn map_history_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<HistoryEntry> {
        Ok(HistoryEntry {
            id: row.get("id")?,
            file_name: row.get("file_name")?,
            timestamp: row.get("timestamp")?,
            saved: row.get("saved")?,
            title: row.get("title")?,
            transcription_text: row.get("transcription_text")?,
            post_processed_text: row.get("post_processed_text")?,
            post_process_prompt: row.get("post_process_prompt")?,
            post_process_requested: row.get("post_process_requested")?,
        })
    }

    pub fn recordings_dir(&self) -> &std::path::Path {
        &self.recordings_dir
    }

    /// Save a new history entry to the database.
    /// Non-empty file_name references a verified WAV; empty means text without an audio attachment.
    pub fn save_entry(
        &self,
        file_name: String,
        transcription_text: String,
        post_process_requested: bool,
        post_processed_text: Option<String>,
        post_process_prompt: Option<String>,
    ) -> Result<HistoryEntry> {
        self.save_entry_with_voice_source(
            file_name,
            transcription_text,
            post_process_requested,
            post_processed_text,
            post_process_prompt,
            None,
        )
    }

    pub(crate) fn save_entry_with_voice_source(
        &self,
        file_name: String,
        transcription_text: String,
        post_process_requested: bool,
        post_processed_text: Option<String>,
        post_process_prompt: Option<String>,
        source: Option<&VerifiedVoiceSource>,
    ) -> Result<HistoryEntry> {
        let _source_write = super::integration::begin_source_write(&self.app_handle);
        let timestamp = Utc::now().timestamp();
        let title = self.format_timestamp_title(timestamp);

        let conn = self.get_connection()?;
        let entry = insert_entry_with_voice_source(
            &conn,
            HistoryEntry {
                id: 0,
                file_name,
                timestamp,
                saved: false,
                title,
                transcription_text,
                post_processed_text,
                post_process_prompt,
                post_process_requested,
            },
            source,
        )?;

        debug!("Saved history entry with id {}", entry.id);

        self.cleanup_old_entries()?;

        // Emit typed event for real-time frontend updates
        if let Err(e) = (HistoryUpdatePayload::Added {
            entry: entry.clone(),
        })
        .emit(&self.app_handle)
        {
            error!("Failed to emit history-updated event: {}", e);
        }

        Ok(entry)
    }

    /// Update an existing history entry with new transcription results (used by retry).
    pub fn update_transcription_checked(
        &self,
        id: i64,
        transcription_text: String,
        post_processed_text: Option<String>,
        post_process_prompt: Option<String>,
        expected_revision: Option<u64>,
    ) -> Result<HistoryEntry> {
        let _source_write = super::integration::begin_source_write(&self.app_handle);
        let mut connection = self.get_connection()?;
        let conn =
            connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if let Some(expected) = expected_revision {
            let current: u64 = conn.query_row(
                "SELECT revision FROM unified_source_versions WHERE record_id=?1",
                [id.to_string()],
                |row| row.get(0),
            )?;
            if current != expected {
                return Err(anyhow!("History entry changed during transcription"));
            }
        }
        let updated = conn.execute(
            "UPDATE transcription_history
             SET transcription_text = ?1,
                 post_processed_text = ?2,
                 post_process_prompt = ?3
             WHERE id = ?4",
            params![
                transcription_text,
                post_processed_text,
                post_process_prompt,
                id
            ],
        )?;

        if updated == 0 {
            return Err(anyhow!("History entry {} not found", id));
        }

        let entry = conn
            .query_row(
                "SELECT id, file_name, timestamp, saved, title, transcription_text, post_processed_text, post_process_prompt, post_process_requested
                 FROM transcription_history WHERE id = ?1",
                params![id],
                Self::map_history_entry,
            )?;

        conn.commit()?;

        debug!("Updated transcription for history entry {}", id);

        if let Err(e) = (HistoryUpdatePayload::Updated {
            entry: entry.clone(),
        })
        .emit(&self.app_handle)
        {
            error!("Failed to emit history-updated event: {}", e);
        }

        Ok(entry)
    }

    pub fn cleanup_old_entries(&self) -> Result<()> {
        let retention_period = crate::settings::get_recording_retention_period(&self.app_handle);

        match retention_period {
            crate::settings::RecordingRetentionPeriod::Never => {
                // Don't delete anything
                Ok(())
            }
            crate::settings::RecordingRetentionPeriod::PreserveLimit => {
                // Use the old count-based logic with history_limit
                let limit = crate::settings::get_history_limit(&self.app_handle);
                self.cleanup_by_count(limit)
            }
            _ => {
                // Use time-based logic
                self.cleanup_by_time(retention_period)
            }
        }
    }

    fn delete_entries_and_files(&self, entries: &[(i64, String)]) -> Result<usize> {
        let _source_write = super::integration::begin_source_write(&self.app_handle);
        if entries.is_empty() {
            return Ok(0);
        }

        let conn = self.get_connection()?;
        let mut deleted_count = 0;

        for (id, file_name) in entries {
            // Delete database entry
            conn.execute(
                "DELETE FROM transcription_history WHERE id = ?1",
                params![id],
            )?;

            // Delete WAV file
            match remove_recording_attachment(&self.recordings_dir, file_name, |path| {
                fs::remove_file(path)
            }) {
                Err(e) => error!("Failed to delete WAV file {}: {}", file_name, e),
                Ok(true) => {
                    debug!("Deleted old WAV file: {}", file_name);
                    deleted_count += 1;
                }
                Ok(false) => {}
            }
        }

        Ok(deleted_count)
    }

    fn cleanup_by_count(&self, limit: usize) -> Result<()> {
        let conn = self.get_connection()?;

        // Get all entries that are not saved, ordered by timestamp desc
        let mut stmt = conn.prepare(
            "SELECT id, file_name FROM transcription_history WHERE saved = 0 ORDER BY timestamp DESC"
        )?;

        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>("id")?, row.get::<_, String>("file_name")?))
        })?;

        let mut entries: Vec<(i64, String)> = Vec::new();
        for row in rows {
            entries.push(row?);
        }

        if entries.len() > limit {
            let entries_to_delete = &entries[limit..];
            let deleted_count = self.delete_entries_and_files(entries_to_delete)?;

            if deleted_count > 0 {
                debug!("Cleaned up {} old history entries by count", deleted_count);
            }
        }

        Ok(())
    }

    fn cleanup_by_time(
        &self,
        retention_period: crate::settings::RecordingRetentionPeriod,
    ) -> Result<()> {
        let conn = self.get_connection()?;

        // Calculate cutoff timestamp (current time minus retention period)
        let now = Utc::now().timestamp();
        let cutoff_timestamp = match retention_period {
            crate::settings::RecordingRetentionPeriod::Days3 => now - (3 * 24 * 60 * 60), // 3 days in seconds
            crate::settings::RecordingRetentionPeriod::Weeks2 => now - (2 * 7 * 24 * 60 * 60), // 2 weeks in seconds
            crate::settings::RecordingRetentionPeriod::Months3 => now - (3 * 30 * 24 * 60 * 60), // 3 months in seconds (approximate)
            _ => unreachable!("Should not reach here"),
        };

        // Get all unsaved entries older than the cutoff timestamp
        let mut stmt = conn.prepare(
            "SELECT id, file_name FROM transcription_history WHERE saved = 0 AND timestamp < ?1",
        )?;

        let rows = stmt.query_map(params![cutoff_timestamp], |row| {
            Ok((row.get::<_, i64>("id")?, row.get::<_, String>("file_name")?))
        })?;

        let mut entries_to_delete: Vec<(i64, String)> = Vec::new();
        for row in rows {
            entries_to_delete.push(row?);
        }

        let deleted_count = self.delete_entries_and_files(&entries_to_delete)?;

        if deleted_count > 0 {
            debug!(
                "Cleaned up {} old history entries based on retention period",
                deleted_count
            );
        }

        Ok(())
    }

    pub async fn get_history_entries(
        &self,
        cursor: Option<i64>,
        limit: Option<usize>,
    ) -> Result<PaginatedHistory> {
        let conn = self.get_connection()?;
        let limit = limit.map(|l| l.min(100));

        let mut entries: Vec<HistoryEntry> = match (cursor, limit) {
            (Some(cursor_id), Some(lim)) => {
                let fetch_count = (lim + 1) as i64;
                let mut stmt = conn.prepare(
                    "SELECT id, file_name, timestamp, saved, title, transcription_text, post_processed_text, post_process_prompt, post_process_requested
                     FROM transcription_history
                     WHERE id < ?1
                     ORDER BY id DESC
                     LIMIT ?2",
                )?;
                let result = stmt
                    .query_map(params![cursor_id, fetch_count], Self::map_history_entry)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                result
            }
            (None, Some(lim)) => {
                let fetch_count = (lim + 1) as i64;
                let mut stmt = conn.prepare(
                    "SELECT id, file_name, timestamp, saved, title, transcription_text, post_processed_text, post_process_prompt, post_process_requested
                     FROM transcription_history
                     ORDER BY id DESC
                     LIMIT ?1",
                )?;
                let result = stmt
                    .query_map(params![fetch_count], Self::map_history_entry)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                result
            }
            (_, None) => {
                let mut stmt = conn.prepare(
                    "SELECT id, file_name, timestamp, saved, title, transcription_text, post_processed_text, post_process_prompt, post_process_requested
                     FROM transcription_history
                     ORDER BY id DESC",
                )?;
                let result = stmt
                    .query_map([], Self::map_history_entry)?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                result
            }
        };

        let has_more = limit.is_some_and(|lim| entries.len() > lim);
        if has_more {
            entries.pop();
        }

        Ok(PaginatedHistory { entries, has_more })
    }

    #[cfg(test)]
    fn get_latest_entry_with_conn(conn: &Connection) -> Result<Option<HistoryEntry>> {
        let mut stmt = conn.prepare(
            "SELECT
                id,
                file_name,
                timestamp,
                saved,
                title,
                transcription_text,
                post_processed_text,
                post_process_prompt,
                post_process_requested
             FROM transcription_history
             ORDER BY timestamp DESC
             LIMIT 1",
        )?;

        let entry = stmt.query_row([], Self::map_history_entry).optional()?;
        Ok(entry)
    }

    /// Get the latest entry with non-empty transcription text.
    pub fn get_latest_completed_entry(&self) -> Result<Option<HistoryEntry>> {
        let conn = self.get_connection()?;
        Self::get_latest_completed_entry_with_conn(&conn)
    }

    fn get_latest_completed_entry_with_conn(conn: &Connection) -> Result<Option<HistoryEntry>> {
        let mut stmt = conn.prepare(
            "SELECT
                id,
                file_name,
                timestamp,
                saved,
                title,
                transcription_text,
                post_processed_text,
                post_process_prompt,
                post_process_requested
             FROM transcription_history
             WHERE transcription_text != ''
             ORDER BY timestamp DESC
             LIMIT 1",
        )?;

        let entry = stmt.query_row([], Self::map_history_entry).optional()?;
        Ok(entry)
    }

    pub async fn toggle_saved_status(&self, id: i64) -> Result<()> {
        let _source_write = super::integration::begin_source_write(&self.app_handle);
        let conn = self.get_connection()?;

        // Get current saved status
        let current_saved: bool = conn.query_row(
            "SELECT saved FROM transcription_history WHERE id = ?1",
            params![id],
            |row| row.get("saved"),
        )?;

        let new_saved = !current_saved;

        conn.execute(
            "UPDATE transcription_history SET saved = ?1 WHERE id = ?2",
            params![new_saved, id],
        )?;

        debug!("Toggled saved status for entry {}: {}", id, new_saved);

        // Emit history updated event
        if let Err(e) = (HistoryUpdatePayload::Toggled { id }).emit(&self.app_handle) {
            error!("Failed to emit history-updated event: {}", e);
        }

        Ok(())
    }

    pub fn get_audio_file_path(&self, file_name: &str) -> PathBuf {
        self.recordings_dir.join(file_name)
    }

    pub async fn get_entry_by_id(&self, id: i64) -> Result<Option<HistoryEntry>> {
        let conn = self.get_connection()?;
        let mut stmt = conn.prepare(
            "SELECT
                id,
                file_name,
                timestamp,
                saved,
                title,
                transcription_text,
                post_processed_text,
                post_process_prompt,
                post_process_requested
             FROM transcription_history
             WHERE id = ?1",
        )?;

        let entry = stmt.query_row([id], Self::map_history_entry).optional()?;

        Ok(entry)
    }

    pub async fn delete_entry(&self, id: i64) -> Result<()> {
        let _source_write = super::integration::begin_source_write(&self.app_handle);
        let conn = self.get_connection()?;

        // Get the entry to find the file name
        if let Some(entry) = self.get_entry_by_id(id).await? {
            // Delete the audio file first
            if let Err(e) =
                remove_recording_attachment(&self.recordings_dir, &entry.file_name, |path| {
                    fs::remove_file(path)
                })
            {
                error!("Failed to delete audio file {}: {}", entry.file_name, e);
                // Continue with database deletion even if file deletion fails
            }
        }

        // Delete from database
        conn.execute(
            "DELETE FROM transcription_history WHERE id = ?1",
            params![id],
        )?;

        debug!("Deleted history entry with id: {}", id);

        // Emit history updated event
        if let Err(e) = (HistoryUpdatePayload::Deleted { id }).emit(&self.app_handle) {
            error!("Failed to emit history-updated event: {}", e);
        }

        Ok(())
    }

    fn format_timestamp_title(&self, timestamp: i64) -> String {
        if let Some(utc_datetime) = DateTime::from_timestamp(timestamp, 0) {
            // Convert UTC to local timezone
            let local_datetime = utc_datetime.with_timezone(&Local);
            local_datetime.format("%B %e, %Y - %l:%M%p").to_string()
        } else {
            format!("Recording {}", timestamp)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};

    fn owned_request() -> inputia_handy_runtime::voice_protocol::VoiceRequest {
        use inputia_handy_runtime::voice_protocol::*;
        VoiceRequest {
            request_id: "request".into(),
            session_id: "session".into(),
            server_instance: "server".into(),
            client_instance: "host".into(),
            policy_epoch: 1,
            command: VoiceCommand::Start {
                target: HostTargetToken {
                    target_id: "target".into(),
                    host_instance: "host".into(),
                    controller_id: "controller".into(),
                    activation_generation: 1,
                    field_id: Some("field".into()),
                    selection_generation: 1,
                    composition_generation: 1,
                    source_app: Some("synthetic.editor".into()),
                },
                post_process: false,
                terms: VoiceTermsVersion {
                    policy_epoch: 1,
                    learning_generation: 1,
                },
            },
        }
    }

    #[test]
    fn voice_source_requires_known_field_and_app_from_owned_start() {
        use inputia_handy_runtime::voice_protocol::VoiceCommand;
        let mut request = owned_request();
        assert!(VerifiedVoiceSource::from_owned_request(&request).is_some());
        if let VoiceCommand::Start { target, .. } = &mut request.command {
            target.field_id = None;
        }
        assert!(VerifiedVoiceSource::from_owned_request(&request).is_none());
        if let VoiceCommand::Start { target, .. } = &mut request.command {
            target.field_id = Some("field".into());
            target.source_app = Some("com.1password.1password".into());
        }
        assert!(VerifiedVoiceSource::from_owned_request(&request).is_none());
        request.command = VoiceCommand::Status;
        assert!(VerifiedVoiceSource::from_owned_request(&request).is_none());
    }

    #[test]
    fn voice_source_and_history_are_one_outbox_event_and_rollback_together() {
        use inputia_handy_runtime::{
            source::{SourceOutbox, SourceTable},
            store::SourceTrust,
        };
        let mut conn = setup_conn();
        let outbox = SourceOutbox::install(&mut conn, SourceTable::History).unwrap();
        let entry = HistoryEntry {
            id: 0,
            file_name: String::new(),
            timestamp: 1,
            saved: false,
            title: "synthetic".into(),
            transcription_text: "Inputia".into(),
            post_processed_text: None,
            post_process_prompt: None,
            post_process_requested: false,
        };
        let source = VerifiedVoiceSource::from_owned_request(&owned_request()).unwrap();
        {
            let tx = conn.unchecked_transaction().unwrap();
            insert_entry_with_voice_source(&tx, entry.clone(), Some(&source)).unwrap();
            tx.rollback().unwrap();
        }
        assert!(outbox.read_batch(&conn, 0, 10).unwrap().is_empty());
        insert_entry_with_voice_source(&conn, entry.clone(), Some(&source)).unwrap();
        let events = outbox.read_batch(&conn, 0, 10).unwrap();
        assert_eq!(events.len(), 1);
        let payload = events[0].payload.as_ref().unwrap();
        assert_eq!(payload.source_trust, SourceTrust::Verified);
        assert_eq!(payload.source_app.as_deref(), Some("synthetic.editor"));
        // 验证实际写入器产生的事件可进入规范确认链，不手工给索引伪造 Verified。
        let directory = tempfile::tempdir().unwrap();
        let mut index = inputia_handy_runtime::store::IntegrationStore::open(
            directory.path().join("integration.db"),
            "source-confirmation",
        )
        .unwrap();
        let key = [83; 32];
        index.register_source("history", outbox.store_id()).unwrap();
        index.enable_learning(&key).unwrap();
        index.apply_change(&events[0]).unwrap();
        let request = inputia_handy_runtime::learning::HistoryTermConfirmation {
            operation_id: inputia_core::integration::events::Identifier::parse("confirm-new-voice")
                .unwrap(),
            item_id: inputia_handy_runtime::store::item_id(outbox.store_id(), &events[0].record_id),
            expected_revision: events[0].revision,
            term: "Inputia".into(),
        };
        assert_eq!(
            index.confirm_history_term(&key, &request, || true).unwrap(),
            inputia_handy_runtime::learning::ApplyContribution::Applied
        );
        assert_eq!(index.list_terms(10, 0).unwrap()[0].contributions, 1);
        insert_entry_with_voice_source(&conn, entry, None).unwrap();
        assert_eq!(
            outbox.read_batch(&conn, 0, 10).unwrap()[1]
                .payload
                .as_ref()
                .unwrap()
                .source_trust,
            SourceTrust::Unknown
        );
    }

    #[test]
    fn no_attachment_never_calls_remove_on_recordings_directory() {
        let temp = tempfile::tempdir().unwrap();
        let recordings = temp.path().join("recordings");
        std::fs::create_dir(&recordings).unwrap();
        let neighbor = recordings.join("keep.wav");
        std::fs::write(&neighbor, b"synthetic").unwrap();
        let mut calls = 0;
        let removed = remove_recording_attachment(&recordings, "", |path| {
            calls += 1;
            std::fs::remove_file(path)
        })
        .unwrap();
        assert!(!removed);
        assert_eq!(calls, 0);
        assert!(recordings.is_dir());
        assert!(neighbor.is_file());
        assert!(
            remove_recording_attachment(&recordings, "keep.wav", |path| {
                calls += 1;
                assert_eq!(path, neighbor);
                std::fs::remove_file(path)
            })
            .unwrap()
        );
        assert_eq!(calls, 1);
        assert!(recordings.is_dir());
        assert!(!neighbor.exists());
    }

    fn setup_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch(
            "CREATE TABLE transcription_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_name TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                saved BOOLEAN NOT NULL DEFAULT 0,
                title TEXT NOT NULL,
                transcription_text TEXT NOT NULL,
                post_processed_text TEXT,
                post_process_prompt TEXT,
                post_process_requested BOOLEAN NOT NULL DEFAULT 0
            );",
        )
        .expect("create transcription_history table");
        conn
    }

    fn insert_entry(conn: &Connection, timestamp: i64, text: &str, post_processed: Option<&str>) {
        conn.execute(
            "INSERT INTO transcription_history (
                file_name,
                timestamp,
                saved,
                title,
                transcription_text,
                post_processed_text,
                post_process_prompt,
                post_process_requested
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                format!("handy-{}.wav", timestamp),
                timestamp,
                false,
                format!("Recording {}", timestamp),
                text,
                post_processed,
                Option::<String>::None,
                false,
            ],
        )
        .expect("insert history entry");
    }

    #[test]
    fn get_latest_entry_returns_none_when_empty() {
        let conn = setup_conn();
        let entry = HistoryManager::get_latest_entry_with_conn(&conn).expect("fetch latest entry");
        assert!(entry.is_none());
    }

    #[test]
    fn get_latest_entry_returns_newest_entry() {
        let conn = setup_conn();
        insert_entry(&conn, 100, "first", None);
        insert_entry(&conn, 200, "second", Some("processed"));

        let entry = HistoryManager::get_latest_entry_with_conn(&conn)
            .expect("fetch latest entry")
            .expect("entry exists");

        assert_eq!(entry.timestamp, 200);
        assert_eq!(entry.transcription_text, "second");
        assert_eq!(entry.post_processed_text.as_deref(), Some("processed"));
    }

    #[test]
    fn get_latest_completed_entry_skips_empty_entries() {
        let conn = setup_conn();
        insert_entry(&conn, 100, "completed", None);
        insert_entry(&conn, 200, "", None);

        let entry = HistoryManager::get_latest_completed_entry_with_conn(&conn)
            .expect("fetch latest completed entry")
            .expect("completed entry exists");

        assert_eq!(entry.timestamp, 100);
        assert_eq!(entry.transcription_text, "completed");
    }
}
