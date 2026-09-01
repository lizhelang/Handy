use std::fmt;
use std::path::{Path, PathBuf};

use inputia_core::{AppContext, AppPolicy, SqliteMemory};

const INPUTIA_MEMORY_DB_FILE: &str = "inputia_memory.db";
const HANDY_HISTORY_DB_FILE: &str = "history.db";
const HANDY_CLIPBOARD_DB_FILE: &str = "clipboard.db";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HandyDataPaths {
    pub app_data_dir: PathBuf,
    pub inputia_memory_db: PathBuf,
    pub history_db: PathBuf,
    pub clipboard_db: PathBuf,
}

impl HandyDataPaths {
    pub fn new(app_data_dir: impl Into<PathBuf>) -> Self {
        let app_data_dir = app_data_dir.into();
        Self {
            inputia_memory_db: app_data_dir.join(INPUTIA_MEMORY_DB_FILE),
            history_db: app_data_dir.join(HANDY_HISTORY_DB_FILE),
            clipboard_db: app_data_dir.join(HANDY_CLIPBOARD_DB_FILE),
            app_data_dir,
        }
    }

    pub fn with_memory_db(mut self, memory_db: impl Into<PathBuf>) -> Self {
        self.inputia_memory_db = memory_db.into();
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImportLimits {
    pub history: usize,
    pub clipboard: usize,
}

impl Default for ImportLimits {
    fn default() -> Self {
        Self {
            history: 2_000,
            clipboard: 2_000,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportSummary {
    pub history_imported: usize,
    pub clipboard_imported: usize,
    pub term_count: usize,
}

pub struct InputiaHandyRuntime {
    paths: HandyDataPaths,
    memory: SqliteMemory,
}

impl InputiaHandyRuntime {
    pub fn open(paths: HandyDataPaths, policy: AppPolicy) -> Result<Self> {
        ensure_parent_dir(&paths.inputia_memory_db)?;
        let memory = SqliteMemory::open(&paths.inputia_memory_db, policy)?;
        Ok(Self { paths, memory })
    }

    pub fn paths(&self) -> &HandyDataPaths {
        &self.paths
    }

    pub fn memory(&self) -> &SqliteMemory {
        &self.memory
    }

    pub fn memory_mut(&mut self) -> &mut SqliteMemory {
        &mut self.memory
    }

    pub fn import_existing_sources(
        &mut self,
        limits: ImportLimits,
        fallback_bundle_id: impl Into<String>,
    ) -> Result<ImportSummary> {
        let fallback_context = AppContext::new(fallback_bundle_id);
        let history_db = self.paths.history_db.clone();
        let clipboard_db = self.paths.clipboard_db.clone();
        let history_imported =
            self.import_history_if_present(&history_db, &fallback_context, limits.history)?;
        let clipboard_imported =
            self.import_clipboard_if_present(&clipboard_db, &fallback_context, limits.clipboard)?;
        let term_count = self.memory.term_count()?;

        Ok(ImportSummary {
            history_imported,
            clipboard_imported,
            term_count,
        })
    }

    fn import_history_if_present(
        &mut self,
        history_db: &Path,
        fallback_context: &AppContext,
        limit: usize,
    ) -> Result<usize> {
        if !history_db.exists() {
            return Ok(0);
        }

        Ok(self
            .memory
            .import_handy_history(history_db, fallback_context, limit)?)
    }

    fn import_clipboard_if_present(
        &mut self,
        clipboard_db: &Path,
        fallback_context: &AppContext,
        limit: usize,
    ) -> Result<usize> {
        if !clipboard_db.exists() {
            return Ok(0);
        }

        Ok(self
            .memory
            .import_handy_clipboard(clipboard_db, fallback_context, limit)?)
    }
}

fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}

pub type Result<T> = std::result::Result<T, RuntimeError>;

#[derive(Debug)]
pub enum RuntimeError {
    Io(std::io::Error),
    Sqlite(rusqlite::Error),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::Sqlite(error) => write!(formatter, "SQLite error: {error}"),
        }
    }
}

impl std::error::Error for RuntimeError {}

impl From<std::io::Error> for RuntimeError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<rusqlite::Error> for RuntimeError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};

