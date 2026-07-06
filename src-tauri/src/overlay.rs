use crate::input;
use crate::settings;
use crate::settings::{OverlayPosition, OverlayStyle};
use log::debug;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindowBuilder};

#[cfg(target_os = "macos")]
use tauri::WebviewUrl;

#[cfg(target_os = "macos")]
use tauri_nspanel::{tauri_panel, CollectionBehavior, PanelBuilder, PanelLevel, StyleMask};

#[cfg(target_os = "linux")]
use gtk_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

#[cfg(target_os = "linux")]
use std::env;

#[cfg(target_os = "macos")]
tauri_panel! {
    panel!(RecordingOverlayPanel {
        config: {
            can_become_key_window: false,
            is_floating_panel: true
        }
    })
}

// Native overlay window sizes (logical points). One window is reused for every
// state and resized in `show_overlay_state`; each size need only be at least as
// large as the card it hosts (the `--ov-*` vars in RecordingOverlay.css). The
// card is CSS-anchored flush to the screen edge, so window height doesn't move
// where the card sits — only OVERLAY_TOP_OFFSET / OVERLAY_BOTTOM_OFFSET do. Keep
// these in sync with the CSS card geometry.
//
// Compact overlay (Minimal / transcribing / processing): the 40h pill animates
// width from 172 (--ov-rest-w) to 216 (--ov-work-w) and expands from center, so
// the window must fit the widest state plus a little slack.
const OVERLAY_WIDTH: f64 = 256.0;
const OVERLAY_HEIGHT: f64 = 46.0;

// Actual is 394x118, just a little extra
const OVERLAY_STREAM_WIDTH: f64 = 400.0;
const OVERLAY_STREAM_HEIGHT: f64 = 120.0;
const CLIPBOARD_OVERLAY_WIDTH: f64 = 400.0;
const CLIPBOARD_OVERLAY_HEIGHT: f64 = 550.0;

#[cfg(target_os = "macos")]
static CLIPBOARD_OVERLAY_PINNED: AtomicBool = AtomicBool::new(false);

#[cfg(target_os = "macos")]
static CLIPBOARD_OVERLAY_FOCUSED: AtomicBool = AtomicBool::new(false);

/// Overlay window size (logical) for a given UI state.
fn overlay_dimensions(state: &str) -> (f64, f64) {
    if state == "streaming" {
        (OVERLAY_STREAM_WIDTH, OVERLAY_STREAM_HEIGHT)
    } else {
        (OVERLAY_WIDTH, OVERLAY_HEIGHT)
    }
}

#[cfg(target_os = "macos")]
const OVERLAY_TOP_OFFSET: f64 = 46.0;
#[cfg(any(target_os = "windows", target_os = "linux"))]
const OVERLAY_TOP_OFFSET: f64 = 4.0;

#[cfg(target_os = "macos")]
const OVERLAY_BOTTOM_OFFSET: f64 = 15.0;

#[cfg(any(target_os = "windows", target_os = "linux"))]
const OVERLAY_BOTTOM_OFFSET: f64 = 40.0;

#[cfg(target_os = "linux")]
fn update_gtk_layer_shell_anchors(overlay_window: &tauri::webview::WebviewWindow) {
    let window_clone = overlay_window.clone();
    let _ = overlay_window.run_on_main_thread(move || {
        // Try to get the GTK window from the Tauri webview
        if let Ok(gtk_window) = window_clone.gtk_window() {
            let settings = settings::get_settings(window_clone.app_handle());
            match settings.overlay_position {
                OverlayPosition::Top => {
                    gtk_window.set_anchor(Edge::Top, true);
                    gtk_window.set_anchor(Edge::Bottom, false);
                }
                OverlayPosition::Bottom => {
                    gtk_window.set_anchor(Edge::Bottom, true);
                    gtk_window.set_anchor(Edge::Top, false);
                }
            }
        }
    });
}

/// Returns true when the environment variable is set to a truthy value
/// (e.g. "1", "true", "yes", "on").
/// "0", "false", "no", "off" and empty string are treated as falsy (case-insensitive).
/// Returns false when the variable is not set.
#[cfg(target_os = "linux")]
fn env_flag_enabled(name: &str) -> bool {
    match env::var(name) {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "" | "0" | "false" | "no" | "off"
        ),
        Err(_) => false,
    }
}

