use crate::managers::clipboard::{ClipboardManager, ClipboardPageResult, ClipboardStats};
use log::{error, info};
use std::sync::Arc;
use tauri::{AppHandle, State};

fn clipboard_binding(
    settings: &crate::settings::AppSettings,
) -> Result<crate::settings::ShortcutBinding, String> {
    settings
        .bindings
        .get(crate::settings::CLIPBOARD_HISTORY_BINDING_ID)
        .cloned()
        .ok_or_else(|| "Clipboard history shortcut binding is missing".to_string())
}

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

/// 查看召回浮窗不等于开启采集；实际插入仍由既有目标校验保护。
#[tauri::command]
#[specta::specta]
pub fn show_clipboard_overlay(app: AppHandle) -> Result<(), String> {
    crate::overlay::show_clipboard_overlay(&app);
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
    if let Some(max) = max_records {
        change_clipboard_max_records_setting(app.clone(), max)?;
    }
    if let Some(key) = hotkey {
        change_clipboard_hotkey_setting(app.clone(), key).await?;
    }

    let settings = crate::settings::get_settings(&app);

    Ok(crate::managers::clipboard::ClipboardSettings {
        max_records: settings.clipboard_max_records,
        hotkey: settings.clipboard_hotkey,
        confirm_mode: "copy".to_string(),
    })
}

#[tauri::command]
#[specta::specta]
pub fn change_clipboard_enabled_setting(
    app: AppHandle,
    manager: State<'_, Arc<ClipboardManager>>,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = crate::settings::get_settings(&app);
    if settings.clipboard_enabled == enabled {
        return Ok(());
    }

    settings.clipboard_enabled = enabled;
    crate::settings::write_settings(&app, settings)?;
    crate::tray::update_tray_menu(&app);

    if enabled {
        manager.start_monitoring();
        if let Err(err) = manager.sync_current_clipboard() {
            error!(
                "Failed to sync clipboard after enabling experimental clipboard feature: {}",
                err
            );
        }
    }

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_clipboard_max_records_setting(
    app: AppHandle,
    max_records: usize,
) -> Result<(), String> {
    let mut settings = crate::settings::get_settings(&app);
    settings.clipboard_max_records = max_records;
    crate::settings::write_settings(&app, settings)?;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn change_clipboard_hotkey_enabled_setting(
    app: AppHandle,
    enabled: bool,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        change_clipboard_hotkey_enabled_setting_blocking(app, enabled)
    })
    .await
    .map_err(|_| "shortcut_settings_task_failed".to_owned())?
}

fn change_clipboard_hotkey_enabled_setting_blocking(
    app: AppHandle,
    enabled: bool,
) -> Result<(), String> {
    let _guard = crate::shortcut::settings_change_guard()?;
    let _idle = crate::shortcut::settings_barrier::SettingsBarrier::acquire(&app)?;
    let mut settings = crate::settings::get_settings(&app);
    let before = settings.clone();
    let previous = settings.clipboard_hotkey_enabled;
    if previous == enabled {
        return Ok(());
    }
    clipboard_binding(&settings)?;
    settings.clipboard_hotkey_enabled = enabled;
    let delta = crate::shortcut::settings_delta::RegistrationDelta::new(&before, &settings);
    super::settings_effects::save_and_apply(
        &app,
        settings,
        |saved| saved.clipboard_hotkey_enabled = previous,
        || delta.apply(&app),
        || delta.restore(&app),
        || crate::shortcut::suspend_for_settings_failure(&app),
    )
}

#[tauri::command]
#[specta::specta]
pub async fn change_clipboard_hotkey_setting(app: AppHandle, hotkey: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || change_clipboard_hotkey_setting_blocking(app, hotkey))
        .await
        .map_err(|_| "shortcut_settings_task_failed".to_owned())?
}

fn change_clipboard_hotkey_setting_blocking(app: AppHandle, hotkey: String) -> Result<(), String> {
    let _guard = crate::shortcut::settings_change_guard()?;
    let _idle = crate::shortcut::settings_barrier::SettingsBarrier::acquire(&app)?;
    let mut settings = crate::settings::get_settings(&app);
    crate::shortcut::validate_shortcut_for_implementation(
        &hotkey,
        settings.keyboard_implementation,
    )?;
    let before = settings.clone();
    let old_bindings = settings.bindings.clone();
    let old_hotkey = settings.clipboard_hotkey.clone();
    let mut new_binding = clipboard_binding(&settings)?;
    new_binding.current_binding = hotkey.clone();
    settings.clipboard_hotkey = hotkey;
    settings.bindings.insert(
        crate::settings::CLIPBOARD_HISTORY_BINDING_ID.into(),
        new_binding,
    );
    let delta = crate::shortcut::settings_delta::RegistrationDelta::new(&before, &settings);
    super::settings_effects::save_and_apply(
        &app,
        settings,
        |saved| {
            saved.bindings = old_bindings;
            saved.clipboard_hotkey = old_hotkey;
        },
        || delta.apply(&app),
        || delta.restore(&app),
        || crate::shortcut::suspend_for_settings_failure(&app),
    )
}

#[tauri::command]
#[specta::specta]
pub async fn toggle_clipboard_monitoring(
    app: AppHandle,
    manager: State<'_, Arc<ClipboardManager>>,
    enabled: bool,
) -> Result<(), String> {
    change_clipboard_enabled_setting(app, manager.clone(), enabled)?;
    info!(
        "Clipboard monitoring {}",
        if enabled { "enabled" } else { "disabled" }
    );

    Ok(())
}
