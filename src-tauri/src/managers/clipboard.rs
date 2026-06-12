use anyhow::{anyhow, Result};
use chrono::{DateTime, Local, Utc};
#[cfg(not(target_os = "macos"))]
use clipboard_rs::common::RustImage;
#[cfg(not(target_os = "macos"))]
use clipboard_rs::{Clipboard, ClipboardContext};
#[cfg(not(target_os = "macos"))]
use clipboard_rs::{ClipboardHandler, ClipboardWatcher, ClipboardWatcherContext};
use log::{debug, error, info};
use rusqlite::{params, Connection, OptionalExtension};
use rusqlite_migration::{Migrations, M};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use specta::Type;
use std::fs;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;
use tauri::image::Image;
use tauri::AppHandle;
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_specta::Event;

/// Database migrations for clipboard history.
static MIGRATIONS: &[M] = &[M::up(
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
)];

#[derive(Clone, Debug, Serialize, Deserialize, Type)]
pub struct ClipboardItem {
    pub id: i64,
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
    #[serde(rename = "deleted")]
    Deleted { id: i64 },
}

#[derive(Clone)]
pub struct ClipboardManager {
    app_handle: AppHandle,
    db_path: PathBuf,
    images_dir: PathBuf,
    last_hash: Arc<Mutex<String>>,
    monitoring_started: Arc<AtomicBool>,
}