/// Initializes GTK layer shell for Linux overlay window
/// Returns true if layer shell was successfully initialized, false otherwise
#[cfg(target_os = "linux")]
fn init_gtk_layer_shell(overlay_window: &tauri::webview::WebviewWindow) -> bool {
    if env_flag_enabled("HANDY_NO_GTK_LAYER_SHELL") {
        debug!("Skipping GTK layer shell init (HANDY_NO_GTK_LAYER_SHELL is enabled)");
        return false;
    }

    if !gtk_layer_shell::is_supported() {
        return false;
    }

    // Try to get the GTK window from the Tauri webview
    if let Ok(gtk_window) = overlay_window.gtk_window() {
        // Initialize layer shell
        gtk_window.init_layer_shell();
        gtk_window.set_layer(Layer::Overlay);
        gtk_window.set_keyboard_mode(KeyboardMode::None);
        gtk_window.set_exclusive_zone(0);

        update_gtk_layer_shell_anchors(overlay_window);

        return true;
    }
    false
}

/// Forces a window to be topmost using Win32 API (Windows only)
/// This is more reliable than Tauri's set_always_on_top which can be overridden
#[cfg(target_os = "windows")]
fn force_overlay_topmost(overlay_window: &tauri::webview::WebviewWindow) {
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
    };

    // Clone because run_on_main_thread takes 'static
    let overlay_clone = overlay_window.clone();

    // Make sure the Win32 call happens on the UI thread
    let _ = overlay_clone.clone().run_on_main_thread(move || {
        if let Ok(hwnd) = overlay_clone.hwnd() {
            unsafe {
                // Force Z-order: make this window topmost without changing size/pos or stealing focus
                let _ = SetWindowPos(
                    hwnd,
                    Some(HWND_TOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                );
            }
        }
    });
}

fn get_monitor_with_cursor(app_handle: &AppHandle) -> Option<tauri::Monitor> {
    if let Some(mouse_location) = input::get_cursor_position(app_handle) {
        if let Ok(monitors) = app_handle.available_monitors() {
            for monitor in monitors {
                // Tauri's monitor position/size are physical pixels, but enigo
                // may return logical coordinates (confirmed on macOS via
                // NSEvent::mouseLocation; on Windows, GetCursorPos behavior
                // depends on the process DPI-awareness context). Dividing by
                // scale_factor normalizes to logical, which is safe regardless:
                // if enigo returns logical it matches directly, and if it returns
                // physical on a scale=1 monitor the division is a no-op.
                let scale = monitor.scale_factor();
                let pos = PhysicalPosition::new(
                    (monitor.position().x as f64 / scale) as i32,
                    (monitor.position().y as f64 / scale) as i32,
                );
                let size = PhysicalSize::new(
                    (monitor.size().width as f64 / scale) as u32,
                    (monitor.size().height as f64 / scale) as u32,
                );
                if is_mouse_within_monitor(mouse_location, &pos, &size) {
                    return Some(monitor);
                }
            }
        }
    }

    app_handle.primary_monitor().ok().flatten()
}

fn is_mouse_within_monitor(
    mouse_pos: (i32, i32),
    monitor_pos: &PhysicalPosition<i32>,
    monitor_size: &PhysicalSize<u32>,
) -> bool {
    let (mouse_x, mouse_y) = mouse_pos;
    let PhysicalPosition {
        x: monitor_x,
        y: monitor_y,
    } = *monitor_pos;
    let PhysicalSize {
        width: monitor_width,
        height: monitor_height,
    } = *monitor_size;

    mouse_x >= monitor_x
        && mouse_x < (monitor_x + monitor_width as i32)
        && mouse_y >= monitor_y
        && mouse_y < (monitor_y + monitor_height as i32)
}

