//! Local Inputia safety patch: atomic callback admission and native-worker accounting.
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    OnceLock,
};
static PERMISSION: OnceLock<fn() -> bool> = OnceLock::new();
static BLOCKING: AtomicBool = AtomicBool::new(true);
static GENERATION: AtomicUsize = AtomicUsize::new(0);
static WORKERS: AtomicUsize = AtomicUsize::new(0);
/// Install a process-lifetime, nonblocking permission predicate before creating listeners.
pub fn set_permission_check(check: fn() -> bool) {
    let _ = PERMISSION.set(check);
}
/// Pause blocking and dispatch immediately, independently of native-worker locks.
pub fn set_blocking_enabled(enabled: bool) {
    if !enabled {
        GENERATION.fetch_add(1, Ordering::AcqRel);
    }
    BLOCKING.store(enabled, Ordering::Release);
}
/// Whether all previously spawned native tap threads have actually exited.
pub fn active_listener_count() -> usize {
    WORKERS.load(Ordering::Acquire)
}
pub(crate) fn allowed(blocking: bool) -> bool {
    PERMISSION.get().map_or(true, |check| check())
        && (!blocking || BLOCKING.load(Ordering::Acquire))
}
pub(crate) struct WorkerLease;
impl WorkerLease {
    pub(crate) fn new() -> Self {
        WORKERS.fetch_add(1, Ordering::AcqRel);
        Self
    }
}
impl Drop for WorkerLease {
    fn drop(&mut self) {
        WORKERS.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(crate) fn generation() -> usize {
    GENERATION.load(Ordering::Acquire)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pause_is_immediate_and_advances_reset_generation() {
        let before = generation();
        set_blocking_enabled(false);
        assert!(!allowed(true));
        assert!(allowed(false)); // Shortcut recording remains an observer.
        assert_ne!(before, generation());
        set_blocking_enabled(true);
        assert!(allowed(true));
    }
    #[test]
    fn stalled_worker_remains_counted_until_it_really_exits() {
        let before = active_listener_count();
        let (tx, rx) = std::sync::mpsc::channel();
        let lease = WorkerLease::new();
        let handle = std::thread::spawn(move || {
            let _lease = lease;
            let _ = rx.recv();
        });
        assert_eq!(active_listener_count(), before + 1);
        // A caller timeout does not decrement accounting or cancel the worker.
        assert!(!handle.is_finished());
        tx.send(()).unwrap();
        handle.join().unwrap();
        assert_eq!(active_listener_count(), before);
    }
}

/// Dispatch admission, shared by Carbon fallback and the native manager.
pub fn events_allowed() -> bool {
    allowed(true)
}
