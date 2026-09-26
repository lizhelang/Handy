//! Keyboard shortcut management module
//!
//! This module provides a unified interface for keyboard shortcuts with
//! multiple backend implementations:
//!
//! - `tauri`: Uses Tauri's built-in global-shortcut plugin
//! - `handy_keys`: Uses the handy-keys library for more control
//!
//! The active implementation is determined by the `keyboard_implementation`
//! setting and can be changed at runtime.

mod cancel_registration;
mod handler;
pub mod handy_keys;
mod implementation_switch;
mod lifecycle_guard;
pub mod tauri_impl;

use log::{debug, error, warn};
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Emitter, Manager};

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use crate::settings::APPLE_INTELLIGENCE_DEFAULT_MODEL_ID;
use crate::settings::{
    self, get_settings, AutoSubmitKey, ClipboardHandling, KeyboardImplementation, LLMPrompt,
    OverlayPosition, OverlayStyle, PasteMethod, ShortcutActivation, ShortcutBinding, SoundTheme,
    Theme, TypingTool, VadBackend, APPLE_INTELLIGENCE_PROVIDER_ID,
};
use crate::tray;

// Note: Commands are accessed via shortcut::handy_keys:: in lib.rs

pub(crate) fn binding_enabled(settings: &settings::AppSettings, id: &str) -> bool {
    match id {
        "cancel" => false,
        "transcribe_with_post_process" => settings.post_process_enabled,
        settings::CLIPBOARD_HISTORY_BINDING_ID => {
            settings.clipboard_enabled && settings.clipboard_hotkey_enabled
        }
        _ => true,
    }
}

fn modifier_side_mask(
    modifiers: ::handy_keys::Modifiers,
    left: ::handy_keys::Modifiers,
    right: ::handy_keys::Modifiers,
) -> u8 {
    u8::from(modifiers.contains(left)) | (u8::from(modifiers.contains(right)) << 1)
}

/// Returns whether two HandyKeys shortcut patterns can both match the same
/// ordinary physical key chord. Compound modifiers accept either side, while
/// left- and right-specific modifiers remain distinct.
fn handy_keys_shortcuts_overlap(first: &str, second: &str) -> Result<bool, String> {
    let first = first
        .parse::<::handy_keys::Hotkey>()
        .map_err(|error| format!("Invalid HandyKeys shortcut '{first}': {error}"))?;
    let second = second
        .parse::<::handy_keys::Hotkey>()
        .map_err(|error| format!("Invalid HandyKeys shortcut '{second}': {error}"))?;

    if first.key != second.key {
        return Ok(false);
    }

    let modifier_groups = [
        (
            ::handy_keys::Modifiers::CMD_LEFT,
            ::handy_keys::Modifiers::CMD_RIGHT,
        ),
        (
            ::handy_keys::Modifiers::SHIFT_LEFT,
            ::handy_keys::Modifiers::SHIFT_RIGHT,
        ),
        (
            ::handy_keys::Modifiers::CTRL_LEFT,
            ::handy_keys::Modifiers::CTRL_RIGHT,
        ),
        (
            ::handy_keys::Modifiers::OPT_LEFT,
            ::handy_keys::Modifiers::OPT_RIGHT,
        ),
    ];

    for (left, right) in modifier_groups {
        let first_sides = modifier_side_mask(first.modifiers, left, right);
        let second_sides = modifier_side_mask(second.modifiers, left, right);
        if (first_sides == 0) != (second_sides == 0) {
            return Ok(false);
        }
        if first_sides != 0 && first_sides & second_sides == 0 {
            return Ok(false);
        }
    }

    Ok(first.modifiers.contains(::handy_keys::Modifiers::FN)
        == second.modifiers.contains(::handy_keys::Modifiers::FN))
}

fn find_handy_keys_conflict(
    settings: &settings::AppSettings,
    binding_id: &str,
    candidate: &str,
) -> Result<Option<String>, String> {
    if settings.keyboard_implementation != KeyboardImplementation::HandyKeys
        || !binding_enabled(settings, binding_id)
    {
        return Ok(None);
    }

    let mut active_ids = settings
        .bindings
        .keys()
        .filter(|id| id.as_str() != binding_id && binding_enabled(settings, id))
        .cloned()
        .collect::<Vec<_>>();
    active_ids.sort();

    for id in active_ids {
        let Some(binding) = settings.bindings.get(&id) else {
            continue;
        };
        if handy_keys_shortcuts_overlap(candidate, &binding.current_binding)? {
            return Ok(Some(id));
        }
    }

    Ok(None)
}

/// Repair the historical configuration that assigned the same physical chord
/// to plain and post-processed transcription. The post-processing chord wins;
/// plain transcription returns to its platform default.
fn repair_transcribe_post_process_overlap(
    settings: &mut settings::AppSettings,
) -> Result<bool, String> {
    if settings.keyboard_implementation != KeyboardImplementation::HandyKeys
        || !settings.post_process_enabled
    {
        return Ok(false);
    }

    let Some(plain) = settings.bindings.get("transcribe") else {
        return Ok(false);
    };
    let Some(post_process) = settings.bindings.get("transcribe_with_post_process") else {
        return Ok(false);
    };

    if !handy_keys_shortcuts_overlap(&plain.current_binding, &post_process.current_binding)? {
        return Ok(false);
    }

    let default_plain = plain.default_binding.clone();
    if handy_keys_shortcuts_overlap(&default_plain, &post_process.current_binding)? {
        return Err("Default transcription shortcut overlaps the post-processing shortcut".into());
    }

    settings
        .bindings
        .get_mut("transcribe")
        .expect("transcribe binding checked above")
        .current_binding = default_plain;
    Ok(true)
}