/// Returns overlay position in logical coordinates (points on macOS).
///
/// Uses monitor position/size directly rather than work_area(), which can
/// return incorrect coordinates on macOS for monitors with negative positions.
/// The per-platform OVERLAY_TOP_OFFSET / OVERLAY_BOTTOM_OFFSET constants
/// already account for system chrome (menu bar, taskbar).
///
/// We must use LogicalPosition (not PhysicalPosition) because Tauri/tao
/// converts PhysicalPosition using the scale factor of the monitor the window
/// is *currently* on, which is wrong when moving cross-monitor.
fn calculate_overlay_position(
    app_handle: &AppHandle,
    width: f64,
    height: f64,
) -> Option<(f64, f64)> {
    let monitor = get_monitor_with_cursor(app_handle)?;
    let scale = monitor.scale_factor();
    let monitor_x = monitor.position().x as f64 / scale;
    let monitor_y = monitor.position().y as f64 / scale;
    let monitor_width = monitor.size().width as f64 / scale;
    let monitor_height = monitor.size().height as f64 / scale;

    let settings = settings::get_settings(app_handle);

    let x = monitor_x + (monitor_width - width) / 2.0;
    let y = match settings.overlay_position {
        OverlayPosition::Top => monitor_y + OVERLAY_TOP_OFFSET,
        OverlayPosition::Bottom => monitor_y + monitor_height - height - OVERLAY_BOTTOM_OFFSET,
    };

    Some((x, y))
}

/// Current overlay window size in logical units (points), for repositioning
/// without assuming a fixed size (compact vs. streaming).
fn current_overlay_logical_size(window: &tauri::webview::WebviewWindow) -> Option<(f64, f64)> {
    let size = window.inner_size().ok()?;
    let scale = window.scale_factor().ok()?;
    Some((size.width as f64 / scale, size.height as f64 / scale))
}

fn calculate_clipboard_overlay_position(app_handle: &AppHandle) -> Option<(f64, f64)> {
    let monitor = get_monitor_with_cursor(app_handle)?;
    let scale = monitor.scale_factor();
    let monitor_x = monitor.position().x as f64 / scale;
    let monitor_y = monitor.position().y as f64 / scale;
    let monitor_width = monitor.size().width as f64 / scale;
    let monitor_height = monitor.size().height as f64 / scale;

    let x = monitor_x + (monitor_width - CLIPBOARD_OVERLAY_WIDTH) / 2.0;
    let y = monitor_y + (monitor_height - CLIPBOARD_OVERLAY_HEIGHT) / 2.0;

    Some((x.max(monitor_x), y.max(monitor_y)))
}

#[cfg(target_os = "macos")]
fn run_clipboard_overlay_on_main_thread<F>(
    app_handle: &AppHandle,
    operation: &'static str,
    action: F,
) where
    F: FnOnce(AppHandle) + Send + 'static,
{
    if tauri_nspanel::objc2::MainThreadMarker::new().is_some() {
        action(app_handle.clone());
        return;
    }

    let app_handle = app_handle.clone();
    let main_app_handle = app_handle.clone();

    if let Err(err) = app_handle.run_on_main_thread(move || action(main_app_handle)) {
        debug!("Failed to schedule clipboard overlay {operation} on main thread: {err}");
    }
}

#[cfg(not(target_os = "macos"))]
pub fn create_clipboard_overlay(app_handle: &AppHandle) {
    if app_handle.get_webview_window("clipboard_overlay").is_some() {
        return;
    }

    let mut builder = WebviewWindowBuilder::new(
        app_handle,
        "clipboard_overlay",
        tauri::WebviewUrl::App("/src/overlay/clipboard/index.html".into()),
    )
    .title("Clipboard")
    .resizable(false)
    .inner_size(CLIPBOARD_OVERLAY_WIDTH, CLIPBOARD_OVERLAY_HEIGHT)
    .shadow(true)
    .maximizable(false)
    .minimizable(false)
    .closable(true)
    .accept_first_mouse(true)
    .decorations(false)
    .always_on_top(false)
    .skip_taskbar(false)
    .transparent(true)
    .visible(false);

    if let Some((x, y)) = calculate_clipboard_overlay_position(app_handle) {
        builder = builder.position(x, y);
    } else {
        builder = builder.center();
    }

    if let Some(data_dir) = crate::portable::data_dir() {
        builder = builder.data_directory(data_dir.join("webview"));
    }

    match builder.build() {
        Ok(_) => {
            debug!("Clipboard overlay window created successfully (hidden)");
        }
        Err(e) => {
            debug!("Failed to create clipboard overlay window: {}", e);
        }
    }
}