    #[test]
    fn data_paths_match_handy_storage_contract() {
        let paths = HandyDataPaths::new("/tmp/handy-data");

        assert_eq!(paths.app_data_dir, PathBuf::from("/tmp/handy-data"));
        assert_eq!(
            paths.inputia_memory_db,
            PathBuf::from("/tmp/handy-data/inputia_memory.db")
        );
        assert_eq!(
            paths.history_db,
            PathBuf::from("/tmp/handy-data/history.db")
        );
        assert_eq!(
            paths.clipboard_db,
            PathBuf::from("/tmp/handy-data/clipboard.db")
        );
    }

    #[test]
    fn import_existing_sources_reads_handy_databases_without_mutating_them() {
        let temp = tempfile::tempdir().unwrap();
        let paths = HandyDataPaths::new(temp.path());
        create_history_db(&paths.history_db);
        create_clipboard_db(&paths.clipboard_db);

        let history_count_before = count_rows(&paths.history_db, "transcription_history");
        let clipboard_count_before = count_rows(&paths.clipboard_db, "clipboard_history");

        let mut runtime = InputiaHandyRuntime::open(paths.clone(), AppPolicy::default()).unwrap();
        let summary = runtime
            .import_existing_sources(ImportLimits::default(), "com.pais.handy")
            .unwrap();

        assert_eq!(summary.history_imported, 2);
        assert_eq!(summary.clipboard_imported, 1);
        assert_eq!(summary.term_count, 3);
        assert_eq!(
            count_rows(&paths.history_db, "transcription_history"),
            history_count_before
        );
        assert_eq!(
            count_rows(&paths.clipboard_db, "clipboard_history"),
            clipboard_count_before
        );
        assert!(paths.inputia_memory_db.exists());

        let zhongguo = runtime
            .memory()
            .completion_candidates("中国", 10)
            .unwrap()
            .into_iter()
            .map(|candidate| candidate.text)
            .collect::<Vec<_>>();
        assert!(zhongguo.iter().any(|text| text == "中国 科技"));

        let password = runtime.memory().completion_candidates("密码", 10).unwrap();
        assert!(password.is_empty());
    }

    #[test]
    fn import_existing_sources_tolerates_missing_handy_databases() {
        let temp = tempfile::tempdir().unwrap();
        let paths = HandyDataPaths::new(temp.path());
        let mut runtime = InputiaHandyRuntime::open(paths, AppPolicy::default()).unwrap();

        let summary = runtime
            .import_existing_sources(ImportLimits::default(), "com.pais.handy")
            .unwrap();

        assert_eq!(
            summary,
            ImportSummary {
                history_imported: 0,
                clipboard_imported: 0,
                term_count: 0,
            }
        );
    }

    fn create_history_db(path: &Path) {
        let conn = Connection::open(path).unwrap();
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
        .unwrap();
        conn.execute(
            "INSERT INTO transcription_history (
                file_name, timestamp, saved, title, transcription_text, post_processed_text
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                "voice-1.wav",
                1,
                false,
                "Voice 1",
                "中国 科技",
                Option::<String>::None
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO transcription_history (
                file_name, timestamp, saved, title, transcription_text, post_processed_text
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params!["voice-2.wav", 2, false, "Voice 2", "raw draft", "语音 热词"],
        )
        .unwrap();
    }

    fn create_clipboard_db(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE clipboard_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                content_type TEXT NOT NULL,
                content_preview TEXT NOT NULL,
                content_hash TEXT NOT NULL UNIQUE,
                full_text TEXT,
                image_path TEXT,
                source_app TEXT,
                is_favorite BOOLEAN NOT NULL DEFAULT 0,
                is_pinned BOOLEAN NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                size_bytes INTEGER NOT NULL,
                title TEXT
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO clipboard_history (
                content_type, content_preview, content_hash, full_text, source_app, created_at, size_bytes
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                "text",
                "剪贴板 常用语",
                "hash-1",
                "剪贴板 常用语",
                "com.apple.TextEdit",
                1,
                18
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO clipboard_history (
                content_type, content_preview, content_hash, full_text, source_app, created_at, size_bytes
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                "text",
                "密码 不应学习",
                "hash-2",
                "密码 不应学习",
                "com.1password.1password",
                2,
                18
            ],
        )
        .unwrap();
    }

    fn count_rows(path: &Path, table: &str) -> i64 {
        let conn = Connection::open(path).unwrap();
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    }
}
