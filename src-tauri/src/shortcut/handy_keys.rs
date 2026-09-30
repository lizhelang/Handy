//! Handy-keys based keyboard shortcut implementation
//!
//! This module provides an alternative to Tauri's global-shortcut plugin
//! using the handy-keys library for more control over keyboard events.
//!
//! ## Architecture
//!
//! The implementation uses a dedicated manager thread that owns the `HotkeyManager`:
//!
//! ```text
//! ┌─────────────────┐     commands      ┌──────────────────────┐
//! │   Main Thread   │ ───────────────▶ │   Manager Thread     │
//! │                 │   (via channel)   │                      │
//! │ - register()    │                   │ - owns HotkeyManager │
//! │ - unregister()  │                   │ - polls for events   │
//! └─────────────────┘                   │ - dispatches actions │
//!                                       └──────────────────────┘
//! ```
//!
//! This design ensures thread-safety since `HotkeyManager` is only accessed
//! from a single thread. Commands (register/unregister) are sent via an mpsc
//! channel and responses are synchronously awaited.
//!
//! ## Recording Mode
//!
//! For UI key capture, a separate `KeyboardListener` is created on-demand and
//! polled from a dedicated recording thread. Events are emitted to the frontend
//! via Tauri's event system.

use handy_keys::{Hotkey, HotkeyId, HotkeyManager, HotkeyState, KeyboardListener};
use log::{debug, error, info};
use serde::Serialize;
use specta::Type;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use tauri::{AppHandle, Emitter, Manager};

use crate::settings::{self, get_settings, ShortcutBinding};

use super::handler::handle_shortcut_event;

/// Commands that can be sent to the hotkey manager thread
enum ManagerCommand {
    Register {
        epoch: u64,
        binding_id: String,
        hotkey_string: String,
        response: Sender<Result<(), String>>,
    },
    Unregister {
        binding_id: String,
        response: Sender<Result<(), String>>,
    },
    Health {
        response: Sender<Result<(), String>>,
    },
    Shutdown,
}

/// State for the handy-keys shortcut manager
pub struct HandyKeysState {
    healthy: Arc<AtomicBool>,
    ready: Mutex<Receiver<Result<(), String>>>,
    recording_thread: Mutex<Option<JoinHandle<()>>>,
    epoch: u64,
    /// Channel to send commands to the manager thread (wrapped in Mutex for Sync)
    command_sender: Mutex<Sender<ManagerCommand>>,
    /// Handle to the manager thread (wrapped in Mutex for Sync, allows proper join on drop)
    thread_handle: Mutex<Option<JoinHandle<()>>>,
    /// Recording listener for UI key capture (only active during recording)
    recording_listener: Mutex<Option<KeyboardListener>>,
    /// Flag indicating if we're in recording mode
    is_recording: AtomicBool,
    /// The binding ID being recorded (if any)
    recording_binding_id: Mutex<Option<String>>,
    /// Flag to stop recording loop
    recording_running: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct HandyKeysSlot(Mutex<Option<Arc<HandyKeysState>>>);

pub fn current(app: &AppHandle) -> Result<Arc<HandyKeysState>, String> {
    let slot = app.try_state::<HandyKeysSlot>().ok_or("快捷键尚未初始化")?;
    let worker = slot.0.try_lock().map_err(|_| "快捷键生命周期忙碌")?;
    worker.clone().ok_or_else(|| "快捷键尚未初始化".into())
}

pub fn retire_current(app: &AppHandle) -> Result<(), String> {
    if app.try_state::<HandyKeysSlot>().is_none() {
        return Ok(());
    }
    current(app)?.retire()
}

/// Key event sent to frontend during recording mode
#[derive(Debug, Clone, Serialize, Type)]
pub struct FrontendKeyEvent {
    capture_token: String,
    /// Currently pressed modifier keys
    pub modifiers: Vec<String>,
    /// The key that was pressed (if any)
    pub key: Option<String>,
    /// Whether this is a key down event
    pub is_key_down: bool,
    /// The full hotkey string (e.g., "option+space")
    pub hotkey_string: String,
}

impl HandyKeysState {
    /// Create a new HandyKeysState
    pub fn new(app: AppHandle) -> Result<Self, String> {
        let epoch = crate::input_permission::initializing_epoch()?;
        handy_keys::set_permission_check(crate::input_permission::callback_allowed);
        let healthy = Arc::new(AtomicBool::new(true));
        let worker_healthy = healthy.clone();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (cmd_tx, cmd_rx) = mpsc::channel::<ManagerCommand>();

        // Start the manager thread
        let app_clone = app.clone();
        let thread_handle = thread::spawn(move || {
            Self::manager_thread(cmd_rx, app_clone, epoch, &worker_healthy, ready_tx);
            if worker_healthy.swap(false, Ordering::AcqRel) {
                crate::input_permission::mark_shortcuts_failed(epoch, "快捷键原生监听器意外退出");
            }
        });

        Ok(Self {
            healthy,
            ready: Mutex::new(ready_rx),
            recording_thread: Mutex::new(None),
            epoch,
            command_sender: Mutex::new(cmd_tx),
            thread_handle: Mutex::new(Some(thread_handle)),
            recording_listener: Mutex::new(None),
            is_recording: AtomicBool::new(false),
            recording_binding_id: Mutex::new(None),
            recording_running: Arc::new(AtomicBool::new(false)),
        })
    }