#[cfg(target_os = "macos")]
fn create_clipboard_overlay_on_main_thread(app_handle: &AppHandle) {
    if app_handle.get_webview_window("clipboard_overlay").is_some() {
        return;
    }

    let clipboard_overlay_url = tauri::WebviewUrl::App("/src/overlay/clipboard/index.html".into());

    let mut builder =
        WebviewWindowBuilder::new(app_handle, "clipboard_overlay", clipboard_overlay_url)
            .title("Clipboard")
            .resizable(false)
            .inner_size(CLIPBOARD_OVERLAY_WIDTH, CLIPBOARD_OVERLAY_HEIGHT)
            .shadow(true)
            .maximizable(false)
            .minimizable(false)
            .closable(true)
            .accept_first_mouse(true)
            .decorations(true)
            .always_on_top(true)
            .transparent(false)
            .visible(true);

    if let Some((x, y)) = calculate_clipboard_overlay_position(app_handle) {
        builder = builder.position(x, y);
    } else {
        builder = builder.center();
    }

    if let Some(data_dir) = crate::portable::data_dir() {
        builder = builder.data_directory(data_dir.join("webview"));
    }

    match builder.build() {
        Ok(_) => {
            debug!("Clipboard overlay window created successfully");
        }
        Err(e) => {
            debug!("Failed to create clipboard overlay window: {}", e);
        }
    }
}

#[cfg(target_os = "macos")]
#[allow(dead_code)]
pub fn create_clipboard_overlay(app_handle: &AppHandle) {
    run_clipboard_overlay_on_main_thread(app_handle, "create", |app_handle| {
        create_clipboard_overlay_on_main_thread(&app_handle);
    });
}

#[cfg(target_os = "macos")]
#[allow(dead_code)]
pub fn show_clipboard_overlay(app_handle: &AppHandle) {
    run_clipboard_overlay_on_main_thread(app_handle, "show", |app_handle| {
        show_clipboard_overlay_on_main_thread(&app_handle);
    });
}

#[cfg(target_os = "macos")]
fn show_clipboard_overlay_on_main_thread(app_handle: &AppHandle) {
    create_clipboard_overlay_on_main_thread(app_handle);

    if let Some(overlay_window) = app_handle.get_webview_window("clipboard_overlay") {
        if let Some((x, y)) = calculate_clipboard_overlay_position(app_handle) {
            if let Err(err) = overlay_window
                .set_position(tauri::Position::Logical(tauri::LogicalPosition { x, y }))
            {
                debug!("Failed to update clipboard overlay position: {err}");
            }
        }

        match overlay_window.show() {
            Ok(_) => {
                #[cfg(debug_assertions)]
                overlay_window.open_devtools();

                if let Err(err) = overlay_window.set_focus() {
                    debug!("Failed to focus clipboard overlay window: {err}");
                }
                if let Err(err) = overlay_window.set_always_on_top(true) {
                    debug!("Failed to update clipboard overlay z-order: {err}");
                }
                CLIPBOARD_OVERLAY_FOCUSED.store(true, Ordering::Relaxed);
                debug!("Clipboard overlay window shown");
            }
            Err(err) => {
                debug!("Failed to show clipboard overlay window: {err}");
            }
        }
    } else {
        debug!("Failed to find clipboard overlay window");
    }
}

#[cfg(not(target_os = "macos"))]
pub fn show_clipboard_overlay(app_handle: &AppHandle) {
    create_clipboard_overlay(app_handle);

    if let Some(overlay_window) = app_handle.get_webview_window("clipboard_overlay") {
        if let Some((x, y)) = calculate_clipboard_overlay_position(app_handle) {
            let _ = overlay_window
                .set_position(tauri::Position::Logical(tauri::LogicalPosition { x, y }));
        }
        let _ = overlay_window.show();
        let _ = overlay_window.set_focus();
    }
}

#[cfg(target_os = "macos")]
fn hide_clipboard_overlay_on_main_thread(app_handle: &AppHandle) {
    if let Some(overlay_window) = app_handle.get_webview_window("clipboard_overlay") {
        if let Err(err) = overlay_window.hide() {
            debug!("Failed to hide clipboard overlay window: {err}");
        }
    }

    CLIPBOARD_OVERLAY_FOCUSED.store(false, Ordering::Relaxed);
}