/// Initialize shortcuts using the configured implementation
pub fn init_shortcuts(app: &AppHandle) -> Result<(), String> {
    crate::input_permission::initializing_epoch()?;
    let mut user_settings = settings::load_or_create_app_settings(app);
    if user_settings.keyboard_implementation == KeyboardImplementation::HandyKeys {
        match repair_transcribe_post_process_overlap(&mut user_settings) {
            Ok(true) => {
                warn!(
                    "Resetting plain transcription shortcut to its default because it overlaps the post-processing shortcut"
                );
                settings::write_settings(app, user_settings.clone());
            }
            Ok(false) => {}
            Err(error) => error!("Failed to repair overlapping transcription shortcuts: {error}"),
        }
    }

    match user_settings.keyboard_implementation {
        KeyboardImplementation::Tauri => {
            tauri_impl::init_shortcuts(app)?;
            ::handy_keys::set_blocking_enabled(true);
            Ok(())
        }
        KeyboardImplementation::HandyKeys => handy_keys::init_shortcuts(app),
    }
}

/// Checks the actual current owner without admitting input or changing permission state.
pub fn health_ready(app: &AppHandle, epoch: u64) -> Result<(), String> {
    let result = match settings::get_settings(app).keyboard_implementation {
        KeyboardImplementation::HandyKeys => handy_keys::current(app)?.health_ready(epoch),
        KeyboardImplementation::Tauri => {
            use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};
            let selected = settings::get_settings(app);
            for (id, default) in settings::get_default_settings().bindings {
                if !binding_enabled(&selected, &id) {
                    continue;
                }
                let binding = selected.bindings.get(&id).unwrap_or(&default);
                let key = binding
                    .current_binding
                    .parse::<Shortcut>()
                    .map_err(|e| e.to_string())?;
                if !app.global_shortcut().is_registered(key) {
                    return Err(format!("快捷键 {} 尚未注册", id));
                }
            }
            Ok(())
        }
    };
    if result.is_ok() {
        crate::input_permission::mark_shortcuts_ready(epoch);
    }
    result
}

/// Bounded retirement. Unknown native cleanup keeps the old owner in place.
pub fn retire_shortcuts(app: &AppHandle) -> Result<(), String> {
    ::handy_keys::set_blocking_enabled(false);
    cancel_registration::set_active(app, false);
    crate::secure_input::reconcile_fallback_checked(app)?;
    tauri_impl::retire_all(app)?;
    handy_keys::retire_current(app)?;
    if ::handy_keys::active_listener_count() != 0 {
        return Err("原生键盘线程仍在退出".into());
    }
    Ok(())
}

/// Register the cancel shortcut (called when recording starts)
pub fn register_cancel_shortcut(app: &AppHandle) {
    cancel_registration::set_active(app, true);
}

/// Unregister the cancel shortcut (called when recording stops)
pub fn unregister_cancel_shortcut(app: &AppHandle) {
    cancel_registration::set_active(app, false);
}

/// Register a shortcut using the appropriate implementation
pub fn register_shortcut(app: &AppHandle, binding: ShortcutBinding) -> Result<(), String> {
    crate::input_permission::capture_epoch()?;
    let settings = get_settings(app);
    match settings.keyboard_implementation {
        KeyboardImplementation::Tauri => tauri_impl::register_shortcut(app, binding),
        KeyboardImplementation::HandyKeys => handy_keys::register_shortcut(app, binding),
    }
}

/// Unregister a shortcut using the appropriate implementation
pub fn unregister_shortcut(app: &AppHandle, binding: ShortcutBinding) -> Result<(), String> {
    let settings = get_settings(app);
    match settings.keyboard_implementation {
        KeyboardImplementation::Tauri => tauri_impl::unregister_shortcut(app, binding),
        KeyboardImplementation::HandyKeys => handy_keys::unregister_shortcut(app, binding),
    }
}

// ============================================================================
// Binding Management Commands
// ============================================================================