    /// The main manager thread - owns the HotkeyManager and processes commands
    fn manager_thread(
        cmd_rx: Receiver<ManagerCommand>,
        app: AppHandle,
        epoch: u64,
        healthy: &AtomicBool,
        ready: Sender<Result<(), String>>,
    ) {
        info!("handy-keys manager thread started");

        // Create the HotkeyManager in this thread
        let manager = match HotkeyManager::new_with_blocking() {
            Ok(m) => m,
            Err(e) => {
                let _ = ready.send(Err(format!("Failed to create HotkeyManager: {e}")));
                error!("Failed to create HotkeyManager: {}", e);
                return;
            }
        };

        let _ = ready.send(Ok(()));

        // Maps binding IDs to HotkeyIds and hotkey strings
        let mut binding_to_hotkey: HashMap<String, HotkeyId> = HashMap::new();
        let mut hotkey_to_binding: HashMap<HotkeyId, (String, String)> = HashMap::new(); // (binding_id, hotkey_string)

        loop {
            if !healthy.load(Ordering::Acquire)
                || !manager.is_healthy()
                || crate::input_permission::initializing_epoch() != Ok(epoch)
            {
                break;
            }
            // Check for hotkey events (non-blocking)
            while let Some(event) = manager.try_recv() {
                if !handy_keys::events_allowed()
                    || crate::input_permission::check_epoch(epoch).is_err()
                {
                    continue;
                }
                if let Some((binding_id, hotkey_string)) = hotkey_to_binding.get(&event.id) {
                    debug!(
                        "handy-keys event: binding={}, hotkey={}, state={:?}",
                        binding_id, hotkey_string, event.state
                    );
                    let is_pressed = event.state == HotkeyState::Pressed;
                    handle_shortcut_event(&app, binding_id, hotkey_string, is_pressed);
                }
            }

            // Check for commands (non-blocking with timeout)
            match cmd_rx.recv_timeout(std::time::Duration::from_millis(10)) {
                Ok(cmd) => match cmd {
                    ManagerCommand::Register {
                        epoch: request_epoch,
                        binding_id,
                        hotkey_string,
                        response,
                    } => {
                        if !super::lifecycle_guard::registration_current(
                            request_epoch,
                            epoch,
                            healthy.load(Ordering::Acquire),
                            crate::input_permission::initializing_epoch(),
                        ) {
                            let _ = response.send(Err("快捷键注册请求已过期".into()));
                            continue;
                        }
                        let result = Self::do_register(
                            &manager,
                            &mut binding_to_hotkey,
                            &mut hotkey_to_binding,
                            &binding_id,
                            &hotkey_string,
                        );
                        let _ = response.send(result);
                    }
                    ManagerCommand::Unregister {
                        binding_id,
                        response,
                    } => {
                        let result = Self::do_unregister(
                            &manager,
                            &mut binding_to_hotkey,
                            &mut hotkey_to_binding,
                            &binding_id,
                        );
                        let _ = response.send(result);
                    }
                    ManagerCommand::Health { response } => {
                        let result = if manager.is_healthy() && healthy.load(Ordering::Acquire) {
                            Ok(())
                        } else {
                            Err("快捷键原生监听器不可用".into())
                        };
                        let _ = response.send(result);
                    }
                    ManagerCommand::Shutdown => {
                        info!("handy-keys manager thread shutting down");
                        break;
                    }
                },
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // No command, continue
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    info!("Command channel disconnected, shutting down");
                    break;
                }
            }
        }

