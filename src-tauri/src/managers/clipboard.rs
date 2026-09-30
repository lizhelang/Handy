use crate::data_migration;
use anyhow::{anyhow, Result};
use chrono::{DateTime, Local, Utc};
#[cfg(not(target_os = "macos"))]
use clipboard_rs::common::RustImage;
use clipboard_rs::{Clipboard, ClipboardContext};
#[cfg(not(target_os = "macos"))]
use clipboard_rs::{ClipboardHandler, ClipboardWatcher, ClipboardWatcherContext};
use inputia_handy_runtime::{
    attachment_store::{AttachmentKind, PinPurpose},
    service::HistoryService,
    source::SourceTable,
};
use log::{debug, error, info};
use rusqlite::{params, Connection, OptionalExtension};
use rusqlite_migration::{Migrations, M};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use specta::Type;
use std::fs;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;
use tauri::image::Image;
use tauri::{AppHandle, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_specta::Event;

#[cfg(target_os = "macos")]
use tauri_nspanel::objc2::MainThreadMarker;

const TEXT_PREVIEW_MAX_CHARS: usize = 200;
const SEARCH_RESULT_LIMIT: i64 = 100;
const FILE_PREVIEW_MAX_PATHS: usize = 3;

// The monitor keeps ownership of a queued read until its closure/result is dropped.
#[cfg(any(target_os = "macos", test))]
struct ClipboardMonitorRead {
    in_flight: Arc<AtomicBool>,
    deadline: std::time::Instant,
}

#[cfg(any(target_os = "macos", test))]
impl ClipboardMonitorRead {
    fn acquire(in_flight: &Arc<AtomicBool>, deadline: std::time::Instant) -> Option<Self> {
        in_flight
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .ok()?;
        Some(Self {
            in_flight: in_flight.clone(),
            deadline,
        })
    }

    fn may_start(&self, now: std::time::Instant) -> bool {
        now < self.deadline
    }
}

#[cfg(any(target_os = "macos", test))]
impl Drop for ClipboardMonitorRead {
    fn drop(&mut self) {
        self.in_flight.store(false, Ordering::Release);
    }
}

#[cfg(any(target_os = "macos", test))]
fn read_changed_clipboard<T, E>(
    last_change_count: Option<isize>,
    change_count: isize,
    read: impl FnOnce() -> std::result::Result<T, E>,
) -> std::result::Result<Option<T>, E> {
    if last_change_count == Some(change_count) {
        return Ok(None);
    }
    read().map(Some)
}

/// Database migrations for clipboard history.
static MIGRATIONS: &[M] = &[
    M::up(
        "CREATE TABLE IF NOT EXISTS clipboard_history (
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
            size_bytes INTEGER NOT NULL
        );",
    ),
    M::up("ALTER TABLE clipboard_history ADD COLUMN title TEXT;"),
    M::up(
        "CREATE INDEX IF NOT EXISTS idx_clipboard_history_order
            ON clipboard_history(is_pinned DESC, created_at DESC);
         CREATE INDEX IF NOT EXISTS idx_clipboard_history_content_order
            ON clipboard_history(content_type, is_pinned DESC, created_at DESC);
         CREATE INDEX IF NOT EXISTS idx_clipboard_history_favorite_order
            ON clipboard_history(is_favorite, is_pinned DESC, created_at DESC);
         CREATE INDEX IF NOT EXISTS idx_clipboard_history_cleanup
            ON clipboard_history(is_pinned, is_favorite, created_at ASC);",
    ),
];

fn run_clipboard_migrations(conn: &mut Connection) -> Result<i32> {
    conn.execute_batch("PRAGMA journal_mode = WAL;")?;

    let migrations = Migrations::new(MIGRATIONS.to_vec());

    #[cfg(debug_assertions)]
    migrations.validate().expect("Invalid migrations");

    migrations.to_latest(conn)?;
    let version_after: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
    Ok(version_after)
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct ClipboardItem {
    pub id: i64,
    pub title: Option<String>,
    pub content_type: String,
    pub content_preview: String,
    pub content_hash: String,
    pub full_text: Option<String>,
    pub image_path: Option<String>,
    pub source_app: Option<String>,
    pub is_favorite: bool,
    pub is_pinned: bool,
    pub created_at: String,
    pub size_bytes: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct ClipboardStats {
    pub total_items: i64,
    pub favorites_count: i64,
    pub pinned_count: i64,
    pub total_size_bytes: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct ClipboardSettings {
    pub max_records: usize,
    pub hotkey: String,
    pub confirm_mode: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct ClipboardPageResult {
    pub items: Vec<ClipboardItem>,
    pub has_more: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, Type, tauri_specta::Event)]
#[serde(tag = "action")]
pub enum ClipboardUpdatePayload {
    #[serde(rename = "added")]
    Added { item: ClipboardItem },
    #[serde(rename = "updated")]
    Updated { item: ClipboardItem },
    #[serde(rename = "deleted")]
    Deleted { id: i64 },
    #[serde(rename = "deleted_many")]
    DeletedMany { ids: Vec<i64> },
    #[serde(rename = "cleared")]
    Cleared { keep_pinned: bool },
}

#[derive(Clone)]
pub struct ClipboardManager {
    app_handle: AppHandle,
    db_path: PathBuf,
    images_dir: PathBuf,
    monitoring_started: Arc<AtomicBool>,
    last_seen_hash: Arc<Mutex<Option<String>>>,
    #[cfg(target_os = "macos")]
    monitor_read_in_flight: Arc<AtomicBool>,
    #[cfg(target_os = "macos")]
    monitor_change_count: Arc<Mutex<Option<isize>>>,
}

#[cfg(target_os = "macos")]
enum ClipboardMonitorSnapshot {
    Files(Vec<String>),
    Image(Image<'static>),
    Text(String),
    Empty,
}

#[cfg(target_os = "macos")]
fn process_macos_clipboard_representations<F, I, T>(
    try_files: F,
    try_image: I,
    try_text: T,
) -> Result<()>
where
    F: FnOnce() -> Result<bool>,
    I: FnOnce() -> Result<bool>,
    T: FnOnce() -> Result<bool>,
{
    if try_files()? {
        return Ok(());
    }

    if try_image()? {
        return Ok(());
    }

    if try_text()? {
        return Ok(());
    }

    debug!("Clipboard change did not contain supported file, image, or text content");
    Ok(())
}

// Adapted from StudentWeis/ropy's MIT-licensed clipboard file utilities.
// See docs/third-party-notices.md.
fn hex_digit_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn decode_percent_encoded(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut index = 0;
    let mut decoded = Vec::with_capacity(bytes.len());

    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let high = hex_digit_value(bytes[index + 1]);
            let low = hex_digit_value(bytes[index + 2]);

            if let (Some(high), Some(low)) = (high, low) {
                decoded.push((high << 4) | low);
                index += 3;
                continue;
            }
        }

        decoded.push(bytes[index]);
        index += 1;
    }

    String::from_utf8_lossy(&decoded).into_owned()
}

fn normalize_clipboard_file_path(path: &str) -> Option<String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }

    let uri_path = trimmed
        .strip_prefix("file://localhost")
        .or_else(|| trimmed.strip_prefix("file://"));

    Some(match uri_path {
        Some(path) => decode_percent_encoded(path),
        None => trimmed.to_owned(),
    })
}

fn normalize_clipboard_file_paths(paths: &[String]) -> Vec<String> {
    paths
        .iter()
        .filter_map(|path| normalize_clipboard_file_path(path))
        .collect()
}

fn serialize_clipboard_file_paths(paths: &[String]) -> Result<String> {
    serde_json::to_string(&normalize_clipboard_file_paths(paths))
        .map_err(|e| anyhow!("Failed to serialize clipboard file paths: {}", e))
}

fn deserialize_clipboard_file_paths(content: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(content).map_or_else(
        |_| {
            content
                .lines()
                .filter_map(normalize_clipboard_file_path)
                .collect()
        },
        |paths| normalize_clipboard_file_paths(&paths),
    )
}