#[derive(Serialize, Type)]
pub struct BindingResponse {
    success: bool,
    binding: Option<ShortcutBinding>,
    error: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub fn change_binding(
    app: AppHandle,
    id: String,
    binding: String,
) -> Result<BindingResponse, String> {
    // Reject empty bindings — every shortcut should have a value
    if binding.trim().is_empty() {
        return Err("Binding cannot be empty".to_string());
    }

    let mut settings = settings::get_settings(&app);

    // Get the binding to modify, or create it from defaults if it doesn't exist
    let binding_to_modify = match settings.bindings.get(&id) {
        Some(binding) => binding.clone(),
        None => {
            // Try to get the default binding for this id
            let default_settings = settings::get_default_settings();
            match default_settings.bindings.get(&id) {
                Some(default_binding) => {
                    warn!(
                        "Binding '{}' not found in settings, creating from defaults",
                        id
                    );
                    default_binding.clone()
                }
                None => {
                    let error_msg = format!("Binding with id '{}' not found in defaults", id);
                    warn!("change_binding error: {}", error_msg);
                    return Ok(BindingResponse {
                        success: false,
                        binding: None,
                        error: Some(error_msg),
                    });
                }
            }
        }
    };

    // If this is the cancel binding, just update the settings and return
    // It's managed dynamically, so we don't register/unregister here
    if id == "cancel" {
        if let Some(mut b) = settings.bindings.get(&id).cloned() {
            b.current_binding = binding;
            settings.bindings.insert(id.clone(), b.clone());
            settings::write_settings(&app, settings);
            cancel_registration::refresh(&app);
            crate::secure_input::reconcile_fallback(&app);
            return Ok(BindingResponse {
                success: true,
                binding: Some(b.clone()),
                error: None,
            });
        }
    }

    // Validate the new shortcut for the current keyboard implementation
    if let Err(e) = validate_shortcut_for_implementation(&binding, settings.keyboard_implementation)
    {
        warn!("change_binding validation error: {}", e);
        return Err(e);
    }

    if let Some(conflict_id) = find_handy_keys_conflict(&settings, &id, &binding)? {
        let error_msg = format!(
            "Shortcut '{}' conflicts with active binding '{}'",
            binding, conflict_id
        );
        warn!("change_binding conflict: {error_msg}");
        return Ok(BindingResponse {
            success: false,
            binding: None,
            error: Some(error_msg),
        });
    }

    // Unregister the existing binding only after the replacement is known to be
    // valid and non-conflicting, so rejection leaves the current shortcut live.
    if let Err(e) = unregister_shortcut(&app, binding_to_modify.clone()) {
        let error_msg = format!("Failed to unregister shortcut: {}", e);
        error!("change_binding error: {}", error_msg);
    }

    // Create an updated binding
    let mut updated_binding = binding_to_modify.clone();
    updated_binding.current_binding = binding;

    // Register the new binding
    if let Err(e) = register_shortcut(&app, updated_binding.clone()) {
        let error_msg = format!("Failed to register shortcut: {}", e);
        error!("change_binding error: {}", error_msg);
        restore_registration(&app, &binding_to_modify);
        return Ok(BindingResponse {
            success: false,
            binding: None,
            error: Some(error_msg),
        });
    }

    // Update the binding in the settings
    settings.bindings.insert(id, updated_binding.clone());

    // Save the settings and synchronize any active Secure Input shadows.
    settings::write_settings(&app, settings);
    crate::secure_input::reconcile_fallback(&app);

    // Return the updated binding
    Ok(BindingResponse {
        success: true,
        binding: Some(updated_binding),
        error: None,
    })
}

/// Best-effort re-register of the previous binding after a failed change,
/// so a failure leaves the user's shortcut working exactly as before.
fn restore_registration(app: &AppHandle, binding: &ShortcutBinding) {
    if let Err(e) = register_shortcut(app, binding.clone()) {
        error!(
            "Failed to restore previous binding '{}' ({}): {}",
            binding.id, binding.current_binding, e
        );
    }
}

#[tauri::command]
#[specta::specta]
pub fn reset_binding(app: AppHandle, id: String) -> Result<BindingResponse, String> {
    let binding = settings::get_stored_binding(&app, &id);
    change_binding(app, id, binding.default_binding)
}

/// Unregister every binding while the user is recording a new shortcut in
/// the UI, so no existing shortcut can fire — or swallow the keystrokes —
/// mid-capture. The "cancel" binding is untouched: it is managed dynamically
/// by the recording lifecycle.
pub fn suspend_all_shortcuts(app: &AppHandle) {
    ::handy_keys::set_blocking_enabled(false);
    let settings = get_settings(app);
    for (id, binding) in &settings.bindings {
        if !binding_enabled(&settings, id) {
            continue;
        }
        if let Err(e) = unregister_shortcut(app, binding.clone()) {
            debug!(
                "suspend_all_shortcuts: could not unregister '{}': {}",
                id, e
            );
        }
    }
}

/// Re-register every binding from settings after shortcut recording ends.
/// Registering an already-registered shortcut fails cleanly in both
/// implementations, so this is idempotent and safe on every exit path.
pub fn resume_all_shortcuts(app: &AppHandle) {
    if crate::input_permission::capture_epoch().is_err() {
        return;
    }
    let settings = get_settings(app);
    for (id, binding) in &settings.bindings {
        if !binding_enabled(&settings, id) {
            continue;
        }
        if let Err(e) = register_shortcut(app, binding.clone()) {
            debug!("resume_all_shortcuts: could not register '{}': {}", id, e);
        }
    }
    if crate::input_permission::capture_epoch().is_ok() {
        ::handy_keys::set_blocking_enabled(true);
    }
}

/// Temporarily unregister all bindings while the user is recording a
/// shortcut in the UI. This avoids firing actions while keys are recorded.
#[tauri::command]
#[specta::specta]
pub fn suspend_all_bindings(app: AppHandle) -> Result<(), String> {
    suspend_all_shortcuts(&app);
    Ok(())
}

/// Re-register all bindings after the user has finished recording.
#[tauri::command]
#[specta::specta]
pub fn resume_all_bindings(app: AppHandle) -> Result<(), String> {
    resume_all_shortcuts(&app);
    Ok(())
}

// ============================================================================
// Keyboard Implementation Switching
// ============================================================================

/// Result of changing keyboard implementation
#[derive(Serialize, Type)]
pub struct ImplementationChangeResult {
    pub success: bool,
    /// List of binding IDs that were reset to defaults due to incompatibility
    pub reset_bindings: Vec<String>,
}

/// Change the keyboard implementation with runtime switching.
/// This will unregister all shortcuts from the old implementation,
/// validate shortcuts for the new implementation (resetting invalid ones to defaults),
/// and register them with the new implementation.
#[tauri::command]
#[specta::specta]
pub fn change_keyboard_implementation_setting(
    app: AppHandle,
    implementation: String,
) -> Result<ImplementationChangeResult, String> {
    let epoch = crate::input_permission::capture_epoch()?;
    let mut selected = settings::get_settings(&app);
    let current_impl = selected.keyboard_implementation;
    let new_impl = parse_keyboard_implementation(&implementation);
    if current_impl == new_impl {
        return Ok(ImplementationChangeResult {
            success: true,
            reset_bindings: vec![],
        });
    }
    app.manage(implementation_switch::ImplementationSwitch::default());
    let switch = app.state::<implementation_switch::ImplementationSwitch>();
    switch.begin(current_impl, new_impl)?;
    let result: Result<(), String> = (|| {
        // Close native dispatch while retiring. Permission state/epoch is unchanged.
        ::handy_keys::set_blocking_enabled(false);
        unregister_all_shortcuts(&app, current_impl)?;
        handy_keys::retire_current(&app)?;
        tauri_impl::retire_all(&app)?;
        crate::input_permission::check_epoch(epoch)?;
        if new_impl == KeyboardImplementation::HandyKeys {
            handy_keys::init_shortcuts(&app)?;
        } else {
            tauri_impl::init_shortcuts(&app)?;
        }
        crate::input_permission::check_epoch(epoch)?;
        ::handy_keys::set_blocking_enabled(true);
        selected.keyboard_implementation = new_impl;
        settings::write_settings(&app, selected);
        Ok(())
    })();
    switch.complete()?;
    if let Err(error) = result {
        crate::input_permission::close_gate(&error);
        return Err(error);
    }
    crate::secure_input::reconcile_fallback(&app);
    Ok(ImplementationChangeResult {
        success: true,
        reset_bindings: vec![],
    })
}

/// Get the current keyboard implementation
#[tauri::command]
#[specta::specta]
pub fn get_keyboard_implementation(app: AppHandle) -> String {
    let settings = settings::get_settings(&app);
    match settings.keyboard_implementation {
        KeyboardImplementation::Tauri => "tauri".to_string(),
        KeyboardImplementation::HandyKeys => "handy_keys".to_string(),
    }
}

// ============================================================================
// Validation Helpers
// ============================================================================

/// Validate a shortcut for a specific implementation
pub(crate) fn validate_shortcut_for_implementation(
    raw: &str,
    implementation: KeyboardImplementation,
) -> Result<(), String> {
    match implementation {
        KeyboardImplementation::Tauri => tauri_impl::validate_shortcut(raw),
        KeyboardImplementation::HandyKeys => handy_keys::validate_shortcut(raw),
    }
}

/// Parse a keyboard implementation string into the enum
fn parse_keyboard_implementation(s: &str) -> KeyboardImplementation {
    match s {
        "tauri" => KeyboardImplementation::Tauri,
        "handy_keys" => KeyboardImplementation::HandyKeys,
        other => {
            warn!(
                "Invalid keyboard implementation '{}', defaulting to tauri",
                other
            );
            KeyboardImplementation::Tauri
        }
    }
}

/// Unregister all shortcuts for the current implementation
fn unregister_all_shortcuts(
    app: &AppHandle,
    implementation: KeyboardImplementation,
) -> Result<(), String> {
    let settings = settings::get_settings(app);

    for (id, binding) in &settings.bindings {
        if !binding_enabled(&settings, id) {
            continue;
        }

        let result = match implementation {
            KeyboardImplementation::Tauri => tauri_impl::unregister_shortcut(app, binding.clone()),
            KeyboardImplementation::HandyKeys => {
                handy_keys::unregister_shortcut(app, binding.clone())
            }
        };

        result?;
    }
    Ok(())
}

// ============================================================================
// General Settings Commands
// ============================================================================

#[tauri::command]
#[specta::specta]
pub fn change_shortcut_activation_setting(
    app: AppHandle,
    activation: ShortcutActivation,
) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.shortcut_activation = activation;
    settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_hold_threshold_ms_setting(app: AppHandle, ms: u64) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.hold_threshold_ms = ms;
    settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_audio_feedback_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.audio_feedback = enabled;
    settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_audio_feedback_volume_setting(app: AppHandle, volume: f32) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.audio_feedback_volume = volume;
    settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_sound_theme_setting(app: AppHandle, theme: String) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    let parsed = match theme.as_str() {
        "marimba" => SoundTheme::Marimba,
        "pop" => SoundTheme::Pop,
        "custom" => SoundTheme::Custom,
        other => {
            warn!("Invalid sound theme '{}', defaulting to marimba", other);
            SoundTheme::Marimba
        }
    };
    settings.sound_theme = parsed;
    settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_theme_setting(app: AppHandle, theme: String) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    let parsed = match theme.as_str() {
        "system" => Theme::System,
        "light" => Theme::Light,
        "dark" => Theme::Dark,
        other => {
            warn!("Invalid theme '{}', defaulting to system", other);
            Theme::System
        }
    };
    settings.theme = parsed;
    settings::write_settings(&app, settings);
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    apply_window_theme(&app, parsed);
    // Notify other webviews (the recording overlay) so they re-apply the palette
    // live — they set `data-theme` on their own document and can't see this one.
    let _ = app.emit("theme-changed", parsed);
    Ok(())
}

