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
        // 原生快捷键回调可能来自后台线程；Carbon 输入源查询只能在主线程执行。
        // 主线程只探测输入源；服务查询和动作保持在同一后台队列中处理。
        let dispatched_app = app.clone();
        let binding_id = binding_id.to_owned();
        let hotkey_string = hotkey_string.to_owned();
        if app
            .run_on_main_thread(move || {
                enqueue_native_shortcut(NativeShortcutEvent {
                    app: dispatched_app,
                    binding_id,
                    hotkey_string,
                    is_pressed,
                    generation,
                    source: crate::host_shortcut_broker::current_input_source(),
                });
            })
            .is_err()
        {
            warn!("Shortcut main-thread dispatch unavailable; event not replayed");
        }
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
    #[cfg(target_os = "macos")] source: crate::host_shortcut_broker::CurrentInputSource,
) {
    let Some(_lease) = super::settings_barrier::admit(generation, is_pressed) else {
        return;
    };
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
        match route_inputia_host_shortcut(app, binding_id, hotkey_string, is_pressed, source) {
            crate::host_shortcut_broker::ShortcutRouting::Legacy => {}
            crate::host_shortcut_broker::ShortcutRouting::Forwarded => return,
            crate::host_shortcut_broker::ShortcutRouting::HostPending => {
                if let Some(coordinator) = app.try_state::<TranscriptionCoordinator>() {
                    coordinator.send_legacy_continuation(
                        binding_id,
                        hotkey_string,
                        is_pressed,
                        settings.shortcut_activation,
                        std::time::Duration::from_millis(settings.hold_threshold_ms),
                    );
                }
                return;
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
    source: crate::host_shortcut_broker::CurrentInputSource,
) -> crate::host_shortcut_broker::ShortcutRouting {
    let Some(broker) = app.try_state::<crate::host_shortcut_broker::HostShortcutBroker>() else {
        return crate::host_shortcut_broker::ShortcutRouting::HostPending;
    };
    let policy_epoch = app
        .try_state::<Arc<crate::managers::integration::IntegrationManager>>()
        .and_then(|manager| manager.service.policy_epoch().ok());
    let settings = get_settings(app);
    broker.route_shortcut_event(
        binding_id,
        hotkey_string,
        is_pressed,
        settings.shortcut_activation,
        std::time::Duration::from_millis(settings.hold_threshold_ms),
        policy_epoch,
        source,
    )
}

#[cfg(target_os = "macos")]
struct NativeShortcutEvent {
    app: AppHandle,
    binding_id: String,
    hotkey_string: String,
    is_pressed: bool,
    generation: u64,
    source: crate::host_shortcut_broker::CurrentInputSource,
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
                        event.source,
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