#[cfg(target_os = "macos")]
#[allow(dead_code)]
pub fn hide_clipboard_overlay(app_handle: &AppHandle) {
    run_clipboard_overlay_on_main_thread(app_handle, "hide", |app_handle| {
        hide_clipboard_overlay_on_main_thread(&app_handle);
    });
}

#[cfg(not(target_os = "macos"))]
pub fn hide_clipboard_overlay(app_handle: &AppHandle) {
    if let Some(overlay_window) = app_handle.get_webview_window("clipboard_overlay") {
        if let Err(err) = overlay_window.hide() {
            debug!("Failed to hide clipboard overlay window: {err}");
        }
    }
}

#[cfg(target_os = "macos")]
fn set_clipboard_overlay_pinned_on_main_thread(app_handle: &AppHandle, _pinned: bool) {
    if let Some(overlay_window) = app_handle.get_webview_window("clipboard_overlay") {
        if let Err(err) = overlay_window.set_always_on_top(true) {
            debug!("Failed to update clipboard overlay pinned state: {err}");
        }
    }
}

#[cfg(target_os = "macos")]
pub fn set_clipboard_overlay_pinned(app_handle: &AppHandle, pinned: bool) {
    CLIPBOARD_OVERLAY_PINNED.store(pinned, Ordering::Relaxed);
    run_clipboard_overlay_on_main_thread(app_handle, "pin", move |app_handle| {
        set_clipboard_overlay_pinned_on_main_thread(&app_handle, pinned);
    });
}

#[cfg(not(target_os = "macos"))]
pub fn set_clipboard_overlay_pinned(app_handle: &AppHandle, pinned: bool) {
    if let Some(overlay_window) = app_handle.get_webview_window("clipboard_overlay") {
        if let Err(err) = overlay_window.set_always_on_top(pinned) {
            debug!("Failed to update clipboard overlay pinned state: {err}");
        }
    }
}