/// Applies the appearance setting to the native window chrome (title bar), which
/// CSS `data-theme` cannot reach. `System` clears the override so the window
/// follows the OS. Call this on startup and whenever the setting changes to keep
/// the title bar in sync with the in-app palette.
///
/// On Windows this themes the title bar only. On macOS `set_theme` sets
/// `NSApp.appearance` app-wide, which is what we want here: it darkens the title
/// bar and keeps the overlay in step. Linux is left to `data-theme` alone, since
/// its window theming is backend-dependent and unreliable.
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub fn apply_window_theme(app: &AppHandle, theme: Theme) {
    let window_theme = match theme {
        Theme::System => None,
        Theme::Light => Some(tauri::Theme::Light),
        Theme::Dark => Some(tauri::Theme::Dark),
    };
    if let Some(window) = app.get_webview_window("main") {
        if let Err(e) = window.set_theme(window_theme) {
            warn!("Failed to apply window theme: {}", e);
        }
    }
}

#[tauri::command]
#[specta::specta]
pub fn change_translate_to_english_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.translate_to_english = enabled;
    settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_selected_language_setting(app: AppHandle, language: String) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.selected_language = language;
    settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_overlay_position_setting(app: AppHandle, position: String) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    let parsed = match position.as_str() {
        // "none" is retired (visibility is overlay_style now); fold legacy callers
        // onto Bottom rather than warn.
        "none" | "bottom" => OverlayPosition::Bottom,
        "top" => OverlayPosition::Top,
        other => {
            warn!("Invalid overlay position '{}', defaulting to bottom", other);
            OverlayPosition::Bottom
        }
    };
    settings.overlay_position = parsed;
    settings::write_settings(&app, settings);

    // Whether the overlay shows at all is owned by overlay_style now; position
    // only ever toggles Top/Bottom, so the enabled cache is untouched here.
    // Update overlay position without recreating window
    crate::utils::update_overlay_position(&app);

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_overlay_style_setting(app: AppHandle, style: String) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    let parsed = match style.as_str() {
        "none" => OverlayStyle::None,
        "minimal" => OverlayStyle::Minimal,
        "live" => OverlayStyle::Live,
        other => {
            warn!("Invalid overlay style '{}', defaulting to minimal", other);
            OverlayStyle::Minimal
        }
    };
    settings.overlay_style = parsed;
    settings::write_settings(&app, settings);

    // Keep the cached overlay-enabled flag in sync so emit_levels stops (or
    // resumes) emitting on the next audio callback.
    crate::overlay::update_overlay_enabled_cache(parsed != OverlayStyle::None);

    // Reposition in case the window needs to re-center for the new style.
    crate::utils::update_overlay_position(&app);

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_debug_mode_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.debug_mode = enabled;
    settings::write_settings(&app, settings);

    // Keep webview log streaming in sync: the live log viewer only exists in
    // debug mode, so logs are forwarded to the frontend only while it is on.
    crate::WEBVIEW_LOG_STREAMING.store(enabled, std::sync::atomic::Ordering::Relaxed);

    // Emit event to notify frontend of debug mode change
    let _ = app.emit(
        "settings-changed",
        serde_json::json!({
            "setting": "debug_mode",
            "value": enabled
        }),
    );

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_start_hidden_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.start_hidden = enabled;
    settings::write_settings(&app, settings);

    // Notify frontend
    let _ = app.emit(
        "settings-changed",
        serde_json::json!({
            "setting": "start_hidden",
            "value": enabled
        }),
    );

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_autostart_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.autostart_enabled = enabled;
    settings::write_settings(&app, settings);

    // Apply the autostart setting immediately
    crate::autostart::apply_autostart(&app, enabled);

    // Notify frontend
    let _ = app.emit(
        "settings-changed",
        serde_json::json!({
            "setting": "autostart_enabled",
            "value": enabled
        }),
    );

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_update_checks_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    if settings::update_checks_forced_disabled() {
        return Err(
            "Update checks are disabled by system configuration (HANDY_DISABLE_UPDATER)".into(),
        );
    }

    let mut settings = settings::get_settings(&app);
    settings.update_checks_enabled = enabled;
    settings::write_settings(&app, settings);

    let _ = app.emit(
        "settings-changed",
        serde_json::json!({
            "setting": "update_checks_enabled",
            "value": enabled
        }),
    );

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_show_whats_new_on_update_setting(
    app: AppHandle,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.show_whats_new_on_update = enabled;
    settings::write_settings(&app, settings);

    let _ = app.emit(
        "settings-changed",
        serde_json::json!({
            "setting": "show_whats_new_on_update",
            "value": enabled
        }),
    );

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_whats_new_last_seen_version_setting(
    app: AppHandle,
    version: String,
) -> Result<(), String> {
    let version = version.trim().to_string();
    let mut settings = settings::get_settings(&app);
    settings.whats_new_last_seen_version = version.clone();
    settings::write_settings(&app, settings);

    let _ = app.emit(
        "settings-changed",
        serde_json::json!({
            "setting": "whats_new_last_seen_version",
            "value": version
        }),
    );

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn update_custom_words(app: AppHandle, words: Vec<String>) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.custom_words = words;
    settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_word_correction_threshold_setting(
    app: AppHandle,
    threshold: f64,
) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.word_correction_threshold = threshold;
    settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_extra_recording_buffer_setting(app: AppHandle, ms: u64) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.extra_recording_buffer_ms = ms;
    settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_paste_delay_ms_setting(app: AppHandle, ms: u64) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.paste_delay_ms = ms;
    settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_paste_delay_after_ms_setting(app: AppHandle, ms: u64) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.paste_delay_after_ms = ms;
    settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_reliable_paste_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.reliable_paste = enabled;
    settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_paste_method_setting(app: AppHandle, method: String) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    let parsed = match method.as_str() {
        "ctrl_v" => PasteMethod::CtrlV,
        "direct" => PasteMethod::Direct,
        "none" => PasteMethod::None,
        "shift_insert" => PasteMethod::ShiftInsert,
        "ctrl_shift_v" => PasteMethod::CtrlShiftV,
        "external_script" => PasteMethod::ExternalScript,
        other => {
            warn!("Invalid paste method '{}', defaulting to ctrl_v", other);
            PasteMethod::CtrlV
        }
    };
    settings.paste_method = parsed;
    settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn get_available_typing_tools() -> Vec<String> {
    #[cfg(target_os = "linux")]
    {
        crate::clipboard::get_available_typing_tools()
    }
    #[cfg(not(target_os = "linux"))]
    {
        vec!["auto".to_string()]
    }
}

#[tauri::command]
#[specta::specta]
pub fn change_typing_tool_setting(app: AppHandle, tool: String) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    let parsed = match tool.as_str() {
        "auto" => TypingTool::Auto,
        "wtype" => TypingTool::Wtype,
        "kwtype" => TypingTool::Kwtype,
        "dotool" => TypingTool::Dotool,
        "ydotool" => TypingTool::Ydotool,
        "xdotool" => TypingTool::Xdotool,
        other => {
            warn!("Invalid typing tool '{}', defaulting to auto", other);
            TypingTool::Auto
        }
    };
    settings.typing_tool = parsed;
    settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_external_script_path_setting(
    app: AppHandle,
    path: Option<String>,
) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.external_script_path = path;
    settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_clipboard_handling_setting(app: AppHandle, handling: String) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    let parsed = match handling.as_str() {
        "dont_modify" => ClipboardHandling::DontModify,
        "copy_to_clipboard" => ClipboardHandling::CopyToClipboard,
        other => {
            warn!(
                "Invalid clipboard handling '{}', defaulting to dont_modify",
                other
            );
            ClipboardHandling::DontModify
        }
    };
    settings.clipboard_handling = parsed;
    settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_auto_submit_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.auto_submit = enabled;
    settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_auto_submit_key_setting(app: AppHandle, key: String) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    let parsed = match key.as_str() {
        "enter" => AutoSubmitKey::Enter,
        "ctrl_enter" => AutoSubmitKey::CtrlEnter,
        "cmd_enter" => AutoSubmitKey::CmdEnter,
        other => {
            warn!("Invalid auto submit key '{}', defaulting to enter", other);
            AutoSubmitKey::Enter
        }
    };
    settings.auto_submit_key = parsed;
    settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_post_process_enabled_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    if settings.post_process_enabled == enabled {
        return Ok(());
    }

    settings.post_process_enabled = enabled;
    let old_plain_binding = settings.bindings.get("transcribe").cloned();
    let repaired_plain_binding = if enabled {
        repair_transcribe_post_process_overlap(&mut settings)?
    } else {
        false
    };
    let new_plain_binding = settings.bindings.get("transcribe").cloned();
    let post_process_binding = settings
        .bindings
        .get("transcribe_with_post_process")
        .cloned();

    if enabled {
        if repaired_plain_binding {
            let old_plain = old_plain_binding
                .as_ref()
                .ok_or("Plain transcription shortcut is missing")?;
            let new_plain = new_plain_binding
                .as_ref()
                .ok_or("Plain transcription shortcut is missing after repair")?;
            unregister_shortcut(&app, old_plain.clone())?;
            if let Err(error) = register_shortcut(&app, new_plain.clone()) {
                restore_registration(&app, old_plain);
                return Err(error);
            }
        }

        if let Some(binding) = post_process_binding.clone() {
            if let Err(error) = register_shortcut(&app, binding) {
                if repaired_plain_binding {
                    if let (Some(old_plain), Some(new_plain)) =
                        (old_plain_binding.as_ref(), new_plain_binding.as_ref())
                    {
                        let _ = unregister_shortcut(&app, new_plain.clone());
                        restore_registration(&app, old_plain);
                    }
                }
                return Err(error);
            }
        }
    } else if let Some(binding) = post_process_binding {
        unregister_shortcut(&app, binding)?;
    }

    settings::write_settings(&app, settings);

    if repaired_plain_binding {
        let _ = app.emit(
            "settings-changed",
            serde_json::json!({
                "setting": "bindings",
                "reason": "transcription_shortcut_overlap_repaired"
            }),
        );
        warn!(
            "Reset plain transcription shortcut to its default while enabling post-processing because the shortcuts overlapped"
        );
    }

    crate::secure_input::reconcile_fallback(&app);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_experimental_enabled_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.experimental_enabled = enabled;
    settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_post_process_base_url_setting(
    app: AppHandle,
    provider_id: String,
    base_url: String,
) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    let label = settings
        .post_process_provider(&provider_id)
        .map(|provider| provider.label.clone())
        .ok_or_else(|| format!("Provider '{}' not found", provider_id))?;

    let provider = settings
        .post_process_provider_mut(&provider_id)
        .expect("Provider looked up above must exist");

    if provider.id != "custom" {
        return Err(format!(
            "Provider '{}' does not allow editing the base URL",
            label
        ));
    }

    provider.base_url = base_url;
    settings::write_settings(&app, settings);
    Ok(())
}

/// Generic helper to validate provider exists
fn validate_provider_exists(
    settings: &settings::AppSettings,
    provider_id: &str,
) -> Result<(), String> {
    if !settings
        .post_process_providers
        .iter()
        .any(|provider| provider.id == provider_id)
    {
        return Err(format!("Provider '{}' not found", provider_id));
    }
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_post_process_api_key_setting(
    app: AppHandle,
    provider_id: String,
    api_key: String,
) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    validate_provider_exists(&settings, &provider_id)?;
    settings.post_process_api_keys.insert(provider_id, api_key);
    settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_post_process_model_setting(
    app: AppHandle,
    provider_id: String,
    model: String,
) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    validate_provider_exists(&settings, &provider_id)?;
    settings.post_process_models.insert(provider_id, model);
    settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn set_post_process_provider(app: AppHandle, provider_id: String) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    validate_provider_exists(&settings, &provider_id)?;
    settings.post_process_provider_id = provider_id;
    settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn add_post_process_prompt(
    app: AppHandle,
    name: String,
    prompt: String,
) -> Result<LLMPrompt, String> {
    let mut settings = settings::get_settings(&app);

    // Generate unique ID using timestamp and random component
    let id = format!("prompt_{}", chrono::Utc::now().timestamp_millis());

    let new_prompt = LLMPrompt {
        id: id.clone(),
        name,
        prompt,
    };

    settings.post_process_prompts.push(new_prompt.clone());
    settings::write_settings(&app, settings);

    Ok(new_prompt)
}

#[tauri::command]
#[specta::specta]
pub fn update_post_process_prompt(
    app: AppHandle,
    id: String,
    name: String,
    prompt: String,
) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);

    if let Some(existing_prompt) = settings
        .post_process_prompts
        .iter_mut()
        .find(|p| p.id == id)
    {
        existing_prompt.name = name;
        existing_prompt.prompt = prompt;
        settings::write_settings(&app, settings);
        Ok(())
    } else {
        Err(format!("Prompt with id '{}' not found", id))
    }
}