        info!("handy-keys manager thread stopped");
    }

    /// Register a hotkey
    fn do_register(
        manager: &HotkeyManager,
        binding_to_hotkey: &mut HashMap<String, HotkeyId>,
        hotkey_to_binding: &mut HashMap<HotkeyId, (String, String)>,
        binding_id: &str,
        hotkey_string: &str,
    ) -> Result<(), String> {
        if binding_to_hotkey.contains_key(binding_id) {
            return Err(format!("Binding '{binding_id}' is already registered"));
        }

        for (registered_id, registered_hotkey) in hotkey_to_binding.values() {
            if super::handy_keys_shortcuts_overlap(hotkey_string, registered_hotkey)? {
                return Err(format!(
                    "Shortcut '{hotkey_string}' conflicts with registered binding '{registered_id}'"
                ));
            }
        }

        let hotkey: Hotkey = hotkey_string
            .parse()
            .map_err(|e| format!("Failed to parse hotkey '{}': {}", hotkey_string, e))?;

        let id = manager
            .register(hotkey)
            .map_err(|e| format!("Failed to register hotkey: {}", e))?;

        binding_to_hotkey.insert(binding_id.to_string(), id);
        hotkey_to_binding.insert(id, (binding_id.to_string(), hotkey_string.to_string()));

        debug!(
            "Registered handy-keys shortcut: {} -> {:?}",
            binding_id, hotkey
        );
        Ok(())
    }

    /// Unregister a hotkey
    fn do_unregister(
        manager: &HotkeyManager,
        binding_to_hotkey: &mut HashMap<String, HotkeyId>,
        hotkey_to_binding: &mut HashMap<HotkeyId, (String, String)>,
        binding_id: &str,
    ) -> Result<(), String> {
        if let Some(id) = binding_to_hotkey.remove(binding_id) {
            manager
                .unregister(id)
                .map_err(|e| format!("Failed to unregister hotkey: {}", e))?;
            hotkey_to_binding.remove(&id);
            debug!("Unregistered handy-keys shortcut: {}", binding_id);
        }
        Ok(())
    }

    fn fault(&self, reason: &str) -> String {
        self.healthy.store(false, Ordering::Release);
        handy_keys::set_blocking_enabled(false);
        crate::input_permission::mark_shortcuts_failed(self.epoch, reason);
        reason.to_owned()
    }

    pub fn health_ready(&self, epoch: u64) -> Result<(), String> {
        if self.epoch != epoch || !self.healthy.load(Ordering::Acquire) {
            return Err("快捷键监听器不可用或已过期".into());
        }
        let (tx, rx) = mpsc::channel();
        self.command_sender
            .try_lock()
            .map_err(|_| "快捷键健康检查忙碌")?
            .send(ManagerCommand::Health { response: tx })
            .map_err(|_| "快捷键健康检查通道掉线")?;
        super::lifecycle_guard::receipt(rx, std::time::Duration::from_millis(500))?
    }