#[cfg(target_os = "macos")]
fn is_clipboard_overlay_visible_on_main_thread(app_handle: &AppHandle) -> bool {
    app_handle
        .get_webview_window("clipboard_overlay")
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
#[allow(dead_code)]
pub fn is_clipboard_overlay_visible(app_handle: &AppHandle) -> bool {
    if tauri_nspanel::objc2::MainThreadMarker::new().is_some() {
        return is_clipboard_overlay_visible_on_main_thread(app_handle);
    }

    false
}

#[cfg(not(target_os = "macos"))]
pub fn is_clipboard_overlay_visible(app_handle: &AppHandle) -> bool {
    app_handle
        .get_webview_window("clipboard_overlay")
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
pub fn toggle_clipboard_overlay(app_handle: &AppHandle) {
    run_clipboard_overlay_on_main_thread(app_handle, "toggle", |app_handle| {
        if is_clipboard_overlay_visible_on_main_thread(&app_handle) {
            hide_clipboard_overlay_on_main_thread(&app_handle);
        } else {
            show_clipboard_overlay_on_main_thread(&app_handle);
        }
    });
}

#[cfg(not(target_os = "macos"))]
pub fn toggle_clipboard_overlay(app_handle: &AppHandle) {
    if is_clipboard_overlay_visible(app_handle) {
        hide_clipboard_overlay(app_handle);
    } else {
        show_clipboard_overlay(app_handle);
    }
}

#[cfg(target_os = "macos")]
pub fn set_clipboard_overlay_focused(focused: bool) {
    CLIPBOARD_OVERLAY_FOCUSED.store(focused, Ordering::Relaxed);
}

#[cfg(not(target_os = "macos"))]
pub fn set_clipboard_overlay_focused(_focused: bool) {}

#[cfg(target_os = "macos")]
pub fn hide_clipboard_overlay_if_unfocused(app_handle: &AppHandle) {
    run_clipboard_overlay_on_main_thread(app_handle, "hide if unfocused", |app_handle| {
        if CLIPBOARD_OVERLAY_FOCUSED.load(Ordering::Relaxed) {
            return;
        }

        if CLIPBOARD_OVERLAY_PINNED.load(Ordering::Relaxed) {
            return;
        }

        hide_clipboard_overlay_on_main_thread(&app_handle);
    });
}

#[cfg(not(target_os = "macos"))]
pub fn hide_clipboard_overlay_if_unfocused(app_handle: &AppHandle) {
    let Some(window) = app_handle.get_webview_window("clipboard_overlay") else {
        return;
    };

    if window.is_focused().unwrap_or(false) {
        return;
    }

    match window.is_always_on_top() {
        Ok(true) => {}
        Ok(false) => {
            hide_clipboard_overlay(app_handle);
        }
        Err(e) => {
            log::error!("Failed to read clipboard overlay pinned state: {}", e);
            hide_clipboard_overlay(app_handle);
        }
    }
}

/// Creates the recording overlay window and keeps it hidden by default
#[cfg(not(target_os = "macos"))]
pub fn create_recording_overlay(app_handle: &AppHandle) {
    // On Linux (Wayland), monitor detection often fails, but we don't need exact coordinates
    // for Layer Shell as we use anchors. On other platforms, we require a monitor.
    #[cfg(not(target_os = "linux"))]
    {
        let position = calculate_overlay_position(app_handle, OVERLAY_WIDTH, OVERLAY_HEIGHT);
        if position.is_none() {
            debug!("Failed to determine overlay position, not creating overlay window");
            return;
        }
    }

    // Position starts unset — update_overlay_position() sets the correct
    // LogicalPosition before the overlay is shown.
    let mut builder = WebviewWindowBuilder::new(
        app_handle,
        "recording_overlay",
        tauri::WebviewUrl::App("src/overlay/index.html".into()),
    )
    .title("Recording")
    .resizable(false)
    .inner_size(OVERLAY_WIDTH, OVERLAY_HEIGHT)
    .shadow(false)
    .maximizable(false)
    .minimizable(false)
    .closable(false)
    .accept_first_mouse(true)
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .transparent(true)
    .focusable(false)
    .focused(false)
    .visible(false);

    if let Some(data_dir) = crate::portable::data_dir() {
        builder = builder.data_directory(data_dir.join("webview"));
    }

    #[allow(unused_variables)]
    match builder.build() {
        Ok(window) => {
            #[cfg(target_os = "linux")]
            {
                // Try to initialize GTK layer shell, ignore errors if compositor doesn't support it
                if init_gtk_layer_shell(&window) {
                    debug!("GTK layer shell initialized for overlay window");
                } else {
                    debug!("GTK layer shell not available, falling back to regular window");
                }
            }

            debug!("Recording overlay window created successfully (hidden)");
        }
        Err(e) => {
            debug!("Failed to create recording overlay window: {}", e);
        }
    }
}

/// Creates the recording overlay panel and keeps it hidden by default (macOS)
#[cfg(target_os = "macos")]
fn run_recording_overlay_on_main_thread<F>(
    app_handle: &AppHandle,
    operation: &'static str,
    action: F,
) where
    F: FnOnce(AppHandle) + Send + 'static,
{
    if tauri_nspanel::objc2::MainThreadMarker::new().is_some() {
        action(app_handle.clone());
        return;
    }

    let app_handle = app_handle.clone();
    let main_app_handle = app_handle.clone();

    if let Err(err) = app_handle.run_on_main_thread(move || action(main_app_handle)) {
        debug!("Failed to schedule recording overlay {operation} on main thread: {err}");
    }
}

#[cfg(target_os = "macos")]
fn create_recording_overlay_on_main_thread(app_handle: &AppHandle) {
    if let Some((x, y)) = calculate_overlay_position(app_handle, OVERLAY_WIDTH, OVERLAY_HEIGHT) {
        // PanelBuilder creates a Tauri window then converts it to NSPanel.
        // The window remains registered, so get_webview_window() still works.
        match PanelBuilder::<_, RecordingOverlayPanel>::new(app_handle, "recording_overlay")
            .url(WebviewUrl::App("src/overlay/index.html".into()))
            .title("Recording")
            .position(tauri::Position::Logical(tauri::LogicalPosition { x, y }))
            .level(PanelLevel::Status)
            .size(tauri::Size::Logical(tauri::LogicalSize {
                width: OVERLAY_WIDTH,
                height: OVERLAY_HEIGHT,
            }))
            .has_shadow(false)
            .transparent(true)
            .no_activate(true)
            .corner_radius(0.0)
            .style_mask(StyleMask::empty().borderless().nonactivating_panel())
            .with_window(|w| w.decorations(false).transparent(true).focusable(false))
            .collection_behavior(
                CollectionBehavior::new()
                    .can_join_all_spaces()
                    .full_screen_auxiliary(),
            )
            .build()
        {
            Ok(panel) => {
                panel.hide();
            }
            Err(e) => {
                log::error!("Failed to create recording overlay panel: {}", e);
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub fn create_recording_overlay(app_handle: &AppHandle) {
    run_recording_overlay_on_main_thread(app_handle, "create", |app_handle| {
        create_recording_overlay_on_main_thread(&app_handle);
    });
}

fn show_overlay_state_now(app_handle: &AppHandle, state: &str) {
    // Whether the overlay shows at all is governed by overlay_style; position
    // only chooses Top vs Bottom placement.
    let settings = settings::get_settings(app_handle);
    if settings.overlay_style == OverlayStyle::None {
        return;
    }

    // Size the overlay for this state (compact vs. streaming), then position it.
    let (width, height) = overlay_dimensions(state);
    if let Some(overlay_window) = app_handle.get_webview_window("recording_overlay") {
        #[cfg(target_os = "linux")]
        update_gtk_layer_shell_anchors(&overlay_window);

        let size_started = std::time::Instant::now();
        let _ = overlay_window.set_size(tauri::Size::Logical(tauri::LogicalSize { width, height }));
        let size_elapsed = size_started.elapsed();

        let pos_started = std::time::Instant::now();
        let mut set_pos_elapsed = std::time::Duration::ZERO;
        if let Some((x, y)) = calculate_overlay_position(app_handle, width, height) {
            let set_pos_started = std::time::Instant::now();
            let _ = overlay_window
                .set_position(tauri::Position::Logical(tauri::LogicalPosition { x, y }));
            set_pos_elapsed = set_pos_started.elapsed();
        }
        let pos_calc_elapsed = pos_started.elapsed() - set_pos_elapsed;

        let show_started = std::time::Instant::now();
        let _ = overlay_window.show();
        let show_elapsed = show_started.elapsed();

        // On Windows, aggressively re-assert "topmost" in the native Z-order after showing
        #[cfg(target_os = "windows")]
        force_overlay_topmost(&overlay_window);

        let _ = overlay_window.emit("show-overlay", state);
        log::debug!(
            "overlay '{}': set_size={:?} pos_calc={:?} set_pos={:?} show={:?}",
            state,
            size_elapsed,
            pos_calc_elapsed,
            set_pos_elapsed,
            show_elapsed
        );
    }
}

#[cfg(target_os = "macos")]
fn show_overlay_state(app_handle: &AppHandle, state: &'static str) {
    run_recording_overlay_on_main_thread(app_handle, "show", move |app_handle| {
        show_overlay_state_now(&app_handle, state);
    });
}

#[cfg(not(target_os = "macos"))]
fn show_overlay_state(app_handle: &AppHandle, state: &str) {
    show_overlay_state_now(app_handle, state);
}

/// Shows the recording overlay window with fade-in animation
pub fn show_recording_overlay(app_handle: &AppHandle) {
    show_overlay_state(app_handle, "recording");
}

/// Shows the larger streaming overlay that displays live transcription text
pub fn show_streaming_overlay(app_handle: &AppHandle) {
    show_overlay_state(app_handle, "streaming");
}

/// Shows the transcribing overlay window
pub fn show_transcribing_overlay(app_handle: &AppHandle) {
    show_overlay_state(app_handle, "transcribing");
}

/// Shows the processing overlay window
pub fn show_processing_overlay(app_handle: &AppHandle) {
    show_overlay_state(app_handle, "processing");
}

/// Updates the overlay window position based on current settings
fn update_overlay_position_now(app_handle: &AppHandle) {
    if let Some(overlay_window) = app_handle.get_webview_window("recording_overlay") {
        #[cfg(target_os = "linux")]
        {
            update_gtk_layer_shell_anchors(&overlay_window);
        }

        // Use the window's current size so centering stays correct whether the
        // overlay is in compact or streaming layout.
        let (width, height) = current_overlay_logical_size(&overlay_window)
            .unwrap_or((OVERLAY_WIDTH, OVERLAY_HEIGHT));
        if let Some((x, y)) = calculate_overlay_position(app_handle, width, height) {
            let _ = overlay_window
                .set_position(tauri::Position::Logical(tauri::LogicalPosition { x, y }));
        }
    }
}

/// Updates the overlay window position based on current settings
#[cfg(target_os = "macos")]
pub fn update_overlay_position(app_handle: &AppHandle) {
    run_recording_overlay_on_main_thread(app_handle, "position", |app_handle| {
        update_overlay_position_now(&app_handle);
    });
}

/// Updates the overlay window position based on current settings
#[cfg(not(target_os = "macos"))]
pub fn update_overlay_position(app_handle: &AppHandle) {
    update_overlay_position_now(app_handle);
}

/// Hides the recording overlay window with fade-out animation
pub fn hide_recording_overlay(app_handle: &AppHandle) {
    // Always hide the overlay regardless of settings - if setting was changed while recording,
    // we still want to hide it properly
    if let Some(overlay_window) = app_handle.get_webview_window("recording_overlay") {
        // Emit event to trigger fade-out animation
        let _ = overlay_window.emit("hide-overlay", ());
        // Hide the window after a short delay to allow animation to complete
        #[cfg(target_os = "macos")]
        {
            let app_handle = app_handle.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(300));
                run_recording_overlay_on_main_thread(&app_handle, "hide", |app_handle| {
                    if let Some(overlay_window) = app_handle.get_webview_window("recording_overlay")
                    {
                        let _ = overlay_window.hide();
                    }
                });
            });
            return;
        }

        #[cfg(not(target_os = "macos"))]
        {
            let window_clone = overlay_window.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(300));
                let window_for_hide = window_clone.clone();
                if let Err(err) = window_clone.run_on_main_thread(move || {
                    let _ = window_for_hide.hide();
                }) {
                    debug!("Failed to schedule recording overlay hide on main thread: {err}");
                }
            });
        }
    }
}

