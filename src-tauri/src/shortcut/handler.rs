//! Shared shortcut event handling logic
//!
//! This module contains the common logic for handling shortcut events,
//! used by both the Tauri and handy-keys implementations.

use log::warn;
use std::sync::Arc;
use tauri::{AppHandle, Manager};

use crate::actions::ACTION_MAP;
use crate::managers::audio::AudioRecordingManager;
use crate::settings::get_settings;
use crate::transcription_coordinator::is_transcribe_binding;
use crate::TranscriptionCoordinator;

/// Handle a shortcut event from either implementation.
///
/// This function contains the shared logic for:
/// - Looking up the action in ACTION_MAP
/// - Handling the cancel binding (only fires when recording)
/// - Routing transcribe bindings to the coordinator, which applies the
///   configured activation mode (toggle / push-to-talk / hold-or-toggle)
///
/// # Arguments
/// * `app` - The Tauri app handle
/// * `binding_id` - The ID of the binding (e.g., "transcribe", "cancel")
/// * `hotkey_string` - The string representation of the hotkey
/// * `is_pressed` - Whether this is a key press (true) or release (false)
pub fn handle_shortcut_event(
    app: &AppHandle,
    binding_id: &str,
    hotkey_string: &str,
    is_pressed: bool,
) {
    let settings = get_settings(app);

    // Transcribe bindings are handled by the coordinator.
    if is_transcribe_binding(binding_id) {
        if let Some(coordinator) = app.try_state::<TranscriptionCoordinator>() {
            if coordinator.voice_output_context().is_some_and(|request| {
                matches!(
                    request.command,
                    inputia_handy_runtime::voice_protocol::VoiceCommand::Start { .. }
                )
            }) {
                coordinator.send_input(
                    binding_id,
                    hotkey_string,
                    is_pressed,
                    settings.shortcut_activation,
                    std::time::Duration::from_millis(settings.hold_threshold_ms),
                );
                return;
            }
        }
        #[cfg(target_os = "macos")]
        if let Some(routing) =
            route_inputia_host_shortcut(app, binding_id, hotkey_string, is_pressed)
        {
            match routing {
                crate::host_shortcut_broker::ShortcutRouting::Legacy => {}
                crate::host_shortcut_broker::ShortcutRouting::Forwarded => return,
                crate::host_shortcut_broker::ShortcutRouting::HostPending => return,
            }
        }
        if let Some(coordinator) = app.try_state::<TranscriptionCoordinator>() {
            coordinator.send_input(
                binding_id,
                hotkey_string,
                is_pressed,
                settings.shortcut_activation,
                std::time::Duration::from_millis(settings.hold_threshold_ms),
            );
        } else {
            warn!("TranscriptionCoordinator is not initialized");
        }
        return;
    }

    let Some(action) = ACTION_MAP.get(binding_id) else {
        warn!(
            "No action defined in ACTION_MAP for shortcut ID '{}'. Shortcut: '{}', Pressed: {}",
            binding_id, hotkey_string, is_pressed
        );
        return;
    };

    // Cancel binding: only fires when recording and key is pressed
    if binding_id == "cancel" {
        let audio_manager = app.state::<Arc<AudioRecordingManager>>();
        if audio_manager.is_recording() && is_pressed {
            action.start(app, binding_id, hotkey_string);
        }
        return;
    }

    // Remaining bindings (e.g. "test") use simple start/stop on press/release.
    if is_pressed {
        action.start(app, binding_id, hotkey_string);
    } else {
        action.stop(app, binding_id, hotkey_string);
    }
}

#[cfg(target_os = "macos")]
fn route_inputia_host_shortcut(
    app: &AppHandle,
    binding_id: &str,
    hotkey_string: &str,
    is_pressed: bool,
) -> Option<crate::host_shortcut_broker::ShortcutRouting> {
    let broker = app.try_state::<crate::host_shortcut_broker::HostShortcutBroker>()?;
    let manager = app.try_state::<Arc<crate::managers::integration::IntegrationManager>>()?;
    let policy_epoch = manager.service.policy_epoch().ok()?;
    let settings = get_settings(app);
    Some(broker.route_shortcut_event(
        binding_id,
        hotkey_string,
        is_pressed,
        settings.shortcut_activation,
        std::time::Duration::from_millis(settings.hold_threshold_ms),
        policy_epoch,
    ))
}