#[tauri::command]
#[specta::specta]
pub fn delete_post_process_prompt(app: AppHandle, id: String) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);

    // Don't allow deleting the last prompt
    if settings.post_process_prompts.len() <= 1 {
        return Err("Cannot delete the last prompt".to_string());
    }

    // Find and remove the prompt
    let original_len = settings.post_process_prompts.len();
    settings.post_process_prompts.retain(|p| p.id != id);

    if settings.post_process_prompts.len() == original_len {
        return Err(format!("Prompt with id '{}' not found", id));
    }

    // If the deleted prompt was selected, select the first one or None
    if settings.post_process_selected_prompt_id.as_ref() == Some(&id) {
        settings.post_process_selected_prompt_id =
            settings.post_process_prompts.first().map(|p| p.id.clone());
    }

    settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn fetch_post_process_models(
    app: AppHandle,
    provider_id: String,
) -> Result<Vec<String>, String> {
    let settings = settings::get_settings(&app);

    // Find the provider
    let provider = settings
        .post_process_providers
        .iter()
        .find(|p| p.id == provider_id)
        .ok_or_else(|| format!("Provider '{}' not found", provider_id))?;

    if provider.id == APPLE_INTELLIGENCE_PROVIDER_ID {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            return Ok(vec![APPLE_INTELLIGENCE_DEFAULT_MODEL_ID.to_string()]);
        }

        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        {
            return Err("Apple Intelligence is only available on Apple silicon Macs running macOS 15 or later.".to_string());
        }
    }

    // Get API key
    let api_key = settings
        .post_process_api_keys
        .get(&provider_id)
        .cloned()
        .unwrap_or_default();

    // Skip fetching if no API key for providers that typically need one
    if api_key.trim().is_empty() && provider.id != "custom" {
        return Err(format!(
            "API key is required for {}. Please add an API key to list available models.",
            provider.label
        ));
    }

    crate::llm_client::fetch_models(provider, api_key).await
}

