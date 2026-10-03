//! Keyboard listener for streaming raw key events
//!
//! This module provides a `KeyboardListener` that streams all keyboard events,
//! useful for implementing "record hotkey" UI flows.
//!
//! # Platform Notes
//!
//! - **macOS**: Uses CGEventTap. Requires accessibility permissions.
//! - **Windows**: Uses low-level keyboard hooks. Clean thread shutdown.
//! - **Linux**: Reads evdev devices directly (Wayland, X11, and console
//!   alike). Requires read access to `/dev/input` (`input` group).
//!   Blocking grabs keyboards exclusively and re-injects non-blocked
//!   events through uinput, so it additionally requires write access to
//!   `/dev/uinput`. Clean thread shutdown.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::error::{Error, Result};
use crate::types::KeyEvent;

pub use crate::platform::state::BlockingHotkeys;

/// Platform-agnostic Keyboard Listener
///
/// Streams all keyboard events. Can optionally block events that match
/// registered hotkeys.
pub struct KeyboardListener {
    event_receiver: Receiver<KeyEvent>,
    _thread_handle: Option<JoinHandle<()>>,
    running: Arc<AtomicBool>,
    blocking_hotkeys: Option<BlockingHotkeys>,
    /// Backend-specific wakeup invoked on Drop after `running` is cleared.
    /// macOS parks its listener thread in a CFRunLoop with no periodic
    /// wakeups (see run_event_tap), so Drop must stop that loop explicitly.
    stop_wakeup: Option<Arc<Mutex<Box<dyn Fn() + Send>>>>,
}

/// Manager 和 listener 共用停止信号；不用等待转发线程 Drop 才唤醒原生循环。
#[derive(Clone)]
pub(crate) struct ListenerStop {
    running: Arc<AtomicBool>,
    wakeup: Option<Arc<Mutex<Box<dyn Fn() + Send>>>>,
}

impl ListenerStop {
    pub(crate) fn request(&self) {
        self.running.store(false, Ordering::Release);
        if let Some(wakeup) = &self.wakeup {
            // 只在退休路径使用，不进入按键回调；没有修改授权或工作线程计数。
            let wakeup = wakeup.lock().unwrap_or_else(|error| error.into_inner());
            wakeup();
        }
    }
}

impl KeyboardListener {
    /// Create a new KeyboardListener (non-blocking mode)
    ///
    /// Events are observed but not blocked. Use this for "record hotkey" UI flows.
    ///
    /// On macOS, this will check for accessibility permissions and fail if not granted.
    pub fn new() -> Result<Self> {
        Self::new_internal(None)
    }

    /// Create a new KeyboardListener with blocking support
    ///
    /// Events matching hotkeys in the provided set will be blocked from reaching
    /// other applications. The set can be modified after creation to add/remove
    /// hotkeys dynamically.
    ///
    /// Note: On Linux this grabs keyboards exclusively and re-injects
    /// non-blocked events through uinput, so it requires write access to
    /// `/dev/uinput` (and fails with an actionable error without it).
    /// Mouse-button hotkeys are detected but not blocked — pointer devices
    /// are never grabbed (same behavior as Windows).
    pub fn new_with_blocking(blocking_hotkeys: BlockingHotkeys) -> Result<Self> {
        Self::new_internal(Some(blocking_hotkeys))
    }

    fn new_internal(blocking_hotkeys: Option<BlockingHotkeys>) -> Result<Self> {
        #[cfg(target_os = "macos")]
        {
            use crate::platform::macos::listener;
            let state = listener::spawn(blocking_hotkeys)?;
            let run_loop = state.run_loop;
            Ok(KeyboardListener {
                event_receiver: state.event_receiver,
                _thread_handle: state.thread_handle,
                running: state.running,
                blocking_hotkeys: state.blocking_hotkeys,
                stop_wakeup: Some(Arc::new(Mutex::new(Box::new(move || run_loop.stop())))),
            })
        }

        #[cfg(target_os = "windows")]
        {
            use crate::platform::windows::listener;
            let state = listener::spawn(blocking_hotkeys)?;
            Ok(KeyboardListener {
                event_receiver: state.event_receiver,
                _thread_handle: state.thread_handle,
                running: state.running,
                blocking_hotkeys: state.blocking_hotkeys,
                stop_wakeup: None,
            })
        }

        #[cfg(target_os = "linux")]
        {
            use crate::platform::linux::listener;
            let state = listener::spawn(blocking_hotkeys)?;
            Ok(KeyboardListener {
                event_receiver: state.event_receiver,
                _thread_handle: state.thread_handle,
                running: state.running,
                blocking_hotkeys: state.blocking_hotkeys,
                stop_wakeup: None,
            })
        }
    }