fn clipboard_file_preview(paths: &[String]) -> String {
    let normalized = normalize_clipboard_file_paths(paths);
    if normalized.is_empty() {
        return String::new();
    }

    normalized
        .into_iter()
        .take(FILE_PREVIEW_MAX_PATHS)
        .map(|path| {
            PathBuf::from(&path)
                .file_name()
                .and_then(|value| value.to_str())
                .map_or(path, ToString::to_string)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn restore_files_strict<T>(
    content: &str,
    write: impl FnOnce(Vec<String>) -> Result<T>,
) -> Result<T> {
    let paths = if content.trim_start().starts_with('[') {
        let raw: Vec<String> =
            serde_json::from_str(content).map_err(|_| anyhow!("Invalid file clipboard payload"))?;
        normalize_clipboard_file_paths(&raw)
    } else {
        deserialize_clipboard_file_paths(content)
    };
    if paths.is_empty()
        || paths.iter().any(|path| {
            !std::path::Path::new(path).is_absolute() || path.chars().any(char::is_control)
        })
    {
        return Err(anyhow!("Clipboard file paths are unavailable or invalid"));
    }
    write(paths)
}

pub(crate) enum PreparedClipboardCopy {
    Text(String),
    Image(Image<'static>),
    Files(Vec<String>),
}

pub(crate) enum ClipboardWriteOutcome {
    NotWritten,
    Unknown,
    Written,
}

impl ClipboardManager {
    /// 只有显式纯文本动作才能把文件列表转换为完整路径；图片不伪装成文本。
    pub(crate) fn prepare_unified_plain_text(
        &self,
        content_type: &str,
        text: Option<String>,
    ) -> Result<PreparedClipboardCopy> {
        let text = text.ok_or_else(|| anyhow!("Text unavailable"))?;
        match content_type {
            "text" => Ok(PreparedClipboardCopy::Text(text)),
            "files" => restore_files_strict(&text, |paths| {
                Ok(PreparedClipboardCopy::Text(paths.join("\n")))
            }),
            _ => Err(anyhow!("Plain text representation unavailable")),
        }
    }

    /// 慢解析、文件路径核验和图像解码在取得短期输出许可之前完成。
    pub(crate) fn prepare_unified_copy(
        &self,
        content_type: &str,
        text: Option<String>,
        image_path: Option<String>,
    ) -> Result<PreparedClipboardCopy> {
        match content_type {
            "text" => Ok(PreparedClipboardCopy::Text(
                text.ok_or_else(|| anyhow!("Text unavailable"))?,
            )),
            "files" => restore_files_strict(
                &text.ok_or_else(|| anyhow!("Files unavailable"))?,
                |paths| Ok(PreparedClipboardCopy::Files(paths)),
            ),
            "image" => Ok(PreparedClipboardCopy::Image(self.read_clipboard_image(
                &image_path.ok_or_else(|| anyhow!("Image unavailable"))?,
            )?)),
            _ => Err(anyhow!("Unsupported clipboard representation")),
        }
    }

    /// 由带截止门控的主线程任务调用；准备结束后紧邻真正写板检查撤销许可。
    pub(crate) fn publish_unified_copy(
        &self,
        prepared: PreparedClipboardCopy,
        expected_change_count: Option<isize>,
        validate: impl FnOnce() -> std::result::Result<(), String>,
    ) -> ClipboardWriteOutcome {
        #[cfg(target_os = "macos")]
        if MainThreadMarker::new().is_none() {
            return ClipboardWriteOutcome::NotWritten;
        }
        #[cfg(target_os = "macos")]
        {
            let Some(change_count) = expected_change_count else {
                return ClipboardWriteOutcome::NotWritten;
            };
            let items = match prepared {
                PreparedClipboardCopy::Text(text) => {
                    vec![vec![("public.utf8-plain-text".into(), text.into_bytes())]]
                }
                PreparedClipboardCopy::Files(paths) => {
                    let mut items = Vec::new();
                    for path in paths {
                        let url = objc2_foundation::NSURL::fileURLWithPath(
                            &objc2_foundation::NSString::from_str(&path),
                        );
                        let Some(url) = url.absoluteString() else {
                            return ClipboardWriteOutcome::NotWritten;
                        };
                        items.push(vec![(
                            "public.file-url".into(),
                            url.to_string().into_bytes(),
                        )]);
                    }
                    items
                }
                PreparedClipboardCopy::Image(image) => {
                    let Ok(bytes) = Self::encode_tauri_image_png(&image) else {
                        return ClipboardWriteOutcome::NotWritten;
                    };
                    vec![vec![("public.png".into(), bytes)]]
                }
            };
            match crate::paste_tx::publish_snapshot(
                crate::paste_tx::ClipboardSnapshot {
                    change_count,
                    items,
                },
                validate,
            ) {
                crate::paste_tx::HistoryPasteOutcome::NotDispatched(_) => {
                    ClipboardWriteOutcome::NotWritten
                }
                crate::paste_tx::HistoryPasteOutcome::PossiblyDispatched(_) => {
                    ClipboardWriteOutcome::Unknown
                }
                crate::paste_tx::HistoryPasteOutcome::Dispatched => ClipboardWriteOutcome::Written,
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = expected_change_count;
            let result = match prepared {
                PreparedClipboardCopy::Files(paths) => {
                    let Ok(clipboard) = ClipboardContext::new() else {
                        return ClipboardWriteOutcome::NotWritten;
                    };
                    if validate().is_err() {
                        return ClipboardWriteOutcome::NotWritten;
                    }
                    clipboard.set_files(paths).map_err(|_| ())
                }
                PreparedClipboardCopy::Text(text) => {
                    if validate().is_err() {
                        return ClipboardWriteOutcome::NotWritten;
                    }
                    self.app_handle.clipboard().write_text(text).map_err(|_| ())
                }
                PreparedClipboardCopy::Image(image) => {
                    if validate().is_err() {
                        return ClipboardWriteOutcome::NotWritten;
                    }
                    self.app_handle
                        .clipboard()
                        .write_image(&image)
                        .map_err(|_| ())
                }
            };
            if result.is_ok() {
                ClipboardWriteOutcome::Written
            } else {
                ClipboardWriteOutcome::Unknown
            }
        }
    }
    fn client_image_path(&self, path: &str) -> String {
        self.images_dir.join(path).to_string_lossy().into_owned()
    }

    fn normalize_item_for_client(&self, mut item: ClipboardItem) -> ClipboardItem {
        if let Some(path) = item.image_path.as_deref() {
            item.image_path = Some(self.client_image_path(path));
        }

        item
    }

    pub fn new(app_handle: &AppHandle) -> Result<Self> {
        let app_data_dir = crate::portable::app_data_dir(app_handle)?;
        let db_path = app_data_dir.join("clipboard.db");
        let images_dir = app_data_dir.join("clipboard_images");

        // Ensure images directory exists
        if !images_dir.exists() {
            fs::create_dir_all(&images_dir)?;
            debug!("Created clipboard images directory: {:?}", images_dir);
        }

        let manager = Self {
            app_handle: app_handle.clone(),
            db_path,
            images_dir,
            monitoring_started: Arc::new(AtomicBool::new(false)),
            last_seen_hash: Arc::new(Mutex::new(None)),
            #[cfg(target_os = "macos")]
            monitor_read_in_flight: Arc::new(AtomicBool::new(false)),
            #[cfg(target_os = "macos")]
            monitor_change_count: Arc::new(Mutex::new(None)),
        };

        // Initialize database
        manager.init_database()?;

        Ok(manager)
    }

    fn init_database(&self) -> Result<()> {
        info!("Initializing clipboard database at {:?}", self.db_path);

        let db_existed = self.db_path.exists();
        let version_before = {
            let conn = self.get_connection()?;
            conn.pragma_query_value(None, "user_version", |row| row.get(0))?
        };
        let target_version = MIGRATIONS.len() as i32;
        debug!(
            "Clipboard database version before migration: {}",
            version_before
        );

        let version_after = if db_existed && version_before < target_version {
            let backup_root = self
                .db_path
                .parent()
                .ok_or_else(|| anyhow!("Clipboard database path has no parent"))?
                .join("migration_backups")
                .join("clipboard");
            let mut migrated_to = version_before;
            data_migration::run_sqlite_migration_with_backup(
                &self.db_path,
                &backup_root,
                |conn| {
                    migrated_to = run_clipboard_migrations(conn)?;
                    Ok(())
                },
            )?;
            migrated_to
        } else {
            let mut conn = self.get_connection()?;
            run_clipboard_migrations(&mut conn)?
        };

        if version_after > version_before {
            info!(
                "Clipboard database migrated from version {} to {}",
                version_before, version_after
            );
        } else {
            debug!(
                "Clipboard database already at latest version {}",
                version_after
            );
        }

        Ok(())
    }

    fn get_connection(&self) -> Result<Connection> {
        let conn = Connection::open(&self.db_path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute_batch(
            "PRAGMA synchronous = NORMAL;
             PRAGMA temp_store = MEMORY;",
        )?;
        Ok(conn)
    }

    fn map_clipboard_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<ClipboardItem> {
        let created_at: i64 = row.get("created_at")?;
        let datetime = DateTime::from_timestamp(created_at, 0)
            .unwrap_or_default()
            .with_timezone(&Local);

        Ok(ClipboardItem {
            id: row.get("id")?,
            title: row.get("title")?,
            content_type: row.get("content_type")?,
            content_preview: row.get("content_preview")?,
            content_hash: row.get("content_hash")?,
            full_text: row.get("full_text")?,
            image_path: row.get("image_path")?,
            source_app: row.get("source_app")?,
            is_favorite: row.get("is_favorite")?,
            is_pinned: row.get("is_pinned")?,
            created_at: datetime.to_rfc3339(),
            size_bytes: row.get("size_bytes")?,
        })
    }

    /// Compute SHA-256 hash of content
    fn compute_hash(content: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(content);
        format!("{:x}", hasher.finalize())
    }

    fn compute_file_hash(serialized_paths: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"file\0");
        hasher.update(serialized_paths.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    fn text_preview(text: &str) -> String {
        let mut chars = text.chars();
        let preview: String = chars.by_ref().take(TEXT_PREVIEW_MAX_CHARS).collect();

        if chars.next().is_some() {
            format!("{}...", preview)
        } else {
            preview
        }
    }

    /// Start monitoring clipboard changes
    pub fn start_monitoring(&self) {
        if self.monitoring_started.swap(true, Ordering::SeqCst) {
            debug!("Clipboard monitoring already active");
            return;
        }

        let manager = self.clone();

        std::thread::spawn(move || {
            info!("Starting clipboard monitoring thread");

            loop {
                let result = catch_unwind(AssertUnwindSafe(|| manager.run_monitoring_backend()));

                match result {
                    Ok(()) => {
                        error!("Clipboard monitoring backend exited unexpectedly; restarting")
                    }
                    Err(_) => error!("Clipboard monitoring backend panicked; restarting"),
                }

                std::thread::sleep(Duration::from_secs(1));
            }
        });
    }

    fn run_monitoring_backend(&self) {
        self.sync_current_clipboard_for_monitor("capture initial clipboard state");

        #[cfg(target_os = "macos")]
        self.run_polling_monitor();

        #[cfg(not(target_os = "macos"))]
        self.run_watcher_monitor_loop();
    }

    #[cfg(target_os = "macos")]
    fn run_polling_monitor(&self) {
        info!("Using main-thread clipboard polling monitor on macOS");

        loop {
            if self.monitoring_enabled() {
                self.sync_current_clipboard_for_monitor("poll clipboard state");
            } else if let Ok(mut count) = self.monitor_change_count.lock() {
                // Re-enabling capture should inspect the current clipboard once.
                *count = None;
            }

            std::thread::sleep(Duration::from_millis(750));
        }
    }

    #[cfg(target_os = "macos")]
    fn run_on_main_thread_sync<R, F>(&self, operation: &str, action: F) -> Option<R>
    where
        R: Send + 'static,
        F: FnOnce(ClipboardManager) -> R + Send + 'static,
    {
        if MainThreadMarker::new().is_some() {
            return match catch_unwind(AssertUnwindSafe(|| action(self.clone()))) {
                Ok(result) => Some(result),
                Err(_) => {
                    error!(
                        "macOS clipboard monitor panicked while trying to {}",
                        operation
                    );
                    None
                }
            };
        }

        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let manager = self.clone();
        let operation_name = operation.to_string();

        if let Err(err) = self.app_handle.run_on_main_thread(move || {
            let result = catch_unwind(AssertUnwindSafe(|| action(manager)));
            let _ = sender.send(result);
        }) {
            error!(
                "Failed to schedule macOS clipboard monitor {} on main thread: {}",
                operation, err
            );
            return None;
        }

        match receiver.recv_timeout(Duration::from_secs(2)) {
            Ok(Ok(result)) => Some(result),
            Ok(Err(_)) => {
                error!(
                    "macOS clipboard monitor panicked while trying to {}",
                    operation_name
                );
                None
            }
            Err(err) => {
                error!(
                    "Timed out waiting for macOS clipboard monitor to {}: {}",
                    operation_name, err
                );
                None
            }
        }
    }

    #[cfg(target_os = "macos")]
    fn sync_current_clipboard_for_monitor(&self, context: &str) {
        if !self.monitoring_enabled() {
            return;
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let Some(permit) = ClipboardMonitorRead::acquire(&self.monitor_read_in_flight, deadline)
        else {
            return;
        };
        let last_change_count = self
            .monitor_change_count
            .lock()
            .ok()
            .and_then(|count| *count);
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let manager = self.clone();
        if let Err(err) = self.app_handle.run_on_main_thread(move || {
            // A receiver timeout cannot cancel a queued AppKit closure. Keep the permit
            // in this closure and skip expired work before touching the pasteboard.
            if !permit.may_start(std::time::Instant::now()) {
                return;
            }
            let result = catch_unwind(AssertUnwindSafe(|| {
                manager.read_monitor_clipboard_snapshot(last_change_count)
            }));
            // The worker owns the permit while encoding/writing. If it timed out,
            // the failed send drops it only after this main-thread read has ended.
            let _ = sender.send((permit, result));
        }) {
            error!("Failed to schedule clipboard monitor {}: {}", context, err);
            return;
        }
        match receiver.recv_timeout(Duration::from_secs(2)) {
            Ok((_permit, Ok(Ok(Some((change_count, snapshot)))))) => {
                if !self.monitoring_enabled() {
                    return;
                }
                let result = match snapshot {
                    ClipboardMonitorSnapshot::Files(paths) => {
                        self.process_file_paths_change(&paths)
                    }
                    ClipboardMonitorSnapshot::Image(image) => {
                        self.process_tauri_image_change(&image)
                    }
                    ClipboardMonitorSnapshot::Text(text) => self.process_tauri_text_change(&text),
                    ClipboardMonitorSnapshot::Empty => Ok(()),
                };
                match result {
                    Ok(()) => {
                        if let Ok(mut count) = self.monitor_change_count.lock() {
                            *count = Some(change_count);
                        }
                    }
                    Err(err) => error!("Failed to process clipboard monitor {}: {}", context, err),
                }
            }
            Ok((_permit, Ok(Ok(None)))) => {}
            Ok((_permit, Ok(Err(err)))) => {
                error!("Failed to read clipboard monitor {}: {}", context, err)
            }
            Ok((_permit, Err(_))) => error!("Clipboard monitor panicked during {}", context),
            Err(err) => debug!(
                "Clipboard monitor {} did not finish in time: {}",
                context, err
            ),
        }
    }

    #[cfg(target_os = "macos")]
    fn read_monitor_clipboard_snapshot(
        &self,
        last_change_count: Option<isize>,
    ) -> Result<Option<(isize, ClipboardMonitorSnapshot)>> {
        if MainThreadMarker::new().is_none() {
            return Err(anyhow!(
                "Refusing to read macOS clipboard off the main thread"
            ));
        }
        let pasteboard = objc2_app_kit::NSPasteboard::generalPasteboard();
        let change_count = pasteboard.changeCount();
        let snapshot = read_changed_clipboard(
            last_change_count,
            change_count,
            || -> Result<ClipboardMonitorSnapshot> {
                let files = ClipboardContext::new()
                    .map_err(|e| anyhow!("Failed to access clipboard files: {}", e))?
                    .get_files()
                    .unwrap_or_default();
                if !files.is_empty() {
                    return Ok(ClipboardMonitorSnapshot::Files(files));
                }
                let clipboard = self.app_handle.clipboard();
                if let Ok(image) = clipboard.read_image() {
                    return Ok(ClipboardMonitorSnapshot::Image(image.to_owned()));
                }
                if let Ok(text) = clipboard.read_text() {
                    if !text.is_empty() {
                        return Ok(ClipboardMonitorSnapshot::Text(text));
                    }
                }
                Ok(ClipboardMonitorSnapshot::Empty)
            },
        )?;
        // A producer may change the board during a promised-data read. Retry later
        // rather than marking a mixed snapshot as the current generation.
        if pasteboard.changeCount() != change_count {
            return Ok(None);
        }
        Ok(snapshot.map(|snapshot| (change_count, snapshot)))
    }

    #[cfg(not(target_os = "macos"))]
    fn sync_current_clipboard_for_monitor(&self, context: &str) {
        match catch_unwind(AssertUnwindSafe(|| self.sync_current_clipboard())) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => error!("Failed to {}: {}", context, e),
            Err(_) => error!("Clipboard monitor panicked while trying to {}", context),
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn run_watcher_monitor_loop(&self) {
        loop {
            self.run_watcher_monitor();
            error!("Clipboard watcher stopped; restarting in 1 second");
            std::thread::sleep(Duration::from_secs(1));
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn run_watcher_monitor(&self) {
        let ctx = match ClipboardContext::new() {
            Ok(ctx) => ctx,
            Err(e) => {
                error!("Failed to create clipboard context: {}", e);
                return;
            }
        };

        let mut watcher = match ClipboardWatcherContext::new() {
            Ok(watcher) => watcher,
            Err(e) => {
                error!("Failed to create clipboard watcher: {}", e);
                return;
            }
        };

        let handler = ClipboardChangeHandler {
            manager: self.clone(),
            clipboard: ctx,
        };

        watcher.add_handler(handler);
        watcher.start_watch(); // Blocking call
    }

    fn emit_deleted_many(&self, ids: Vec<i64>) {
        if ids.is_empty() {
            return;
        }

        if let Err(e) = (ClipboardUpdatePayload::DeletedMany { ids }).emit(&self.app_handle) {
            error!("Failed to emit clipboard-deleted-many event: {}", e);
        }
    }

    pub fn sync_current_clipboard(&self) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            self.run_on_main_thread_sync("sync current clipboard", |manager| {
                manager.process_tauri_clipboard_change_on_main_thread()
            })
            .ok_or_else(|| anyhow!("Failed to sync clipboard on the macOS main thread"))?
        }

        #[cfg(not(target_os = "macos"))]
        {
            let mut clipboard = ClipboardContext::new()
                .map_err(|e| anyhow!("Failed to access clipboard: {}", e))?;
            self.process_clipboard_change(&mut clipboard)
        }
    }

    fn monitoring_enabled(&self) -> bool {
        let settings = crate::settings::get_settings(&self.app_handle);
        settings.clipboard_enabled
    }

    fn process_text_change(&self, text: &str) -> Result<()> {
        self.add_text(text).map(|_| ())
    }

    fn process_file_paths_change(&self, file_paths: &[String]) -> Result<()> {
        let serialized_paths = serialize_clipboard_file_paths(file_paths)?;
        let hash = Self::compute_file_hash(&serialized_paths);

        #[cfg(target_os = "macos")]
        if self.is_last_seen_hash(&hash) {
            return Ok(());
        }

        let result = self.add_files(file_paths).map(|_| ());

        #[cfg(target_os = "macos")]
        if result.is_ok() {
            self.remember_last_seen_hash(hash);
        }

        result
    }

    #[cfg(target_os = "macos")]
    fn is_last_seen_hash(&self, hash: &str) -> bool {
        self.last_seen_hash
            .lock()
            .map(|last_seen| last_seen.as_deref() == Some(hash))
            .unwrap_or(false)
    }

    #[cfg(target_os = "macos")]
    fn remember_last_seen_hash(&self, hash: String) {
        if let Ok(mut last_seen) = self.last_seen_hash.lock() {
            *last_seen = Some(hash);
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn process_image_change(&self, image_data: &clipboard_rs::RustImageData) -> Result<()> {
        self.add_image(image_data).map(|_| ())
    }

    #[cfg(target_os = "macos")]
    fn process_tauri_image_change(&self, image: &Image<'_>) -> Result<()> {
        let png_bytes = Self::encode_tauri_image_png(image)?;
        let hash = Self::compute_hash(&png_bytes);

        if self.is_last_seen_hash(&hash) {
            return Ok(());
        }

        let result = self
            .add_image_png(hash.clone(), image.width(), image.height(), &png_bytes)
            .map(|_| ());

        if result.is_ok() {
            self.remember_last_seen_hash(hash);
        }

        result
    }

    #[cfg(target_os = "macos")]
    fn process_tauri_text_change(&self, text: &str) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }

        let hash = Self::compute_hash(text.as_bytes());
        if self.is_last_seen_hash(&hash) {
            return Ok(());
        }

        let result = self.process_text_change(text);
        if result.is_ok() {
            self.remember_last_seen_hash(hash);
        }

        result
    }

    #[cfg(target_os = "macos")]
    fn process_tauri_clipboard_change_on_main_thread(&self) -> Result<()> {
        if MainThreadMarker::new().is_none() {
            return Err(anyhow!(
                "Refusing to read macOS clipboard off the main thread"
            ));
        }

        if !self.monitoring_enabled() {
            return Ok(());
        }

        let clipboard = self.app_handle.clipboard();

        process_macos_clipboard_representations(
            || {
                let file_paths = self.read_file_paths_from_system_clipboard()?;
                if file_paths.is_empty() {
                    return Ok(false);
                }

                self.process_file_paths_change(&file_paths)?;
                Ok(true)
            },
            || match clipboard.read_image() {
                Ok(image) => {
                    self.process_tauri_image_change(&image)?;
                    Ok(true)
                }
                Err(_) => Ok(false),
            },
            || match clipboard.read_text() {
                Ok(text) if !text.is_empty() => {
                    self.process_tauri_text_change(&text)?;
                    Ok(true)
                }
                Ok(_) | Err(_) => Ok(false),
            },
        )
    }

    #[cfg(not(target_os = "macos"))]
    fn process_clipboard_change(&self, clipboard: &mut ClipboardContext) -> Result<()> {
        if !self.monitoring_enabled() {
            return Ok(());
        }

        if let Ok(file_paths) = clipboard.get_files() {
            let file_paths = normalize_clipboard_file_paths(&file_paths);
            if !file_paths.is_empty() {
                return self.process_file_paths_change(&file_paths);
            }
        }

        if let Ok(text) = clipboard.get_text() {
            if !text.is_empty() {
                return self.process_text_change(&text);
            }
        }

        if let Ok(image_data) = clipboard.get_image() {
            if !image_data.is_empty() {
                return self.process_image_change(&image_data);
            }
        }

        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn encode_tauri_image_png(image: &Image<'_>) -> Result<Vec<u8>> {
        let mut png_bytes = Vec::new();

        {
            let mut encoder = png::Encoder::new(&mut png_bytes, image.width(), image.height());
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder
                .write_header()
                .map_err(|e| anyhow!("Failed to create PNG header: {}", e))?;
            writer
                .write_image_data(image.rgba())
                .map_err(|e| anyhow!("Failed to encode PNG image: {}", e))?;
        }

        Ok(png_bytes)
    }

    fn refresh_existing_item(&self, hash: &str) -> Result<Option<ClipboardItem>> {
        let _source_write = super::integration::begin_source_write(&self.app_handle);
        let now = Utc::now().timestamp();
        let conn = self.get_connection()?;
        let changed = conn.execute(
            "UPDATE clipboard_history SET created_at = ?1 WHERE content_hash = ?2",
            params![now, hash],
        )?;

        if changed == 0 {
            return Ok(None);
        }

        let item = conn
            .query_row(
                "SELECT id, title, content_type, content_preview, content_hash, full_text, image_path,
                    source_app, is_favorite, is_pinned, created_at, size_bytes
                 FROM clipboard_history WHERE content_hash = ?1",
                params![hash],
                Self::map_clipboard_item,
            )
            .optional()?;

        if let Some(item) = item {
            info!("Refreshed existing clipboard entry with id {}", item.id);
            let item = self.normalize_item_for_client(item);

            if let Err(e) =
                (ClipboardUpdatePayload::Added { item: item.clone() }).emit(&self.app_handle)
            {
                error!("Failed to emit clipboard-refreshed event: {}", e);
            }

            return Ok(Some(item));
        }

        Ok(None)
    }

    fn get_item_by_id(&self, id: i64) -> Result<Option<ClipboardItem>> {
        let conn = self.get_connection()?;
        let item = conn
            .query_row(
                "SELECT id, title, content_type, content_preview, content_hash, full_text, image_path,
                    source_app, is_favorite, is_pinned, created_at, size_bytes
                 FROM clipboard_history WHERE id = ?1",
                params![id],
                Self::map_clipboard_item,
            )
            .optional()?;

        Ok(item.map(|item| self.normalize_item_for_client(item)))
    }

    fn attachment_service(&self) -> Result<Arc<HistoryService>> {
        self.app_handle
            .try_state::<Arc<super::integration::IntegrationManager>>()
            .map(|manager| manager.service.clone())
            .ok_or_else(|| anyhow!("Unified attachment service unavailable"))
    }
    fn import_image_bytes(
        &self,
        bytes: &[u8],
    ) -> Result<inputia_handy_runtime::service::PendingAttachmentImport> {
        let operation = format!(
            "image-{}",
            inputia_handy_runtime::attachment_store::new_operation_id()?
        );
        self.attachment_service()?
            .import_attachment_owned(AttachmentKind::Image, operation, bytes.to_vec())
            .map_err(|reason| anyhow!("Image attachment unavailable: {reason:?}"))
    }
    fn read_clipboard_image(&self, image_path: &str) -> Result<Image<'static>> {
        let raw = std::path::Path::new(image_path);
        let relative = if raw.is_absolute() {
            raw.strip_prefix(&self.images_dir)
                .map_err(|_| anyhow!("Image outside managed root"))?
        } else {
            raw
        };
        if relative.components().count() != 1
            || !matches!(
                relative.components().next(),
                Some(std::path::Component::Normal(_))
            )
        {
            return Err(anyhow!("Invalid image attachment name"));
        }
        let name = relative.to_string_lossy().into_owned();
        let id:i64=self.get_connection()?.query_row("SELECT id FROM clipboard_history WHERE image_path=?1 OR image_path=?2 ORDER BY id DESC LIMIT 1",params![name,image_path],|row|row.get(0))?;
        let service = self.attachment_service()?;
        let operation = format!(
            "image-read-{}",
            inputia_handy_runtime::attachment_store::new_operation_id()?
        );
        let result = (|| {
            let (lease, _, _) = service
                .acquire_source_attachment(
                    SourceTable::Clipboard,
                    id.to_string(),
                    None,
                    PinPurpose::Active,
                    operation.clone(),
                )
                .map_err(anyhow::Error::msg)?;
            let bytes = service.read_attachment(lease.clone());
            let released = service.release_attachment(lease);
            let bytes = bytes.map_err(anyhow::Error::msg)?;
            released.map_err(anyhow::Error::msg)?;
            Image::from_bytes(&bytes).map_err(Into::into)
        })();
        if result.is_err() {
            let _ = service.cancel_attachment_acquire(operation);
        }
        result
    }

    #[cfg(target_os = "macos")]
    fn read_file_paths_from_system_clipboard(&self) -> Result<Vec<String>> {
        self.run_on_main_thread_sync("read files from system clipboard", |_manager| {
            let clipboard = ClipboardContext::new()
                .map_err(|e| anyhow!("Failed to access clipboard files: {}", e))?;
            let file_paths = clipboard.get_files().unwrap_or_default();
            Ok(normalize_clipboard_file_paths(&file_paths))
        })
        .ok_or_else(|| anyhow!("Failed to read clipboard files on the macOS main thread"))?
    }

    fn write_text_to_system_clipboard(&self, text: String) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            self.run_on_main_thread_sync("write text to system clipboard", move |manager| {
                manager
                    .app_handle
                    .clipboard()
                    .write_text(text)
                    .map_err(|e| anyhow!("Failed to write text to system clipboard: {}", e))
            })
            .ok_or_else(|| {
                anyhow!("Failed to write text to system clipboard on the macOS main thread")
            })?
        }

        #[cfg(not(target_os = "macos"))]
        {
            self.app_handle
                .clipboard()
                .write_text(text)
                .map_err(|e| anyhow!("Failed to write text to system clipboard: {}", e))
        }
    }

    fn write_image_to_system_clipboard(&self, image: Image<'static>) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            self.run_on_main_thread_sync("write image to system clipboard", move |manager| {
                manager
                    .app_handle
                    .clipboard()
                    .write_image(&image)
                    .map_err(|e| anyhow!("Failed to write image to system clipboard: {}", e))
            })
            .ok_or_else(|| {
                anyhow!("Failed to write image to system clipboard on the macOS main thread")
            })?
        }

        #[cfg(not(target_os = "macos"))]
        {
            self.app_handle
                .clipboard()
                .write_image(&image)
                .map_err(|e| anyhow!("Failed to write image to system clipboard: {}", e))
        }
    }

    fn write_file_paths_to_system_clipboard(&self, file_paths: Vec<String>) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            self.run_on_main_thread_sync("write files to system clipboard", move |_manager| {
                let clipboard = ClipboardContext::new()
                    .map_err(|e| anyhow!("Failed to access clipboard files: {}", e))?;
                clipboard
                    .set_files(file_paths)
                    .map_err(|e| anyhow!("Failed to write clipboard files: {}", e))
            })
            .ok_or_else(|| anyhow!("Failed to write clipboard files on the macOS main thread"))?
        }

        #[cfg(not(target_os = "macos"))]
        {
            let clipboard = ClipboardContext::new()
                .map_err(|e| anyhow!("Failed to access clipboard files: {}", e))?;
            clipboard
                .set_files(file_paths)
                .map_err(|e| anyhow!("Failed to write clipboard files: {}", e))
        }
    }

    fn write_stored_file_content_to_system_clipboard(&self, content: String) -> Result<()> {
        let file_paths = deserialize_clipboard_file_paths(&content);
        if !file_paths.is_empty() {
            match self.write_file_paths_to_system_clipboard(file_paths) {
                Ok(()) => return Ok(()),
                Err(error) => debug!(
                    "Failed to restore stored file paths as native clipboard files; falling back to text: {}",
                    error
                ),
            }
        }

        // Historical file records may predate the JSON path-array format. Keep
        // their raw payload retrievable even when native file restoration is not
        // possible.
        self.write_text_to_system_clipboard(content)
    }

    /// Copy an already-loaded clipboard payload without re-querying history.
    pub fn copy_content_to_clipboard(
        &self,
        content_type: &str,
        text: Option<String>,
        image_path: Option<String>,
    ) -> Result<()> {
        match content_type {
            "text" | "richtext" => {
                let text = text.ok_or_else(|| anyhow!("Text content not found"))?;
                self.write_text_to_system_clipboard(text)
            }
            "file" => {
                let text = text.ok_or_else(|| anyhow!("File content not found"))?;
                self.write_stored_file_content_to_system_clipboard(text)
            }
            "image" => {
                let path = image_path.ok_or_else(|| anyhow!("Image path not found"))?;
                self.write_image_to_system_clipboard(self.read_clipboard_image(&path)?)
            }
            _ => Err(anyhow!("Unsupported content type")),
        }
    }

    /// Add a text entry to clipboard history
    pub fn add_text(&self, text: &str) -> Result<Option<ClipboardItem>> {
        let _source_write = super::integration::begin_source_write(&self.app_handle);
        let hash = Self::compute_hash(text.as_bytes());

        // Check if hash already exists
        {
            let conn = self.get_connection()?;
            let exists: bool = conn.query_row(
                "SELECT COUNT(*) > 0 FROM clipboard_history WHERE content_hash = ?1",
                params![hash],
                |row| row.get(0),
            )?;

            if exists {
                debug!("Clipboard content already exists (hash: {})", hash);
                return self.refresh_existing_item(&hash);
            }
        }

        let preview = Self::text_preview(text);

        let size_bytes = text.len() as i64;
        let now = Utc::now().timestamp();

        let conn = self.get_connection()?;
        conn.execute(
            "INSERT INTO clipboard_history (
                content_type, content_preview, content_hash, full_text,
                created_at, size_bytes
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params!["text", preview, hash, text, now, size_bytes],
        )?;

        let item = ClipboardItem {
            id: conn.last_insert_rowid(),
            title: None,
            content_type: "text".to_string(),
            content_preview: preview,
            content_hash: hash,
            full_text: Some(text.to_string()),
            image_path: None,
            source_app: None,
            is_favorite: false,
            is_pinned: false,
            created_at: DateTime::from_timestamp(now, 0)
                .unwrap_or_default()
                .with_timezone(&Local)
                .to_rfc3339(),
            size_bytes,
        };

        info!("Added clipboard text entry with id {}", item.id);

        // Emit event
        if let Err(e) =
            (ClipboardUpdatePayload::Added { item: item.clone() }).emit(&self.app_handle)
        {
            error!("Failed to emit clipboard-added event: {}", e);
        }

        match self.cleanup_old_entries() {
            Ok(deleted_ids) => self.emit_deleted_many(deleted_ids),
            Err(error) => error!("Clipboard item saved; retention cleanup pending: {error}"),
        }

        Ok(Some(item))
    }

    /// Add a file-path entry to clipboard history.
    pub fn add_files(&self, file_paths: &[String]) -> Result<Option<ClipboardItem>> {
        let _source_write = super::integration::begin_source_write(&self.app_handle);
        let normalized_paths = normalize_clipboard_file_paths(file_paths);
        if normalized_paths.is_empty() {
            return Ok(None);
        }

        let serialized_paths = serialize_clipboard_file_paths(&normalized_paths)?;
        let hash = Self::compute_file_hash(&serialized_paths);

        {
            let conn = self.get_connection()?;
            let exists: bool = conn.query_row(
                "SELECT COUNT(*) > 0 FROM clipboard_history WHERE content_hash = ?1",
                params![hash],
                |row| row.get(0),
            )?;

            if exists {
                debug!("Clipboard file list already exists (hash: {})", hash);
                return self.refresh_existing_item(&hash);
            }
        }

        let preview = clipboard_file_preview(&normalized_paths);
        let size_bytes = serialized_paths.len() as i64;
        let now = Utc::now().timestamp();

        let conn = self.get_connection()?;
        conn.execute(
            "INSERT INTO clipboard_history (
                content_type, content_preview, content_hash, full_text,
                created_at, size_bytes
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params!["file", preview, hash, serialized_paths, now, size_bytes],
        )?;

        let item = ClipboardItem {
            id: conn.last_insert_rowid(),
            title: None,
            content_type: "file".to_string(),
            content_preview: preview,
            content_hash: hash,
            full_text: Some(serialized_paths),
            image_path: None,
            source_app: None,
            is_favorite: false,
            is_pinned: false,
            created_at: DateTime::from_timestamp(now, 0)
                .unwrap_or_default()
                .with_timezone(&Local)
                .to_rfc3339(),
            size_bytes,
        };

        info!("Added clipboard file entry with id {}", item.id);

        if let Err(e) =
            (ClipboardUpdatePayload::Added { item: item.clone() }).emit(&self.app_handle)
        {
            error!("Failed to emit clipboard-added event: {}", e);
        }

        match self.cleanup_old_entries() {
            Ok(deleted_ids) => self.emit_deleted_many(deleted_ids),
            Err(error) => error!("Clipboard item saved; retention cleanup pending: {error}"),
        }

        Ok(Some(item))
    }

    #[cfg(not(target_os = "macos"))]
    /// Add an image entry to clipboard history
    pub fn add_image(
        &self,
        image_data: &clipboard_rs::RustImageData,
    ) -> Result<Option<ClipboardItem>> {
        let _source_write = super::integration::begin_source_write(&self.app_handle);
        // Get image bytes for hash
        let png_buffer = image_data
            .to_png()
            .map_err(|e| anyhow!("Failed to convert image to PNG: {}", e))?;
        let image_bytes = png_buffer.get_bytes();

        let hash = Self::compute_hash(image_bytes);

        // Check if hash already exists
        {
            let conn = self.get_connection()?;
            let exists: bool = conn.query_row(
                "SELECT COUNT(*) > 0 FROM clipboard_history WHERE content_hash = ?1",
                params![hash],
                |row| row.get(0),
            )?;

            if exists {
                debug!("Clipboard image already exists (hash: {})", hash);
                return self.refresh_existing_item(&hash);
            }
        }

        // Get image dimensions
        let (width, height) = image_data.get_size();

        let attachment = self.import_image_bytes(image_bytes)?;
        let filename = attachment.import.file_name.clone();
        let size_bytes = image_bytes.len() as i64;
        let now = Utc::now().timestamp();
        let preview = format!("Image {}x{}", width, height);

        let conn = self.get_connection()?;
        conn.execute(
            "INSERT INTO clipboard_history (
                content_type, content_preview, content_hash, image_path,
                created_at, size_bytes
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params!["image", preview, hash, filename, now, size_bytes],
        )?;

        let item = ClipboardItem {
            id: conn.last_insert_rowid(),
            title: None,
            content_type: "image".to_string(),
            content_preview: preview,
            content_hash: hash,
            full_text: None,
            image_path: Some(filename),
            source_app: None,
            is_favorite: false,
            is_pinned: false,
            created_at: DateTime::from_timestamp(now, 0)
                .unwrap_or_default()
                .with_timezone(&Local)
                .to_rfc3339(),
            size_bytes,
        };

        info!("Added clipboard image entry with id {}", item.id);

        // Emit event
        let item = self.normalize_item_for_client(item);

        if let Err(e) =
            (ClipboardUpdatePayload::Added { item: item.clone() }).emit(&self.app_handle)
        {
            error!("Failed to emit clipboard-added event: {}", e);
        }

        match self.cleanup_old_entries() {
            Ok(deleted_ids) => self.emit_deleted_many(deleted_ids),
            Err(error) => error!("Clipboard item saved; retention cleanup pending: {error}"),
        }

        Ok(Some(item))
    }

    #[cfg(target_os = "macos")]
    fn add_image_png(
        &self,
        hash: String,
        width: u32,
        height: u32,
        png_bytes: &[u8],
    ) -> Result<Option<ClipboardItem>> {
        let _source_write = super::integration::begin_source_write(&self.app_handle);
        {
            let conn = self.get_connection()?;
            let exists: bool = conn.query_row(
                "SELECT COUNT(*) > 0 FROM clipboard_history WHERE content_hash = ?1",
                params![hash],
                |row| row.get(0),
            )?;

            if exists {
                debug!("Clipboard image already exists (hash: {})", hash);
                return self.refresh_existing_item(&hash);
            }
        }

        let attachment = self.import_image_bytes(png_bytes)?;
        let filename = attachment.import.file_name.clone();
        let size_bytes = png_bytes.len() as i64;
        let now = Utc::now().timestamp();
        let preview = format!("Image {}x{}", width, height);

        let conn = self.get_connection()?;
        conn.execute(
            "INSERT INTO clipboard_history (
                content_type, content_preview, content_hash, image_path,
                created_at, size_bytes
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params!["image", preview, hash, filename, now, size_bytes],
        )?;

        let item = ClipboardItem {
            id: conn.last_insert_rowid(),
            title: None,
            content_type: "image".to_string(),
            content_preview: preview,
            content_hash: hash,
            full_text: None,
            image_path: Some(filename),
            source_app: None,
            is_favorite: false,
            is_pinned: false,
            created_at: DateTime::from_timestamp(now, 0)
                .unwrap_or_default()
                .with_timezone(&Local)
                .to_rfc3339(),
            size_bytes,
        };

        info!("Added clipboard image entry with id {}", item.id);

        let item = self.normalize_item_for_client(item);

        if let Err(e) =
            (ClipboardUpdatePayload::Added { item: item.clone() }).emit(&self.app_handle)
        {
            error!("Failed to emit clipboard-added event: {}", e);
        }

        match self.cleanup_old_entries() {
            Ok(deleted_ids) => self.emit_deleted_many(deleted_ids),
            Err(error) => error!("Clipboard item saved; retention cleanup pending: {error}"),
        }

        Ok(Some(item))
    }

    fn query_items_page<P>(
        &self,
        conn: &Connection,
        sql: &str,
        params: P,
        page_size: usize,
    ) -> Result<ClipboardPageResult>
    where
        P: rusqlite::Params,
    {
        let mut stmt = conn.prepare(sql)?;
        let items: Vec<ClipboardItem> = stmt
            .query_map(params, Self::map_clipboard_item)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let has_more = items.len() > page_size;
        let items = if has_more {
            items[..page_size].to_vec()
        } else {
            items
        }
        .into_iter()
        .map(|item| self.normalize_item_for_client(item))
        .collect();

        Ok(ClipboardPageResult { items, has_more })
    }

    /// Get clipboard items with pagination.
    pub fn get_items(
        &self,
        page: usize,
        page_size: usize,
        sort: &str,
        content_type: &str,
        favorite_only: bool,
    ) -> Result<ClipboardPageResult> {
        let conn = self.get_connection()?;
        let offset = (page * page_size) as i64;
        let fetch_count = page_size as i64 + 1;
        let order = if sort == "oldest" { "ASC" } else { "DESC" };
        let query = |where_clause: &str| {
            format!(
                "SELECT id, title, content_type, content_preview, content_hash, full_text, image_path,
                        source_app, is_favorite, is_pinned, created_at, size_bytes
                 FROM clipboard_history
                 {}
                 ORDER BY is_pinned DESC, created_at {}
                 LIMIT ?{} OFFSET ?{}",
                where_clause,
                order,
                if content_type == "all" || content_type == "text" { 1 } else { 2 },
                if content_type == "all" || content_type == "text" { 2 } else { 3 },
            )
        };

        match (favorite_only, content_type) {
            (false, "all") => {
                self.query_items_page(&conn, &query(""), params![fetch_count, offset], page_size)
            }
            (true, "all") => self.query_items_page(
                &conn,
                &query("WHERE is_favorite = 1"),
                params![fetch_count, offset],
                page_size,
            ),
            (false, "text") => self.query_items_page(
                &conn,
                &query("WHERE content_type IN ('text', 'richtext')"),
                params![fetch_count, offset],
                page_size,
            ),
            (true, "text") => self.query_items_page(
                &conn,
                &query("WHERE is_favorite = 1 AND content_type IN ('text', 'richtext')"),
                params![fetch_count, offset],
                page_size,
            ),
            (false, _) => self.query_items_page(
                &conn,
                &query("WHERE content_type = ?1"),
                params![content_type, fetch_count, offset],
                page_size,
            ),
            (true, _) => self.query_items_page(
                &conn,
                &query("WHERE is_favorite = 1 AND content_type = ?1"),
                params![content_type, fetch_count, offset],
                page_size,
            ),
        }
    }

    /// Get favorite clipboard items for the overlay favorite view.
    pub fn get_favorite_items(
        &self,
        page: usize,
        page_size: usize,
        content_type: &str,
        sort: &str,
    ) -> Result<ClipboardPageResult> {
        self.get_items(page, page_size, sort, content_type, true)
    }

    /// Search clipboard items
    pub fn search(&self, query: &str, content_type: &str) -> Result<Vec<ClipboardItem>> {
        let conn = self.get_connection()?;
        let search_query = format!("%{}%", query.trim());

        let mut stmt = if content_type == "all" {
            conn.prepare(
                "SELECT id, title, content_type, content_preview, content_hash, full_text, image_path,
                        source_app, is_favorite, is_pinned, created_at, size_bytes
                 FROM clipboard_history
                 WHERE title LIKE ?1 OR content_preview LIKE ?1 OR full_text LIKE ?1
                 ORDER BY is_pinned DESC, created_at DESC
                 LIMIT ?2",
            )?
        } else if content_type == "text" {
            conn.prepare(
                "SELECT id, title, content_type, content_preview, content_hash, full_text, image_path,
                        source_app, is_favorite, is_pinned, created_at, size_bytes
                 FROM clipboard_history
                 WHERE (title LIKE ?1 OR content_preview LIKE ?1 OR full_text LIKE ?1)
                    AND content_type IN ('text', 'richtext')
                 ORDER BY is_pinned DESC, created_at DESC
                 LIMIT ?2",
            )?
        } else {
            conn.prepare(
                "SELECT id, title, content_type, content_preview, content_hash, full_text, image_path,
                        source_app, is_favorite, is_pinned, created_at, size_bytes
                 FROM clipboard_history
                 WHERE (title LIKE ?1 OR content_preview LIKE ?1 OR full_text LIKE ?1) AND content_type = ?2
                 ORDER BY is_pinned DESC, created_at DESC
                 LIMIT ?3",
            )?
        };

        let items = if content_type == "all" || content_type == "text" {
            stmt.query_map(
                params![search_query, SEARCH_RESULT_LIMIT],
                Self::map_clipboard_item,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?
        } else {
            stmt.query_map(
                params![search_query, content_type, SEARCH_RESULT_LIMIT],
                Self::map_clipboard_item,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?
        };

        Ok(items
            .into_iter()
            .map(|item| self.normalize_item_for_client(item))
            .collect())
    }

    /// Toggle favorite status
    pub fn toggle_favorite(&self, id: i64) -> Result<()> {
        let _source_write = super::integration::begin_source_write(&self.app_handle);
        let conn = self.get_connection()?;
        conn.execute(
            "UPDATE clipboard_history SET is_favorite = NOT is_favorite WHERE id = ?1",
            params![id],
        )?;
        debug!("Toggled favorite status for clipboard item {}", id);

        if let Some(item) = self.get_item_by_id(id)? {
            if let Err(e) = (ClipboardUpdatePayload::Updated { item }).emit(&self.app_handle) {
                error!("Failed to emit clipboard-updated event: {}", e);
            }
        }

        Ok(())
    }

    /// Toggle pin status
    pub fn toggle_pin(&self, id: i64) -> Result<()> {
        let _source_write = super::integration::begin_source_write(&self.app_handle);
        let conn = self.get_connection()?;
        conn.execute(
            "UPDATE clipboard_history SET is_pinned = NOT is_pinned WHERE id = ?1",
            params![id],
        )?;
        debug!("Toggled pin status for clipboard item {}", id);

        if let Some(item) = self.get_item_by_id(id)? {
            if let Err(e) = (ClipboardUpdatePayload::Updated { item }).emit(&self.app_handle) {
                error!("Failed to emit clipboard-updated event: {}", e);
            }
        }

        Ok(())
    }

    /// Update the user-facing title for a clipboard item.
    pub fn update_title(&self, id: i64, title: Option<String>) -> Result<()> {
        let _source_write = super::integration::begin_source_write(&self.app_handle);
        let title = title
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let conn = self.get_connection()?;
        conn.execute(
            "UPDATE clipboard_history SET title = ?1 WHERE id = ?2",
            params![title, id],
        )?;
        debug!("Updated title for clipboard item {}", id);

        if let Some(item) = self.get_item_by_id(id)? {
            if let Err(e) = (ClipboardUpdatePayload::Updated { item }).emit(&self.app_handle) {
                error!("Failed to emit clipboard-updated event: {}", e);
            }
        }

        Ok(())
    }

    /// Delete a clipboard item
    pub fn delete_item(&self, id: i64) -> Result<()> {
        self.attachment_service()?
            .delete_source_record(SourceTable::Clipboard, id.to_string())
            .map_err(anyhow::Error::msg)?;
        if let Err(error) = (ClipboardUpdatePayload::Deleted { id }).emit(&self.app_handle) {
            error!("Failed to emit clipboard-deleted event: {error}");
        }
        Ok(())
    }

    /// 每个源记录均保留可重试删除回执；部分失败时不发布全部清空的假结果。
    pub fn clear_history(&self, keep_pinned: bool) -> Result<()> {
        let ids = {
            let conn = self.get_connection()?;
            let mut query = conn.prepare(if keep_pinned {
                "SELECT id FROM clipboard_history WHERE is_pinned=0 AND is_favorite=0"
            } else {
                "SELECT id FROM clipboard_history"
            })?;
            let ids = query
                .query_map([], |row| row.get::<_, i64>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            ids
        };
        let service = self.attachment_service()?;
        for id in ids {
            service
                .delete_source_record(SourceTable::Clipboard, id.to_string())
                .map_err(anyhow::Error::msg)?;
        }
        if let Err(error) = (ClipboardUpdatePayload::Cleared { keep_pinned }).emit(&self.app_handle)
        {
            error!("Failed to emit clipboard-cleared event: {error}");
        }
        Ok(())
    }

    /// Copy a clipboard item to system clipboard
    pub fn copy_to_clipboard(&self, id: i64) -> Result<()> {
        let conn = self.get_connection()?;
        let item = conn.query_row(
            "SELECT id, title, content_type, content_preview, content_hash, full_text, image_path,
                    source_app, is_favorite, is_pinned, created_at, size_bytes
             FROM clipboard_history WHERE id = ?1",
            params![id],
            Self::map_clipboard_item,
        )?;

        match item.content_type.as_str() {
            "text" | "richtext" => {
                let text = item.full_text.unwrap_or(item.content_preview);
                self.write_text_to_system_clipboard(text)
            }
            "file" => {
                let text = item.full_text.unwrap_or(item.content_preview);
                self.write_stored_file_content_to_system_clipboard(text)
            }
            "image" => {
                let path = item
                    .image_path
                    .ok_or_else(|| anyhow!("Image path not found"))?;
                self.write_image_to_system_clipboard(self.read_clipboard_image(&path)?)
            }
            _ => Err(anyhow!("Unsupported content type")),
        }
    }

    /// Get clipboard statistics
    pub fn get_stats(&self) -> Result<ClipboardStats> {
        let conn = self.get_connection()?;

        let total_items: i64 =
            conn.query_row("SELECT COUNT(*) FROM clipboard_history", [], |row| {
                row.get(0)
            })?;

        let favorites_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM clipboard_history WHERE is_favorite = 1",
            [],
            |row| row.get(0),
        )?;

        let pinned_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM clipboard_history WHERE is_pinned = 1",
            [],
            |row| row.get(0),
        )?;

        let total_size_bytes: i64 = conn.query_row(
            "SELECT COALESCE(SUM(size_bytes), 0) FROM clipboard_history",
            [],
            |row| row.get(0),
        )?;

        Ok(ClipboardStats {
            total_items,
            favorites_count,
            pinned_count,
            total_size_bytes,
        })
    }

    /// Cleanup old entries when exceeding max_records
    fn cleanup_old_entries(&self) -> Result<Vec<i64>> {
        let _source_write = super::integration::begin_source_write(&self.app_handle);
        let max_records = crate::settings::get_settings(&self.app_handle).clipboard_max_records;
        let conn = self.get_connection()?;

        if max_records == 0 {
            return Ok(Vec::new());
        }

        // Count current entries
        let current_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM clipboard_history", [], |row| {
                row.get(0)
            })?;

        if current_count <= max_records as i64 {
            return Ok(Vec::new());
        }

        // Get entries to delete (oldest, non-pinned, non-favorite)
        let excess = current_count - max_records as i64;
        let mut stmt = conn.prepare(
            "SELECT id, image_path FROM clipboard_history
             WHERE is_pinned = 0 AND is_favorite = 0
             ORDER BY created_at ASC
             LIMIT ?1",
        )?;

        let entries_to_delete: Vec<(i64, Option<String>)> = stmt
            .query_map(params![excess], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let mut deleted_ids = Vec::new();

        let service = self.attachment_service()?;
        for (id, _) in entries_to_delete {
            if service
                .delete_source_record(SourceTable::Clipboard, id.to_string())
                .map_err(anyhow::Error::msg)?
            {
                deleted_ids.push(id);
            }
        }

        debug!("Cleaned up {} old clipboard entries", excess);
        Ok(deleted_ids)
    }
}

/// Handler for clipboard change events
#[cfg(not(target_os = "macos"))]
struct ClipboardChangeHandler {
    manager: ClipboardManager,
    clipboard: ClipboardContext,
}

#[cfg(not(target_os = "macos"))]
impl ClipboardHandler for ClipboardChangeHandler {
    fn on_clipboard_change(&mut self) {
        match catch_unwind(AssertUnwindSafe(|| {
            self.manager.process_clipboard_change(&mut self.clipboard)
        })) {
            Ok(Ok(())) => {}
            Ok(Err(e)) => error!("Failed to process clipboard change: {}", e),
            Err(_) => error!("Clipboard watcher panicked while processing clipboard change"),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn monitor_unchanged_generation_does_not_read_or_encode_payload() {
        let result = super::read_changed_clipboard::<(), ()>(Some(42), 42, || {
            panic!("unchanged clipboard must not read or encode its payload")
        });
        assert_eq!(result, Ok(None));
        assert_eq!(
            super::read_changed_clipboard(None, 42, || Ok::<_, ()>(7)),
            Ok(Some(7))
        );
        assert_eq!(
            super::read_changed_clipboard(Some(42), 43, || Ok::<_, ()>(8)),
            Ok(Some(8))
        );
    }

    #[test]
    fn monitor_timeout_does_not_admit_another_queued_read() {
        use std::sync::{atomic::AtomicBool, Arc};
        use std::time::{Duration, Instant};
        let in_flight = Arc::new(AtomicBool::new(false));
        let deadline = Instant::now();
        let queued = super::ClipboardMonitorRead::acquire(&in_flight, deadline).unwrap();
        for _ in 0..100 {
            assert!(super::ClipboardMonitorRead::acquire(&in_flight, deadline).is_none());
        }
        assert!(!queued.may_start(deadline + Duration::from_secs(2)));
        drop(queued);
        assert!(super::ClipboardMonitorRead::acquire(&in_flight, deadline).is_some());
    }

    #[test]
    fn monitor_dropped_timeout_receiver_leaves_one_expired_closure() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        use std::time::{Duration, Instant};
        let in_flight = Arc::new(AtomicBool::new(false));
        let touched_clipboard = Arc::new(AtomicBool::new(false));
        let deadline = Instant::now();
        let permit = super::ClipboardMonitorRead::acquire(&in_flight, deadline).unwrap();
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let touched = touched_clipboard.clone();
        let queued = move || {
            if !permit.may_start(deadline + Duration::from_secs(2)) {
                return;
            }
            touched.store(true, Ordering::Release);
            let _ = sender.send(permit);
        };
        assert!(matches!(
            receiver.recv_timeout(Duration::ZERO),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
        drop(receiver);
        for _ in 0..100 {
            assert!(super::ClipboardMonitorRead::acquire(&in_flight, deadline).is_none());
        }
        queued();
        assert!(!touched_clipboard.load(Ordering::Acquire));
        assert!(super::ClipboardMonitorRead::acquire(&in_flight, deadline).is_some());
    }

    #[test]
    fn monitor_timed_out_running_read_releases_after_failed_send() {
        use std::sync::{atomic::AtomicBool, Arc};
        use std::time::{Duration, Instant};
        let in_flight = Arc::new(AtomicBool::new(false));
        let now = Instant::now();
        let permit =
            super::ClipboardMonitorRead::acquire(&in_flight, now + Duration::from_secs(2)).unwrap();
        assert!(permit.may_start(now));
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        assert!(receiver.recv_timeout(Duration::ZERO).is_err());
        drop(receiver);
        assert!(super::ClipboardMonitorRead::acquire(&in_flight, now).is_none());
        drop(sender.send(permit));
        assert!(super::ClipboardMonitorRead::acquire(&in_flight, now).is_some());
    }

    #[test]
    fn monitor_started_read_keeps_gate_until_result_is_consumed() {
        use std::sync::{atomic::AtomicBool, Arc};
        use std::time::{Duration, Instant};
        let in_flight = Arc::new(AtomicBool::new(false));
        let now = Instant::now();
        let started =
            super::ClipboardMonitorRead::acquire(&in_flight, now + Duration::from_secs(2)).unwrap();
        assert!(started.may_start(now));
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        sender.send(started).unwrap();
        assert!(super::ClipboardMonitorRead::acquire(&in_flight, now).is_none());
        drop(receiver.recv().unwrap());
        assert!(super::ClipboardMonitorRead::acquire(&in_flight, now).is_some());
    }

    #[test]
    fn unified_files_never_fall_back_to_text_on_invalid_payload_or_native_error() {
        assert!(super::restore_files_strict::<()>("[broken", |_| panic!(
            "invalid payload dispatched"
        ))
        .is_err());
        assert!(
            super::restore_files_strict::<()>("relative/path", |_| panic!(
                "relative path dispatched"
            ))
            .is_err()
        );
        assert!(
            super::restore_files_strict::<()>("[]", |_| panic!("empty payload dispatched"))
                .is_err()
        );
        let temp = tempfile::tempdir().unwrap();
        let path = temp
            .path()
            .join("synthetic-file")
            .to_string_lossy()
            .into_owned();
        let payload = serde_json::to_string(&vec![path.clone()]).unwrap();
        let result = super::restore_files_strict::<()>(&payload, |paths| {
            assert_eq!(paths, vec![path]);
            Err(anyhow::anyhow!("native failure fixture"))
        });
        assert!(result.is_err());
    }

    #[cfg(target_os = "macos")]
    use super::process_macos_clipboard_representations;
    use super::{
        clipboard_file_preview, deserialize_clipboard_file_paths, normalize_clipboard_file_paths,
        serialize_clipboard_file_paths, ClipboardManager, TEXT_PREVIEW_MAX_CHARS,
    };

    #[test]
    fn text_preview_keeps_short_text_unchanged() {
        assert_eq!(ClipboardManager::text_preview("hello"), "hello");
    }

    #[test]
    fn text_preview_truncates_ascii_text() {
        let text = "a".repeat(TEXT_PREVIEW_MAX_CHARS + 1);
        let preview = ClipboardManager::text_preview(&text);

        assert_eq!(
            preview,
            format!("{}...", "a".repeat(TEXT_PREVIEW_MAX_CHARS))
        );
    }

    #[test]
    fn text_preview_truncates_multibyte_text_without_panicking() {
        let text = "中文".repeat(TEXT_PREVIEW_MAX_CHARS);
        let preview = ClipboardManager::text_preview(&text);

        assert_eq!(preview.chars().count(), TEXT_PREVIEW_MAX_CHARS + 3);
        assert!(preview.ends_with("..."));
    }

    #[test]
    fn normalize_clipboard_file_paths_strips_uri_prefix_and_decodes_spaces() {
        let paths = vec![
            "file:///tmp/hello%20world.txt".to_string(),
            "file://localhost/tmp/demo.txt".to_string(),
        ];

        let normalized = normalize_clipboard_file_paths(&paths);

        assert_eq!(normalized, vec!["/tmp/hello world.txt", "/tmp/demo.txt"]);
    }

    #[test]
    fn normalize_clipboard_file_paths_preserves_percent_sequences_in_local_paths() {
        let normalized = normalize_clipboard_file_paths(&["/tmp/100%20literal.txt".to_string()]);

        assert_eq!(normalized, vec!["/tmp/100%20literal.txt"]);
    }

    #[test]
    fn serialize_and_deserialize_clipboard_file_paths_round_trip() {
        let serialized = serialize_clipboard_file_paths(&[
            "/tmp/alpha.txt".to_string(),
            "/tmp/beta.txt".to_string(),
        ])
        .unwrap();

        let deserialized = deserialize_clipboard_file_paths(&serialized);

        assert_eq!(deserialized, vec!["/tmp/alpha.txt", "/tmp/beta.txt"]);
    }

    #[test]
    fn deserialize_legacy_file_paths_normalizes_each_line() {
        let deserialized = deserialize_clipboard_file_paths(
            "file:///tmp/alpha%20one.txt\nfile://localhost/tmp/beta.txt",
        );

        assert_eq!(deserialized, vec!["/tmp/alpha one.txt", "/tmp/beta.txt"]);
    }

    #[test]
    fn file_hash_cannot_collide_with_identical_plain_text_payload() {
        let serialized = r#"["/tmp/alpha.txt"]"#;

        assert_ne!(
            ClipboardManager::compute_file_hash(serialized),
            ClipboardManager::compute_hash(serialized.as_bytes())
        );
    }

    #[test]
    fn clipboard_file_preview_uses_file_names() {
        let preview = clipboard_file_preview(&[
            "/tmp/alpha.txt".to_string(),
            "/tmp/nested/beta.png".to_string(),
        ]);

        assert_eq!(preview, "alpha.txt\nbeta.png");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_clipboard_processing_prefers_files_over_other_formats() {
        let calls = std::cell::RefCell::new(Vec::new());

        process_macos_clipboard_representations(
            || {
                calls.borrow_mut().push("files");
                Ok(true)
            },
            || {
                calls.borrow_mut().push("image");
                Ok(true)
            },
            || {
                calls.borrow_mut().push("text");
                Ok(true)
            },
        )
        .unwrap();

        assert_eq!(calls.into_inner(), vec!["files"]);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_clipboard_processing_prefers_image_when_both_formats_exist() {
        let calls = std::cell::RefCell::new(Vec::new());

        process_macos_clipboard_representations(
            || {
                calls.borrow_mut().push("files");
                Ok(false)
            },
            || {
                calls.borrow_mut().push("image");
                Ok(true)
            },
            || {
                calls.borrow_mut().push("text");
                Ok(true)
            },
        )
        .unwrap();

        assert_eq!(calls.into_inner(), vec!["files", "image"]);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_clipboard_processing_falls_back_to_text_without_image() {
        let calls = std::cell::RefCell::new(Vec::new());

        process_macos_clipboard_representations(
            || {
                calls.borrow_mut().push("files");
                Ok(false)
            },
            || {
                calls.borrow_mut().push("image");
                Ok(false)
            },
            || {
                calls.borrow_mut().push("text");
                Ok(true)
            },
        )
        .unwrap();

        assert_eq!(calls.into_inner(), vec!["files", "image", "text"]);
    }
}