#[tauri::command]
#[specta::specta]
pub fn set_post_process_selected_prompt(app: AppHandle, id: String) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);

    // Verify the prompt exists
    if !settings.post_process_prompts.iter().any(|p| p.id == id) {
        return Err(format!("Prompt with id '{}' not found", id));
    }

    settings.post_process_selected_prompt_id = Some(id);
    settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_mute_while_recording_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.mute_while_recording = enabled;
    settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_append_trailing_space_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.append_trailing_space = enabled;
    settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_lazy_stream_close_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.lazy_stream_close = enabled;
    settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_vad_enabled_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.vad_enabled = enabled;
    settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn change_vad_backend_setting(app: AppHandle, backend: VadBackend) -> Result<(), String> {
    if settings::get_settings(&app).vad_backend == backend {
        return Ok(());
    }

    // Construct/swap the detector and, when necessary, reopen cpal away from
    // the webview thread. Persist only after the runtime change succeeds so a
    // rejected in-progress switch or failed microphone reopen rolls back cleanly.
    let manager = app
        .state::<std::sync::Arc<crate::managers::audio::AudioRecordingManager>>()
        .inner()
        .clone();
    tokio::task::spawn_blocking(move || manager.update_vad_backend(backend))
        .await
        .map_err(|e| format!("audio task join failed: {e}"))?
        .map_err(|e| format!("Failed to update VAD backend: {e}"))?;

    let mut current_settings = settings::get_settings(&app);
    current_settings.vad_backend = backend;
    settings::write_settings(&app, current_settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_filler_word_removal_enabled_setting(
    app: AppHandle,
    enabled: bool,
) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.filler_word_removal_enabled = enabled;
    settings::write_settings(&app, settings);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_app_language_setting(app: AppHandle, language: String) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.app_language = language.clone();
    settings::write_settings(&app, settings);

    // Refresh the tray menu with the new language
    tray::update_tray_menu(&app);

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_show_tray_icon_setting(app: AppHandle, enabled: bool) -> Result<(), String> {
    let mut settings = settings::get_settings(&app);
    settings.show_tray_icon = tray::independent_tray_enabled(enabled, false);
    settings::write_settings(&app, settings);

    // Apply change immediately
    tray::set_tray_visibility(&app, false);

    Ok(())
}

/// Save accelerator settings and make the next model use reload with them.
/// The currently running transcription, if any, keeps its existing engine.
fn save_accelerator_and_reload_next_use(app: &AppHandle, s: settings::AppSettings) {
    settings::write_settings(app, s);

    let tm = app.state::<std::sync::Arc<crate::managers::transcription::TranscriptionManager>>();
    tm.reload_model_on_next_use();
}

#[tauri::command]
#[specta::specta]
pub fn change_transcribe_accelerator_setting(
    app: AppHandle,
    accelerator: settings::TranscribeAcceleratorSetting,
) -> Result<(), String> {
    let mut s = settings::get_settings(&app);
    s.transcribe_accelerator = accelerator;
    save_accelerator_and_reload_next_use(&app, s);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_ort_accelerator_setting(
    app: AppHandle,
    accelerator: settings::OrtAcceleratorSetting,
) -> Result<(), String> {
    let mut s = settings::get_settings(&app);
    s.ort_accelerator = accelerator;
    save_accelerator_and_reload_next_use(&app, s);
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub fn change_transcribe_gpu_device(app: AppHandle, device: Option<String>) -> Result<(), String> {
    let mut s = settings::get_settings(&app);
    s.transcribe_gpu_device = device;
    save_accelerator_and_reload_next_use(&app, s);
    Ok(())
}

/// Return which accelerators and GPU devices are available for this build.
///
/// First-call cost is dominated by enumerating GPU devices through the
/// transcribe.cpp Metal/Vulkan backend, which loads dynamic libraries and
/// probes hardware. Run it on the blocking pool so the webview thread
/// stays responsive — see also the startup pre-warm in `lib.rs`.
#[tauri::command]
#[specta::specta]
pub async fn get_available_accelerators() -> crate::managers::transcription::AvailableAccelerators {
    tauri::async_runtime::spawn_blocking(crate::managers::transcription::get_available_accelerators)
        .await
        .expect("get_available_accelerators panicked")
}

#[cfg(test)]
mod tests {
    use super::{
        binding_enabled, find_handy_keys_conflict, handy_keys_shortcuts_overlap,
        repair_transcribe_post_process_overlap,
    };
    use crate::settings::{
        get_default_settings, KeyboardImplementation, CLIPBOARD_HISTORY_BINDING_ID,
    };

    #[test]
    fn clipboard_binding_requires_feature_and_hotkey_toggles() {
        let mut settings = get_default_settings();
        assert!(!binding_enabled(&settings, CLIPBOARD_HISTORY_BINDING_ID));

        settings.clipboard_enabled = true;
        assert!(!binding_enabled(&settings, CLIPBOARD_HISTORY_BINDING_ID));

        settings.clipboard_hotkey_enabled = true;
        assert!(binding_enabled(&settings, CLIPBOARD_HISTORY_BINDING_ID));
    }

    #[test]
    fn handy_keys_generic_modifiers_overlap_side_specific_modifiers() {
        assert!(
            handy_keys_shortcuts_overlap("option+shift+space", "option_left+shift_left+space")
                .unwrap()
        );
        assert!(handy_keys_shortcuts_overlap(
            "option+shift+space",
            "option_right+shift_right+space"
        )
        .unwrap());
    }

    #[test]
    fn handy_keys_distinct_sides_and_keys_do_not_overlap() {
        assert!(!handy_keys_shortcuts_overlap(
            "option_left+shift_left+space",
            "option_right+shift_right+space"
        )
        .unwrap());
        assert!(!handy_keys_shortcuts_overlap("option+shift+space", "option+space").unwrap());
        assert!(!handy_keys_shortcuts_overlap("option+shift+space", "option+shift+k").unwrap());
    }

    #[test]
    fn active_handy_keys_binding_conflict_is_reported() {
        let mut settings = get_default_settings();
        settings.keyboard_implementation = KeyboardImplementation::HandyKeys;
        settings.post_process_enabled = true;
        settings
            .bindings
            .get_mut("transcribe")
            .unwrap()
            .current_binding = "option_left+shift_left+space".into();

        let conflict =
            find_handy_keys_conflict(&settings, "transcribe", "option_left+shift_left+space")
                .unwrap();

        assert_eq!(conflict.as_deref(), Some("transcribe_with_post_process"));
    }

    #[test]
    fn startup_repair_preserves_post_process_and_resets_plain_transcribe() {
        let mut settings = get_default_settings();
        settings.keyboard_implementation = KeyboardImplementation::HandyKeys;
        settings.post_process_enabled = true;
        settings
            .bindings
            .get_mut("transcribe")
            .unwrap()
            .current_binding = "option_left+shift_left+space".into();
        let post_process_before = settings.bindings["transcribe_with_post_process"]
            .current_binding
            .clone();

        assert!(repair_transcribe_post_process_overlap(&mut settings).unwrap());
        assert_eq!(
            settings.bindings["transcribe"].current_binding,
            settings.bindings["transcribe"].default_binding
        );
        assert_eq!(
            settings.bindings["transcribe_with_post_process"].current_binding,
            post_process_before
        );
    }

    #[test]
    fn startup_repair_leaves_disabled_post_process_binding_untouched() {
        let mut settings = get_default_settings();
        settings.keyboard_implementation = KeyboardImplementation::HandyKeys;
        settings.post_process_enabled = false;
        settings
            .bindings
            .get_mut("transcribe")
            .unwrap()
            .current_binding = "option_left+shift_left+space".into();

        assert!(!repair_transcribe_post_process_overlap(&mut settings).unwrap());
        assert_eq!(
            settings.bindings["transcribe"].current_binding,
            "option_left+shift_left+space"
        );
    }
}