    /// Observe actual native-thread completion without confusing an empty queue with health.
    pub(crate) fn running_flag(&self) -> Arc<AtomicBool> {
        self.running.clone()
    }

    pub fn is_healthy(&self) -> bool {
        self.running.load(Ordering::Acquire)
            && self
                ._thread_handle
                .as_ref()
                .is_some_and(|h| !h.is_finished())
    }

    /// Stop with a bounded receipt. A false result retains the native owner for retry.
    pub fn stop_and_wait(&mut self, timeout: Duration) -> bool {
        self.stop_handle().request();
        let deadline = std::time::Instant::now() + timeout;
        while self
            ._thread_handle
            .as_ref()
            .is_some_and(|h| !h.is_finished())
        {
            if std::time::Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        if let Some(handle) = self._thread_handle.take() {
            let _ = handle.join();
        }
        true
    }

    pub(crate) fn stop_handle(&self) -> ListenerStop {
        ListenerStop {
            running: self.running.clone(),
            wakeup: self.stop_wakeup.clone(),
        }
    }

    /// Get a reference to the blocking hotkeys set (if blocking is enabled)
    pub fn blocking_hotkeys(&self) -> Option<&BlockingHotkeys> {
        self.blocking_hotkeys.as_ref()
    }

    /// Blocking receive for key events
    ///
    /// Blocks until a key event is received or the listener stops.
    pub fn recv(&self) -> Result<KeyEvent> {
        self.event_receiver
            .recv()
            .map_err(|_| Error::EventLoopNotRunning)
    }

    /// Blocking receive with timeout
    ///
    /// Blocks until a key event is received, the timeout expires, or the listener stops.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<KeyEvent> {
        self.event_receiver
            .recv_timeout(timeout)
            .map_err(|e| match e {
                std::sync::mpsc::RecvTimeoutError::Timeout => Error::Timeout,
                std::sync::mpsc::RecvTimeoutError::Disconnected => Error::EventLoopNotRunning,
            })
    }

    /// Non-blocking receive for key events
    ///
    /// Returns `Some(event)` if an event is available, `None` otherwise.
    pub fn try_recv(&self) -> Option<KeyEvent> {
        match self.event_receiver.try_recv() {
            Ok(event) => Some(event),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => None,
        }
    }
}

impl Drop for KeyboardListener {
    fn drop(&mut self) {
        // Wake the listener thread if the backend parks it (macOS stops the
        // tap thread's CFRunLoop). Windows/Linux event loops re-check
        // `running` at least every ~100ms on their own, so the join below is
        // short and shutdown is clean on all platforms.
        self.stop_handle().request();
        // Never block Drop on a native run loop. Worker accounting remains live
        // until the thread actually exits, so callers cannot mistake detach for retirement.
        if let Some(handle) = self._thread_handle.take() {
            if handle.is_finished() {
                let _ = handle.join();
            }
        }
    }
}

#[cfg(test)]
mod stop_tests {
    use super::*;

    #[test]
    fn owner_can_stop_native_loop_before_forwarder_drops_listener() {
        let running = Arc::new(AtomicBool::new(true));
        let called = Arc::new(AtomicBool::new(false));
        let notified = called.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let listener = KeyboardListener {
            event_receiver: receiver,
            _thread_handle: None,
            running: running.clone(),
            blocking_hotkeys: None,
            stop_wakeup: Some(Arc::new(Mutex::new(Box::new(move || {
                notified.store(true, Ordering::Release);
            })))),
        };
        let stop = listener.stop_handle();
        let (release, wait) = std::sync::mpsc::channel();
        let forwarder = std::thread::spawn(move || {
            wait.recv().unwrap();
            drop(listener);
        });
        stop.request();
        assert!(!forwarder.is_finished());
        assert!(!running.load(Ordering::Acquire));
        assert!(called.load(Ordering::Acquire));
        release.send(()).unwrap();
        forwarder.join().unwrap();
        drop(sender);
    }
}
