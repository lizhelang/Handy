//! Pure receipt and generation rules; no native permissions or keyboard events.
use std::sync::mpsc::Receiver;
use std::time::Duration;

pub(super) fn registration_current(
    request: u64,
    worker: u64,
    healthy: bool,
    authorized: Result<u64, String>,
) -> bool {
    healthy && request == worker && authorized == Ok(worker)
}

pub(super) fn receipt<T>(receiver: Receiver<T>, timeout: Duration) -> Result<T, String> {
    receiver
        .recv_timeout(timeout)
        .map_err(|_| "快捷键操作回执未知".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_registration_needs_authorization_but_not_business_ready() {
        assert!(registration_current(3, 3, true, Ok(3)));
        assert!(!registration_current(2, 3, true, Ok(3)));
        assert!(!registration_current(3, 3, false, Ok(3)));
        assert!(!registration_current(3, 3, true, Err("revoked".into())));
        assert!(!registration_current(3, 3, true, Ok(4)));
    }
    #[test]
    fn timeout_does_not_cancel_late_worker_side_effect() {
        let (response, receiver) = std::sync::mpsc::channel::<()>();
        let (release, wait) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            wait.recv().unwrap();
            // Native operation can finish after the caller timed out.
            response.send(()).is_err()
        });
        assert!(receipt(receiver, Duration::from_millis(1)).is_err());
        assert!(!worker.is_finished());
        release.send(()).unwrap();
        assert!(worker.join().unwrap());
    }
    #[test]
    fn disconnected_receipt_is_not_success() {
        let (sender, receiver) = std::sync::mpsc::channel::<()>();
        drop(sender);
        assert!(receipt(receiver, Duration::from_millis(1)).is_err());
    }
}
