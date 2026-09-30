//! 本地模型提示的撤销观察器；只持有版本元数据，实际正文生命周期由调用方 pin 保护。
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc, Mutex,
};
use std::time::{Duration, Instant};
use transcribe_cpp::CancelToken;

pub(crate) struct NativePromptWatch {
    cancel: CancelToken,
    valid: Arc<AtomicBool>,
    checked: Arc<Mutex<Instant>>,
    stop: mpsc::Sender<()>,
    worker: Option<std::thread::JoinHandle<()>>,
    watchdog: Option<std::thread::JoinHandle<()>>,
}
impl NativePromptWatch {
    pub(crate) fn start(verify: impl Fn() -> bool + Send + 'static) -> Result<Self, ()> {
        let started = Instant::now();
        if !verify() || started.elapsed() >= Duration::from_secs(2) {
            return Err(());
        }
        let cancel = CancelToken::new();
        let valid = Arc::new(AtomicBool::new(true));
        let checked = Arc::new(Mutex::new(started));
        let (stop, receive) = mpsc::channel();
        let (worker_cancel, worker_valid, worker_checked) =
            (cancel.clone(), valid.clone(), checked.clone());
        let worker = std::thread::Builder::new()
            .name("inputia-native-prompt".into())
            .spawn(move || {
                loop {
                    match receive.recv_timeout(Duration::from_millis(100)) {
                        Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => return,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                    let started = Instant::now();
                    if !verify() || started.elapsed() >= Duration::from_secs(2) {
                        worker_valid.store(false, Ordering::Release);
                        worker_cancel.cancel();
                        return;
                    }
                    let Ok(mut value) = worker_checked.lock() else {
                        worker_valid.store(false, Ordering::Release);
                        worker_cancel.cancel();
                        return;
                    };
                    // 在线验证的排队与执行时间计入租约，绝不从完成时另续两秒。
                    if !worker_valid.load(Ordering::Acquire) {
                        return;
                    }
                    *value = started;
                }
            })
            .map_err(|_| ())?;
        let (deadline_cancel, deadline_valid, deadline_checked) =
            (cancel.clone(), valid.clone(), checked.clone());
        let watchdog = std::thread::Builder::new()
            .name("inputia-prompt-deadline".into())
            .spawn(move || {
                while deadline_valid.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(25));
                    if !deadline_checked
                        .lock()
                        .is_ok_and(|at| at.elapsed() < Duration::from_secs(2))
                    {
                        deadline_valid.store(false, Ordering::Release);
                        deadline_cancel.cancel();
                        return;
                    }
                }
            });
        let watchdog = match watchdog {
            Ok(worker) => worker,
            Err(_) => {
                valid.store(false, Ordering::Release);
                cancel.cancel();
                let _ = stop.send(());
                let _ = worker.join();
                return Err(());
            }
        };
        Ok(Self {
            cancel,
            valid,
            checked,
            stop,
            worker: Some(worker),
            watchdog: Some(watchdog),
        })
    }
    pub(crate) fn cancel_token(&self) -> &CancelToken {
        &self.cancel
    }
    pub(crate) fn current(&self) -> bool {
        let current = self.valid.load(Ordering::Acquire)
            && self
                .checked
                .lock()
                .is_ok_and(|time| time.elapsed() < Duration::from_secs(2));
        if !current {
            self.valid.store(false, Ordering::Release);
            self.cancel.cancel();
        }
        current
    }
}
impl Drop for NativePromptWatch {
    fn drop(&mut self) {
        self.valid.store(false, Ordering::Release);
        self.cancel.cancel();
        let _ = self.stop.send(());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let Some(watchdog) = self.watchdog.take() {
            let _ = watchdog.join();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blocked_verification_cannot_extend_the_native_cancellation_deadline() {
        let (entered, awaiting) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let watch = NativePromptWatch::start(move || {
            if calls.fetch_add(1, Ordering::AcqRel) > 0 {
                let _ = entered.send(());
                let _ = blocked.recv();
            }
            true
        })
        .unwrap();
        awaiting.recv_timeout(Duration::from_secs(1)).unwrap();
        // 模拟原生run仍阻塞，完全不调用current；独立watchdog必须照样撤销token。
        let start = Instant::now();
        while !watch.cancel_token().is_cancelled() && start.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(10));
        }
        let cancelled_while_blocked = watch.cancel_token().is_cancelled();
        release.send(()).unwrap();
        assert!(cancelled_while_blocked);
        assert!(!watch.current());
    }
    #[test]
    fn revocation_cancels_native_token_without_revalidating_a_revoked_watch() {
        let permitted = Arc::new(AtomicBool::new(true));
        let check = permitted.clone();
        let watch = NativePromptWatch::start(move || check.load(Ordering::Acquire)).unwrap();
        assert!(watch.current());
        permitted.store(false, Ordering::Release);
        let start = Instant::now();
        while !watch.cancel_token().is_cancelled() && start.elapsed() < Duration::from_secs(1) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(watch.cancel_token().is_cancelled());
        permitted.store(true, Ordering::Release);
        assert!(!watch.current());
        assert!(NativePromptWatch::start(|| false).is_err());
    }
}