    pub fn await_ready(&self) -> Result<(), String> {
        self.ready
            .try_lock()
            .map_err(|_| "快捷键初始化仍在进行")?
            .recv_timeout(std::time::Duration::from_secs(2))
            .map_err(|_| self.fault("快捷键监听器启动超时或已退出"))?
    }

    pub fn retire(&self) -> Result<(), String> {
        handy_keys::set_blocking_enabled(false);
        self.healthy.store(false, Ordering::Release);
        self.stop_recording()?;
        if let Ok(sender) = self.command_sender.try_lock() {
            let _ = sender.send(ManagerCommand::Shutdown);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_millis(500);
        loop {
            let mut thread = self
                .thread_handle
                .try_lock()
                .map_err(|_| "快捷键线程状态忙碌")?;
            if thread.as_ref().is_none_or(|h| h.is_finished())
                && handy_keys::active_listener_count() == 0
            {
                if let Some(handle) = thread.take() {
                    let _ = handle.join();
                }
                return Ok(());
            }
            drop(thread);
            if std::time::Instant::now() >= deadline {
                return Err("旧快捷键监听器尚未退出，不能创建第二套".into());
            }
            thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// Register a shortcut binding
    pub fn register(&self, binding: &ShortcutBinding) -> Result<(), String> {
        let epoch = crate::input_permission::initializing_epoch()?;
        if epoch != self.epoch || !self.healthy.load(Ordering::Acquire) {
            return Err("快捷键监听器不可用".into());
        }
        let (tx, rx) = mpsc::channel();
        self.command_sender
            .try_lock()
            .map_err(|_| "Failed to lock command_sender")?
            .send(ManagerCommand::Register {
                epoch,
                binding_id: binding.id.clone(),
                hotkey_string: binding.current_binding.clone(),
                response: tx,
            })
            .map_err(|_| self.fault("快捷键注册通道掉线"))?;

        super::lifecycle_guard::receipt(rx, std::time::Duration::from_millis(500))
            .map_err(|_| self.fault("快捷键注册回执超时或通道掉线，结果未知"))?
    }

    /// Unregister a shortcut binding
    pub fn unregister(&self, binding: &ShortcutBinding) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        self.command_sender
            .try_lock()
            .map_err(|_| "Failed to lock command_sender")?
            .send(ManagerCommand::Unregister {
                binding_id: binding.id.clone(),
                response: tx,
            })
            .map_err(|_| self.fault("快捷键注销通道掉线"))?;

        super::lifecycle_guard::receipt(rx, std::time::Duration::from_millis(500))
            .map_err(|_| self.fault("快捷键注销回执超时或通道掉线，结果未知"))?
    }

    /// Start recording mode for a specific binding
    pub fn start_recording(
        &self,
        app: &AppHandle,
        binding_id: String,
        capture_token: String,
    ) -> Result<(), String> {
        crate::input_permission::check_epoch(self.epoch)?;
        if self.is_recording.load(Ordering::SeqCst) {
            return Err("Already recording".into());
        }
        self.stop_recording()?;

        // Create a new keyboard listener for recording
        let listener = KeyboardListener::new()
            .map_err(|e| format!("Failed to create keyboard listener: {}", e))?;

        {
            let mut recording = self
                .recording_listener
                .lock()
                .map_err(|_| "Failed to lock recording_listener")?;
            *recording = Some(listener);
        }
        {
            let mut binding = self
                .recording_binding_id
                .lock()
                .map_err(|_| "Failed to lock recording_binding_id")?;
            *binding = Some(binding_id);
        }

        self.is_recording.store(true, Ordering::SeqCst);
        self.recording_running.store(true, Ordering::SeqCst);

        // Start a thread to emit key events to the frontend
        let app_clone = app.clone();
        let recording_running = Arc::clone(&self.recording_running);
        let epoch = self.epoch;
        *self
            .recording_thread
            .try_lock()
            .map_err(|_| "快捷键录制线程忙碌")? = Some(thread::spawn(move || {
            Self::recording_loop(app_clone, recording_running, epoch, capture_token);
        }));

        debug!("Started handy-keys recording mode");
        Ok(())
    }

    /// Recording loop - emits key events to frontend during recording
    fn recording_loop(app: AppHandle, running: Arc<AtomicBool>, epoch: u64, capture_token: String) {
        while running.load(Ordering::SeqCst) && crate::input_permission::check_epoch(epoch).is_ok()
        {
            let event = {
                let state = match current(&app) {
                    Ok(s) => s,
                    Err(_) => break,
                };
                let listener = state.recording_listener.lock().ok();
                listener.as_ref().and_then(|l| l.as_ref()?.try_recv())
            };

            if let Some(key_event) = event {
                // Convert to frontend-friendly format
                let frontend_event = FrontendKeyEvent {
                    capture_token: capture_token.clone(),
                    modifiers: modifiers_to_strings(key_event.modifiers),
                    key: key_event.key.map(|k| k.to_string().to_lowercase()),
                    is_key_down: key_event.is_key_down,
                    hotkey_string: key_event
                        .as_hotkey()
                        .map(|h| h.to_handy_string())
                        .unwrap_or_default(),
                };

                if crate::input_permission::check_epoch(epoch).is_err() {
                    break;
                }
                // Emit to frontend
                if let Err(e) = app.emit("handy-keys-event", &frontend_event) {
                    error!("Failed to emit key event: {}", e);
                }
            } else {
                thread::sleep(std::time::Duration::from_millis(10));
            }
        }

        debug!("Recording loop ended");
    }

    /// Stop recording mode
    pub fn stop_recording(&self) -> Result<(), String> {
        self.is_recording.store(false, Ordering::SeqCst);
        self.recording_running.store(false, Ordering::SeqCst);

        {
            let mut recording = self
                .recording_listener
                .lock()
                .map_err(|_| "Failed to lock recording_listener")?;
            if let Some(listener) = recording.as_mut() {
                if !listener.stop_and_wait(std::time::Duration::from_millis(150)) {
                    return Err("旧录制监听器尚未退出".into());
                }
            }
            *recording = None;
        }
        {
            let mut binding = self
                .recording_binding_id
                .lock()
                .map_err(|_| "Failed to lock recording_binding_id")?;
            *binding = None;
        }

        let mut worker = self
            .recording_thread
            .try_lock()
            .map_err(|_| "快捷键录制线程忙碌")?;
        if let Some(handle) = worker.as_ref() {
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(100);
            while !handle.is_finished() && std::time::Instant::now() < deadline {
                thread::sleep(std::time::Duration::from_millis(5));
            }
            if !handle.is_finished() {
                return Err("旧快捷键录制线程尚未退出".into());
            }
        }
        if let Some(handle) = worker.take() {
            let _ = handle.join();
        }
        debug!("Stopped handy-keys recording mode");
        Ok(())
    }
}

impl Drop for HandyKeysState {
    fn drop(&mut self) {
        // Signal recording to stop
        self.recording_running.store(false, Ordering::SeqCst);
        self.is_recording.store(false, Ordering::SeqCst);

        // Send shutdown command
        if let Ok(sender) = self.command_sender.lock() {
            let _ = sender.send(ManagerCommand::Shutdown);
        }

        // Wait for the manager thread to finish
        if let Ok(mut handle) = self.thread_handle.lock() {
            if let Some(h) = handle.take() {
                if h.is_finished() {
                    let _ = h.join();
                }
            }
        }
    }
}

/// Convert handy-keys Modifiers to a list of strings
fn modifiers_to_strings(modifiers: handy_keys::Modifiers) -> Vec<String> {
    let mut result = Vec::new();

    if modifiers.contains(handy_keys::Modifiers::CTRL) {
        result.push("ctrl".to_string());
    }
    if modifiers.contains(handy_keys::Modifiers::OPT) {
        #[cfg(target_os = "macos")]
        result.push("option".to_string());
        #[cfg(not(target_os = "macos"))]
        result.push("alt".to_string());
    }
    if modifiers.contains(handy_keys::Modifiers::SHIFT) {
        result.push("shift".to_string());
    }
    if modifiers.contains(handy_keys::Modifiers::CMD) {
        #[cfg(target_os = "macos")]
        result.push("command".to_string());
        #[cfg(not(target_os = "macos"))]
        result.push("super".to_string());
    }
    if modifiers.contains(handy_keys::Modifiers::FN) {
        result.push("fn".to_string());
    }

    result
}

/// Validate a shortcut string for the HandyKeys implementation.
/// HandyKeys is more permissive: allows modifier-only combos and the fn key.
pub fn validate_shortcut(raw: &str) -> Result<(), String> {
    if raw.trim().is_empty() {
        return Err("Shortcut cannot be empty".into());
    }
    // HandyKeys accepts modifier-only, key-only, and modifier+key combos
    // Just verify the string is parseable
    raw.parse::<Hotkey>()
        .map(|_| ())
        .map_err(|e| format!("Invalid shortcut for HandyKeys: {}", e))
}

/// Initialize handy-keys shortcuts
pub fn init_shortcuts(app: &AppHandle) -> Result<(), String> {
    app.manage(HandyKeysSlot::default());
    let slot = app.state::<HandyKeysSlot>();
    let mut owned = slot.0.try_lock().map_err(|_| "快捷键生命周期忙碌")?;
    if let Some(old) = owned.as_ref() {
        old.retire()?;
    }
    if handy_keys::active_listener_count() != 0 {
        return Err("旧原生键盘线程尚未退出".into());
    }
    handy_keys::set_blocking_enabled(false);
    let state = Arc::new(HandyKeysState::new(app.clone())?);
    *owned = Some(state.clone());
    state.await_ready()?;
    let user_settings = settings::load_or_create_app_settings(app);
    for (id, default_binding) in settings::get_default_settings().bindings {
        if !super::binding_enabled(&user_settings, &id) {
            continue;
        }
        let binding = user_settings
            .bindings
            .get(&id)
            .cloned()
            .unwrap_or(default_binding);
        state.register(&binding)?;
    }
    if crate::input_permission::initializing_epoch() != Ok(state.epoch) {
        return Err(state.fault("快捷键初始化已过期"));
    }
    state.health_ready(state.epoch)?;
    handy_keys::set_blocking_enabled(true);
    info!("handy-keys shortcuts initialized");
    Ok(())
}

/// Register a shortcut
pub fn register_shortcut(app: &AppHandle, binding: ShortcutBinding) -> Result<(), String> {
    let state = current(app)?;
    state.register(&binding)
}

/// Unregister a shortcut
pub fn unregister_shortcut(app: &AppHandle, binding: ShortcutBinding) -> Result<(), String> {
    let state = current(app)?;
    state.unregister(&binding)
}

/// 原生录制和 Webview 录制使用同一 token/空闲屏障。
#[tauri::command]
#[specta::specta]
pub async fn start_handy_keys_recording(
    app: AppHandle,
    binding_id: String,
) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        super::capture::begin(
            &app,
            binding_id,
            settings::KeyboardImplementation::HandyKeys,
        )
    })
    .await
    .map_err(|_| "shortcut_capture_task_failed".to_owned())?
}

#[tauri::command]
#[specta::specta]
pub async fn stop_handy_keys_recording(app: AppHandle, token: String) -> Result<bool, String> {
    tokio::task::spawn_blocking(move || super::capture::end(&app, token))
        .await
        .map_err(|_| "shortcut_capture_task_failed".to_owned())?
}