// Cached "overlay is enabled" flag, kept in sync with overlay_style. Avoids
// reading the Tauri store on every audio callback (~24 Hz during recording).
// Defaults to false so the audio path doesn't emit until lib.rs::setup
// populates the cache from initial settings.
static OVERLAY_ENABLED: AtomicBool = AtomicBool::new(false);

/// Update the cached overlay-enabled flag. Called from `lib.rs` at
/// startup after settings load, and from `change_overlay_style_setting`
/// whenever the user changes whether the overlay is shown.
pub fn update_overlay_enabled_cache(enabled: bool) {
    OVERLAY_ENABLED.store(enabled, Ordering::Relaxed);
}

pub fn emit_levels(app_handle: &AppHandle, levels: &[f32]) {
    // Skip emission when the overlay is disabled. The recording_overlay
    // window is created at boot regardless of overlay_style, so without this
    // guard a hidden overlay's WebKit subprocess still
    // processes every event. Each event drives some kind of WebKit
    // C++ allocation that accumulates without bound (mechanism not
    // directly characterized; see issue #1279 for the investigation).
    // For users with `overlay_style: none` (the Linux default) this skip
    // eliminates the upstream driver of that accumulation.
    if !OVERLAY_ENABLED.load(Ordering::Relaxed) {
        return;
    }

    // Target only the overlay window. In Tauri 2 both `AppHandle::emit`
    // and `WebviewWindow::emit` broadcast to all webviews; Tauri's
    // listener filter then skips webviews with no registered listener
    // for the event, so the settings webview never received `mic-level`.
    // But the previous dual-call pattern still produced two `eval_script`
    // calls to the overlay per audio callback (one from each .emit()).
    // `emit_to` with the overlay's window label produces a single
    // eval_script call per callback, cutting the per-callback WebKit
    // dispatch work in half.
    let _ = app_handle.emit_to("recording_overlay", "mic-level", levels);
}