impl ClipboardManager {
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
            last_hash: Arc::new(Mutex::new(String::new())),
            monitoring_started: Arc::new(AtomicBool::new(false)),
        };

        // Initialize database
        manager.init_database()?;

        Ok(manager)
    }

    fn init_database(&self) -> Result<()> {
        info!("Initializing clipboard database at {:?}", self.db_path);

        let mut conn = Connection::open(&self.db_path)?;

        // Create migrations object and run to latest version
        let migrations = Migrations::new(MIGRATIONS.to_vec());

        // Validate migrations in debug builds
        #[cfg(debug_assertions)]
        migrations.validate().expect("Invalid migrations");

        // Get current version before migration
        let version_before: i32 =
            conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        debug!(
            "Clipboard database version before migration: {}",
            version_before
        );

        // Apply any pending migrations
        migrations.to_latest(&mut conn)?;

        // Get version after migration
        let version_after: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;

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
        Ok(Connection::open(&self.db_path)?)
    }

    fn map_clipboard_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<ClipboardItem> {
        let created_at: i64 = row.get("created_at")?;
        let datetime = DateTime::from_timestamp(created_at, 0)
            .unwrap_or_default()
            .with_timezone(&Local);

        Ok(ClipboardItem {
            id: row.get("id")?,
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

    /// Start monitoring clipboard changes
    pub fn start_monitoring(&self) {
        if self.monitoring_started.swap(true, Ordering::SeqCst) {
            debug!("Clipboard monitoring already active");
            return;
        }

        let manager = self.clone();

        std::thread::spawn(move || {
            info!("Starting clipboard monitoring thread");

            if let Err(e) = manager.sync_current_clipboard() {
                error!("Failed to capture initial clipboard state: {}", e);
            }

            #[cfg(target_os = "macos")]
            manager.run_polling_monitor();

            #[cfg(not(target_os = "macos"))]
            manager.run_watcher_monitor();

            error!("Clipboard monitoring thread exited unexpectedly");
            manager.monitoring_started.store(false, Ordering::SeqCst);
        });
    }

    #[cfg(target_os = "macos")]
    fn run_polling_monitor(&self) {
        info!("Using polling clipboard monitor on macOS");

        loop {
            if self.monitoring_enabled() {
                if let Err(e) = self.sync_current_clipboard() {
                    error!("Failed to poll clipboard state: {}", e);
                }
            }

            std::thread::sleep(Duration::from_millis(500));
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

    pub fn sync_current_clipboard(&self) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            return self.process_tauri_clipboard_change();
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
        settings.experimental_enabled && settings.clipboard_enabled
    }

    fn should_process_hash(&self, hash: &str) -> bool {
        let mut last_hash = self.last_hash.lock().unwrap();
        if last_hash.as_str() == hash {
            return false;
        }

        *last_hash = hash.to_string();
        true
    }

    fn process_text_change(&self, text: &str) -> Result<()> {
        let hash = Self::compute_hash(text.as_bytes());
        if !self.should_process_hash(&hash) {
            return Ok(());
        }

        self.add_text(text).map(|_| ())
    }

    #[cfg(not(target_os = "macos"))]
    fn process_image_change(&self, image_data: &clipboard_rs::RustImageData) -> Result<()> {
        let png_buffer = image_data
            .to_png()
            .map_err(|e| anyhow!("Failed to convert image to PNG: {}", e))?;
        let hash = Self::compute_hash(png_buffer.get_bytes());

        if !self.should_process_hash(&hash) {
            return Ok(());
        }

        self.add_image(image_data).map(|_| ())
    }

    #[cfg(target_os = "macos")]
    fn process_tauri_image_change(&self, image: &Image<'_>) -> Result<()> {
        let png_bytes = Self::encode_tauri_image_png(image)?;
        let hash = Self::compute_hash(&png_bytes);

        if !self.should_process_hash(&hash) {
            return Ok(());
        }

        self.add_image_png(hash, image.width(), image.height(), &png_bytes)
            .map(|_| ())
    }

    #[cfg(target_os = "macos")]
    fn process_tauri_clipboard_change(&self) -> Result<()> {
        if !self.monitoring_enabled() {
            return Ok(());
        }

        let clipboard = self.app_handle.clipboard();

        if let Ok(text) = clipboard.read_text() {
            if !text.is_empty() {
                return self.process_text_change(&text);
            }
        }

        if let Ok(image) = clipboard.read_image() {
            return self.process_tauri_image_change(&image);
        }

        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    fn process_clipboard_change(&self, clipboard: &mut ClipboardContext) -> Result<()> {
        if !self.monitoring_enabled() {
            return Ok(());
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

    /// Add a text entry to clipboard history
    pub fn add_text(&self, text: &str) -> Result<Option<ClipboardItem>> {
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
                return Ok(None);
            }
        }

        let preview = if text.len() > 200 {
            format!("{}...", &text[..200])
        } else {
            text.to_string()
        };

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

        self.cleanup_old_entries()?;

        Ok(Some(item))
    }

    #[cfg(not(target_os = "macos"))]
    /// Add an image entry to clipboard history
    pub fn add_image(
        &self,
        image_data: &clipboard_rs::RustImageData,
    ) -> Result<Option<ClipboardItem>> {
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
                return Ok(None);
            }
        }

        // Get image dimensions
        let (width, height) = image_data.get_size();

        // Generate filename
        let filename = format!("{}.png", hash);
        let image_path = self.images_dir.join(&filename);

        // Save as PNG
        image_data
            .save_to_path(image_path.to_str().unwrap_or_default())
            .map_err(|e| anyhow!("Failed to save image: {}", e))?;

        let size_bytes = fs::metadata(&image_path)?.len() as i64;
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

        self.cleanup_old_entries()?;

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
        {
            let conn = self.get_connection()?;
            let exists: bool = conn.query_row(
                "SELECT COUNT(*) > 0 FROM clipboard_history WHERE content_hash = ?1",
                params![hash],
                |row| row.get(0),
            )?;

            if exists {
                debug!("Clipboard image already exists (hash: {})", hash);
                return Ok(None);
            }
        }

        let filename = format!("{}.png", hash);
        let image_path = self.images_dir.join(&filename);
        fs::write(&image_path, png_bytes)?;

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

        self.cleanup_old_entries()?;

        Ok(Some(item))
    }

    /// Get clipboard items with pagination
    pub fn get_items(
        &self,
        page: usize,
        page_size: usize,
        sort: &str,
    ) -> Result<ClipboardPageResult> {
        let conn = self.get_connection()?;
        let offset = (page * page_size) as i64;
        let limit = page_size as i64;

        let order = if sort == "oldest" { "ASC" } else { "DESC" };

        // Fetch one extra to determine if there are more items
        let fetch_count = limit + 1;

        let mut stmt = conn.prepare(&format!(
            "SELECT id, content_type, content_preview, content_hash, full_text, image_path, 
                    source_app, is_favorite, is_pinned, created_at, size_bytes
             FROM clipboard_history
             ORDER BY is_pinned DESC, created_at {}
             LIMIT ?1 OFFSET ?2",
            order
        ))?;

        let items: Vec<ClipboardItem> = stmt
            .query_map(params![fetch_count, offset], Self::map_clipboard_item)?
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

    /// Search clipboard items
    pub fn search(&self, query: &str, content_type: &str) -> Result<Vec<ClipboardItem>> {
        let conn = self.get_connection()?;
        let search_query = format!("%{}%", query);

        let mut stmt = if content_type == "all" {
            conn.prepare(
                "SELECT id, content_type, content_preview, content_hash, full_text, image_path, 
                        source_app, is_favorite, is_pinned, created_at, size_bytes
                 FROM clipboard_history
                 WHERE content_preview LIKE ?1 OR full_text LIKE ?1
                 ORDER BY is_pinned DESC, created_at DESC",
            )?
        } else {
            conn.prepare(
                "SELECT id, content_type, content_preview, content_hash, full_text, image_path, 
                        source_app, is_favorite, is_pinned, created_at, size_bytes
                 FROM clipboard_history
                 WHERE (content_preview LIKE ?1 OR full_text LIKE ?1) AND content_type = ?2
                 ORDER BY is_pinned DESC, created_at DESC",
            )?
        };

        let items = if content_type == "all" {
            stmt.query_map(params![search_query], Self::map_clipboard_item)?
                .collect::<std::result::Result<Vec<_>, _>>()?
        } else {
            stmt.query_map(
                params![search_query, content_type],
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
        let conn = self.get_connection()?;
        conn.execute(
            "UPDATE clipboard_history SET is_favorite = NOT is_favorite WHERE id = ?1",
            params![id],
        )?;
        debug!("Toggled favorite status for clipboard item {}", id);
        Ok(())
    }

    /// Toggle pin status
    pub fn toggle_pin(&self, id: i64) -> Result<()> {
        let conn = self.get_connection()?;
        conn.execute(
            "UPDATE clipboard_history SET is_pinned = NOT is_pinned WHERE id = ?1",
            params![id],
        )?;
        debug!("Toggled pin status for clipboard item {}", id);
        Ok(())
    }

    /// Delete a clipboard item
    pub fn delete_item(&self, id: i64) -> Result<()> {
        let conn = self.get_connection()?;

        // Get image path before deleting
        let image_path: Option<String> = conn
            .query_row(
                "SELECT image_path FROM clipboard_history WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?;

        // Delete from database
        conn.execute("DELETE FROM clipboard_history WHERE id = ?1", params![id])?;

        // Delete image file if exists
        if let Some(path) = image_path {
            let full_path = self.images_dir.join(&path);
            if full_path.exists() {
                if let Err(e) = fs::remove_file(&full_path) {
                    error!("Failed to delete clipboard image {}: {}", path, e);
                }
            }
        }

        debug!("Deleted clipboard item {}", id);

        // Emit event
        if let Err(e) = (ClipboardUpdatePayload::Deleted { id }).emit(&self.app_handle) {
            error!("Failed to emit clipboard-deleted event: {}", e);
        }

        Ok(())
    }

    /// Clear clipboard history
    pub fn clear_history(&self, keep_pinned: bool) -> Result<()> {
        let conn = self.get_connection()?;

        if keep_pinned {
            // Get all non-pinned, non-favorite items to delete their images
            let mut stmt = conn.prepare(
                "SELECT image_path FROM clipboard_history WHERE is_pinned = 0 AND is_favorite = 0 AND image_path IS NOT NULL"
            )?;
            let image_paths: Vec<String> = stmt
                .query_map([], |row| row.get(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;

            // Delete images
            for path in image_paths {
                let full_path = self.images_dir.join(&path);
                if full_path.exists() {
                    if let Err(e) = fs::remove_file(&full_path) {
                        error!("Failed to delete clipboard image {}: {}", path, e);
                    }
                }
            }

            // Delete non-pinned, non-favorite entries
            conn.execute(
                "DELETE FROM clipboard_history WHERE is_pinned = 0 AND is_favorite = 0",
                [],
            )?;
        } else {
            // Get all image paths to delete
            let mut stmt = conn
                .prepare("SELECT image_path FROM clipboard_history WHERE image_path IS NOT NULL")?;
            let image_paths: Vec<String> = stmt
                .query_map([], |row| row.get(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;

            // Delete images
            for path in image_paths {
                let full_path = self.images_dir.join(&path);
                if full_path.exists() {
                    if let Err(e) = fs::remove_file(&full_path) {
                        error!("Failed to delete clipboard image {}: {}", path, e);
                    }
                }
            }

            // Delete all entries
            conn.execute("DELETE FROM clipboard_history", [])?;
        }

        info!("Cleared clipboard history (keep_pinned: {})", keep_pinned);
        Ok(())
    }

    /// Copy a clipboard item to system clipboard
    pub fn copy_to_clipboard(&self, id: i64) -> Result<()> {
        let conn = self.get_connection()?;
        let item = conn.query_row(
            "SELECT id, content_type, content_preview, content_hash, full_text, image_path, 
                    source_app, is_favorite, is_pinned, created_at, size_bytes
             FROM clipboard_history WHERE id = ?1",
            params![id],
            Self::map_clipboard_item,
        )?;

        match item.content_type.as_str() {
            "text" => {
                let text = item.full_text.unwrap_or(item.content_preview);
                self.app_handle
                    .clipboard()
                    .write_text(text)
                    .map_err(|e| anyhow!("Failed to write text to system clipboard: {}", e))?;
                Ok(())
            }
            "image" => {
                let path = item
                    .image_path
                    .ok_or_else(|| anyhow!("Image path not found"))?;
                let full_path = self.images_dir.join(&path);
                let image = Image::from_path(&full_path)
                    .map_err(|e| anyhow!("Failed to load clipboard image: {}", e))?;
                self.app_handle
                    .clipboard()
                    .write_image(&image)
                    .map_err(|e| anyhow!("Failed to write image to system clipboard: {}", e))?;
                Ok(())
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
    fn cleanup_old_entries(&self) -> Result<()> {
        let max_records = crate::settings::get_settings(&self.app_handle).clipboard_max_records;
        let conn = self.get_connection()?;

        // Count current entries
        let current_count: i64 =
            conn.query_row("SELECT COUNT(*) FROM clipboard_history", [], |row| {
                row.get(0)
            })?;

        if current_count <= max_records as i64 {
            return Ok(());
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

        for (id, image_path) in entries_to_delete {
            // Delete image file if exists
            if let Some(path) = image_path {
                let full_path = self.images_dir.join(&path);
                if full_path.exists() {
                    if let Err(e) = fs::remove_file(&full_path) {
                        error!("Failed to delete clipboard image {}: {}", path, e);
                    }
                }
            }

            // Delete entry
            conn.execute("DELETE FROM clipboard_history WHERE id = ?1", params![id])?;
        }

        debug!("Cleaned up {} old clipboard entries", excess);
        Ok(())
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
        if let Err(e) = self.manager.process_clipboard_change(&mut self.clipboard) {
            error!("Failed to process clipboard change: {}", e);
        }
    }
}
