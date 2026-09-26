//! Tauri global-shortcut implementation
//!
//! This module provides shortcut functionality using Tauri's built-in
//! global-shortcut plugin.

use log::{debug, error, warn};
use tauri::AppHandle;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::settings::{self, ShortcutBinding};

use super::handler::handle_shortcut_event;

/// Initialize shortcuts using Tauri's global-shortcut plugin
pub fn init_shortcuts(app: &AppHandle) -> Result<(), String> {
    crate::input_permission::initializing_epoch()?;
    let default_bindings = settings::get_default_settings().bindings;
    let user_settings = settings::load_or_create_app_settings(app);

    // Register all default shortcuts, applying user customizations
    for (id, default_binding) in default_bindings {
        if !super::binding_enabled(&user_settings, &id) {
            continue;
        }
        let binding = user_settings
            .bindings
            .get(&id)
            .cloned()
            .unwrap_or(default_binding);

        register_shortcut(app, binding)?;
    }
    Ok(())
}

/// Validate a shortcut string for the Tauri global-shortcut implementation.
/// Tauri requires at least one non-modifier key and doesn't support the fn key.
pub fn validate_shortcut(raw: &str) -> Result<(), String> {
    if raw.trim().is_empty() {
        return Err("Shortcut cannot be empty".into());
    }

    let modifiers = [
        "ctrl", "control", "shift", "alt", "option", "meta", "command", "cmd", "super", "win",
        "windows",
    ];

    // Check for fn key which Tauri doesn't support
    let parts: Vec<String> = raw.split('+').map(|p| p.trim().to_lowercase()).collect();
    for part in &parts {
        if part == "fn" || part == "function" {
            return Err("The 'fn' key is not supported by Tauri global shortcuts".into());
        }
    }

    // Check for at least one non-modifier key
    let has_non_modifier = parts.iter().any(|part| !modifiers.contains(&part.as_str()));

    if has_non_modifier {
        Ok(())
    } else {
        Err("Tauri shortcuts must include a main key (letter, number, F-key, etc.) in addition to modifiers".into())
    }
}

/// Register a shortcut using Tauri's global-shortcut plugin
pub fn register_shortcut(app: &AppHandle, binding: ShortcutBinding) -> Result<(), String> {
    let epoch = crate::input_permission::initializing_epoch()?;
    let handle = app.clone();
    on_main(app, move || {
        if crate::input_permission::initializing_epoch() != Ok(epoch) {
            return Err("Carbon 注册请求已过期".into());
        }
        register_on_main(&handle, binding, epoch)
    })
}

fn register_on_main(app: &AppHandle, binding: ShortcutBinding, epoch: u64) -> Result<(), String> {
    // Validate for Tauri requirements
    if let Err(e) = validate_shortcut(&binding.current_binding) {
        warn!(
            "register_tauri_shortcut validation error for binding '{}': {}",
            binding.current_binding, e
        );
        return Err(e);
    }

    // Parse shortcut and return error if it fails
    let shortcut = match binding.current_binding.parse::<Shortcut>() {
        Ok(s) => s,
        Err(e) => {
            let error_msg = format!(
                "Failed to parse shortcut '{}': {}",
                binding.current_binding, e
            );
            error!("register_tauri_shortcut parse error: {}", error_msg);
            return Err(error_msg);
        }
    };

    // Prevent duplicate registrations that would silently shadow one another
    if app.global_shortcut().is_registered(shortcut) {
        let error_msg = format!("Shortcut '{}' is already in use", binding.current_binding);
        warn!("register_tauri_shortcut duplicate error: {}", error_msg);
        return Err(error_msg);
    }

    // Clone binding.id for use in the closure
    let binding_id_for_closure = binding.id.clone();

    app.global_shortcut()
        .on_shortcut(shortcut, move |app_handle, scut, event| {
            if ::handy_keys::events_allowed()
                && crate::input_permission::check_epoch(epoch).is_ok()
                && scut == &shortcut
            {
                let shortcut_string = scut.into_string();
                let is_pressed = event.state == ShortcutState::Pressed;
                // Mirrors the handy-keys event log line; the distinct prefix
                // makes it possible to tell which backend fired a shortcut
                // (e.g. when diagnosing the Secure Input fallback)
                debug!(
                    "tauri global-shortcut event: binding={}, shortcut={}, state={:?}",
                    binding_id_for_closure, shortcut_string, event.state
                );
                handle_shortcut_event(
                    app_handle,
                    &binding_id_for_closure,
                    &shortcut_string,
                    is_pressed,
                );
            }
        })
        .map_err(|e| {
            let error_msg = format!(
                "Couldn't register shortcut '{}': {}",
                binding.current_binding, e
            );
            error!("register_tauri_shortcut registration error: {}", error_msg);
            error_msg
        })?;

    Ok(())
}

/// Unregister a shortcut from Tauri's global-shortcut plugin
pub fn unregister_shortcut(app: &AppHandle, binding: ShortcutBinding) -> Result<(), String> {
    let handle = app.clone();
    on_main(app, move || unregister_on_main(&handle, binding))
}

fn unregister_on_main(app: &AppHandle, binding: ShortcutBinding) -> Result<(), String> {
    let shortcut = match binding.current_binding.parse::<Shortcut>() {
        Ok(s) => s,
        Err(e) => {
            let error_msg = format!(
                "Failed to parse shortcut '{}' for unregistration: {}",
                binding.current_binding, e
            );
            error!("unregister_tauri_shortcut parse error: {}", error_msg);
            return Err(error_msg);
        }
    };

    app.global_shortcut().unregister(shortcut).map_err(|e| {
        let error_msg = format!(
            "Failed to unregister shortcut '{}': {}",
            binding.current_binding, e
        );
        error!("unregister_tauri_shortcut error: {}", error_msg);
        error_msg
    })?;

    Ok(())
}

/// Carbon work is dispatched with a bounded receipt; a timeout is unknown, not cancellation.
fn on_main(
    app: &AppHandle,
    operation: impl FnOnce() -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    if objc2::MainThreadMarker::new().is_some() {
        return operation();
    }
    #[cfg(not(target_os = "macos"))]
    return operation();
    #[cfg(target_os = "macos")]
    {
        let (tx, rx) = std::sync::mpsc::channel();
        app.run_on_main_thread(move || {
            let _ = tx.send(operation());
        })
        .map_err(|e| e.to_string())?;
        rx.recv_timeout(std::time::Duration::from_millis(500))
            .map_err(|_| {
                crate::input_permission::close_gate("Carbon 注册或注销回执未知");
                "Carbon 注册或注销回执未知".to_owned()
            })?
    }
}

pub fn retire_all(app: &AppHandle) -> Result<(), String> {
    let handle = app.clone();
    on_main(app, move || {
        handle
            .global_shortcut()
            .unregister_all()
            .map_err(|e| e.to_string())
    })
}
