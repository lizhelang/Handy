use crate::managers::clipboard::{ClipboardManager, ClipboardPageResult, ClipboardStats};
use log::{error, info};
use std::sync::Arc;
use tauri::{AppHandle, State};

#[tauri::command]
#[specta::specta]
pub async fn get_clipboard_items(
    manager: State<'_, Arc<ClipboardManager>>,
    page: usize,
    page_size: usize,
    sort: String,
    content_type: String,
    favorite_only: bool,
) -> Result<ClipboardPageResult, String> {
    manager
        .get_items(page, page_size, &sort, &content_type, favorite_only)
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_favorite_clipboard_items(
    manager: State<'_, Arc<ClipboardManager>>,
    page: usize,
    page_size: usize,
    content_type: String,
    sort: String,
) -> Result<ClipboardPageResult, String> {
    manager
        .get_favorite_items(page, page_size, &content_type, &sort)
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn search_clipboard(
    manager: State<'_, Arc<ClipboardManager>>,
    query: String,
    content_type: String,
) -> Result<Vec<crate::managers::clipboard::ClipboardItem>, String> {
    manager
        .search(&query, &content_type)
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn toggle_clipboard_favorite(
    manager: State<'_, Arc<ClipboardManager>>,
    id: i64,
) -> Result<(), String> {
    manager.toggle_favorite(id).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn toggle_clipboard_pin(
    manager: State<'_, Arc<ClipboardManager>>,
    id: i64,
) -> Result<(), String> {
    manager.toggle_pin(id).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn update_clipboard_title(
    manager: State<'_, Arc<ClipboardManager>>,
    id: i64,
    title: Option<String>,
) -> Result<(), String> {
    manager.update_title(id, title).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn delete_clipboard_item(
    manager: State<'_, Arc<ClipboardManager>>,
    id: i64,
) -> Result<(), String> {
    manager.delete_item(id).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn clear_clipboard_history(
    manager: State<'_, Arc<ClipboardManager>>,
    keep_pinned: bool,
) -> Result<(), String> {
    manager
        .clear_history(keep_pinned)
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn copy_clipboard_to_system(
    manager: State<'_, Arc<ClipboardManager>>,
    id: i64,
) -> Result<(), String> {
    manager.copy_to_clipboard(id).map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn copy_clipboard_content_to_system(
    manager: State<'_, Arc<ClipboardManager>>,
    content_type: String,
    text: Option<String>,
    image_path: Option<String>,
) -> Result<(), String> {
    manager
        .copy_content_to_clipboard(&content_type, text, image_path)
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub fn set_clipboard_overlay_pinned(app: AppHandle, pinned: bool) -> Result<(), String> {
    crate::overlay::set_clipboard_overlay_pinned(&app, pinned);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn hide_clipboard_overlay(app: AppHandle) -> Result<(), String> {
    crate::overlay::hide_clipboard_overlay(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn get_clipboard_stats(
    manager: State<'_, Arc<ClipboardManager>>,
) -> Result<ClipboardStats, String> {
    manager.get_stats().map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_clipboard_settings(
    app: AppHandle,
) -> Result<crate::managers::clipboard::ClipboardSettings, String> {
    let settings = crate::settings::get_settings(&app);
    Ok(crate::managers::clipboard::ClipboardSettings {
        max_records: settings.clipboard_max_records,
        hotkey: settings.clipboard_hotkey,
        confirm_mode: "copy".to_string(),
    })
}

#[tauri::command]
#[specta::specta]
pub async fn update_clipboard_settings(
    app: AppHandle,
    max_records: Option<usize>,
    hotkey: Option<String>,
    _confirm_mode: Option<String>,
) -> Result<crate::managers::clipboard::ClipboardSettings, String> {
    let mut settings = crate::settings::get_settings(&app);

    if let Some(max) = max_records {
        settings.clipboard_max_records = max;
    }
    if let Some(key) = hotkey {
        settings.clipboard_hotkey = key;
    }

    crate::settings::write_settings(&app, settings.clone());

    Ok(crate::managers::clipboard::ClipboardSettings {
        max_records: settings.clipboard_max_records,
        hotkey: settings.clipboard_hotkey,
        confirm_mode: "copy".to_string(),
    })
}

#[tauri::command]
#[specta::specta]
pub async fn toggle_clipboard_monitoring(
    app: AppHandle,
    manager: State<'_, Arc<ClipboardManager>>,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = crate::settings::get_settings(&app);
    settings.clipboard_enabled = enabled;
    crate::settings::write_settings(&app, settings);

    if enabled {
        manager.start_monitoring();
        if let Err(err) = manager.sync_current_clipboard() {
            error!(
                "Failed to sync clipboard after enabling monitoring: {}",
                err
            );
        }
        info!("Clipboard monitoring enabled");
    } else {
        // Note: Stopping monitoring would require a shutdown channel
        // For now, we just update the setting
        info!("Clipboard monitoring disabled (restart required to fully stop)");
    }

    Ok(())
}
