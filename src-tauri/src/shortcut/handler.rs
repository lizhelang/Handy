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
    let Some(generation) = super::settings_barrier::capture(is_pressed) else {
        return;
    };
    #[cfg(target_os = "macos")]
    {
        // 普通全局快捷键固定进入本地听写，不因输入源或 Host 租约改变路线。
        enqueue_native_shortcut(NativeShortcutEvent {
            app: app.clone(),
            binding_id: binding_id.to_owned(),
            hotkey_string: hotkey_string.to_owned(),
            is_pressed,
            generation,
            occurred_at: std::time::Instant::now(),
        });
    }
    #[cfg(not(target_os = "macos"))]
    handle_shortcut_event_on_dispatch_thread(
        app,
        binding_id,
        hotkey_string,
        is_pressed,
        generation,
    );
}

fn handle_shortcut_event_on_dispatch_thread(
    app: &AppHandle,
    binding_id: &str,
    hotkey_string: &str,
    is_pressed: bool,
    generation: u64,
    #[cfg(target_os = "macos")] occurred_at: std::time::Instant,
) {
    let Some(_lease) = super::settings_barrier::admit(generation, is_pressed) else {
        return;
    };
    let settings = get_settings(app);

    // Transcribe bindings are handled by the coordinator.
    if is_transcribe_binding(binding_id) {
        if let Some(coordinator) = app.try_state::<TranscriptionCoordinator>() {
            #[cfg(target_os = "macos")]
            {
                coordinator.send_dictation_input(
                    binding_id,
                    hotkey_string,
                    is_pressed,
                    settings.shortcut_activation,
                    std::time::Duration::from_millis(settings.hold_threshold_ms),
                    occurred_at,
                );
            }
            #[cfg(not(target_os = "macos"))]
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
struct NativeShortcutEvent {
    app: AppHandle,
    binding_id: String,
    hotkey_string: String,
    is_pressed: bool,
    generation: u64,
    occurred_at: std::time::Instant,
}

#[cfg(target_os = "macos")]
fn enqueue_native_shortcut(event: NativeShortcutEvent) {
    use std::sync::{mpsc, OnceLock};
    static QUEUE: OnceLock<Option<mpsc::Sender<NativeShortcutEvent>>> = OnceLock::new();
    let queue = QUEUE.get_or_init(|| {
        let (sender, receiver) = mpsc::channel::<NativeShortcutEvent>();
        std::thread::Builder::new()
            .name("inputia-shortcut-dispatch".into())
            .spawn(move || {
                for event in receiver {
                    handle_shortcut_event_on_dispatch_thread(
                        &event.app,
                        &event.binding_id,
                        &event.hotkey_string,
                        event.is_pressed,
                        event.generation,
                        event.occurred_at,
                    );
                }
            })
            .ok()
            .map(|_| sender)
    });
    if queue
        .as_ref()
        .is_none_or(|queue| queue.send(event).is_err())
    {
        warn!("Shortcut worker unavailable; event not replayed");
    }
}
