//! 仅管理动态 Cancel 键：一个后台执行者完成全部 OS 操作，合并最新期望状态。

use super::{handy_keys, tauri_impl};
#[cfg(not(target_os = "linux"))]
use crate::settings::get_settings;
use crate::settings::{KeyboardImplementation, ShortcutBinding};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Manager};

#[derive(Clone, Debug)]
struct Target {
    backend: KeyboardImplementation,
    binding: ShortcutBinding,
}

impl PartialEq for Target {
    fn eq(&self, other: &Self) -> bool {
        self.backend == other.backend
            && self.binding.id == other.binding.id
            && self.binding.current_binding == other.binding.current_binding
    }
}

#[derive(Clone, Copy, Default)]
struct Desired {
    active: bool,
    epoch: u64,
    shutdown: bool,
    idle_epoch: Option<u64>,
}

#[derive(Default)]
struct Shared {
    desired: Mutex<Desired>,
    changed: Condvar,
}

impl Shared {
    fn update(&self, active: Option<bool>) {
        if let Ok(mut desired) = self.desired.lock() {
            if let Some(active) = active {
                desired.active = active;
            }
            desired.epoch = desired.epoch.wrapping_add(1);
            desired.idle_epoch = None;
            self.changed.notify_all();
        }
    }
}

trait Backend: Send + 'static {
    fn resolve(&mut self, active: bool) -> Result<Option<Target>, String>;
    fn register(&mut self, target: &Target) -> Result<(), String>;
    fn unregister(&mut self, target: &Target) -> Result<(), String>;
}

fn run(shared: Arc<Shared>, mut backend: impl Backend) {
    let mut applied_epoch = Some(0);
    let mut installed: Option<Target> = None;
    let mut retry = false;
    loop {
        let desired = {
            let Ok(mut desired) = shared.desired.lock() else {
                return;
            };
            while !desired.shutdown && Some(desired.epoch) == applied_epoch {
                if retry {
                    let Ok((next, _)) = shared
                        .changed
                        .wait_timeout(desired, Duration::from_millis(250))
                    else {
                        return;
                    };
                    desired = next;
                    break;
                }
                let Ok(next) = shared.changed.wait(desired) else {
                    return;
                };
                desired = next;
            }
            if desired.shutdown {
                return;
            }
            *desired
        };
        let resolved = backend.resolve(desired.active);
        let Ok(latest) = shared.desired.lock() else {
            return;
        };
        if latest.epoch != desired.epoch || latest.shutdown {
            continue;
        }
        drop(latest);
        let target = match resolved {
            Ok(target) => target,
            Err(error) => {
                log::warn!("Cancel shortcut resolution failed: {error}");
                applied_epoch = Some(desired.epoch);
                retry = true;
                continue;
            }
        };
        // 每轮最多一个同步副作用。返回后立即重新读取最新 epoch，旧操作完成
        // 不能越过后来的注册/注销；注销未确认成功时不开始新 backend 注册。
        if installed != target {
            let result = if let Some(old) = &installed {
                backend.unregister(old).map(|_| {
                    installed = None;
                })
            } else if let Some(new) = target {
                backend.register(&new).map(|_| {
                    installed = Some(new);
                })
            } else {
                Ok(())
            };
            if let Err(error) = result {
                log::warn!("Cancel shortcut reconciliation failed: {error}");
                applied_epoch = Some(desired.epoch);
                retry = true;
            } else {
                // 不把本轮标为收敛：必须用最新 desired 再验证一次。
                retry = false;
                applied_epoch = None;
            }
            continue;
        }
        applied_epoch = Some(desired.epoch);
        retry = false;
        if installed.is_none() && !desired.active {
            if let Ok(mut current) = shared.desired.lock() {
                if current.epoch == desired.epoch && !current.active {
                    current.idle_epoch = Some(desired.epoch);
                    shared.changed.notify_all();
                }
            }
        }
    }
}

struct LiveBackend {
    app: AppHandle,
    fallback_active: Option<bool>,
}

impl Backend for LiveBackend {
    fn resolve(&mut self, active: bool) -> Result<Option<Target>, String> {
        let active = active && crate::input_permission::capture_epoch().is_ok();
        if self.fallback_active != Some(active) {
            if active {
                crate::secure_input::register_cancel_fallback(&self.app);
            } else {
                crate::secure_input::unregister_cancel_fallback(&self.app);
            }
            self.fallback_active = Some(active);
        }
        // checked 接口将取锁和 Carbon 操作整体派发到主线程；本 worker 无锁等待
        // 清理屏障，避免 backend 切换时把旧 fallback 误当成新的 Tauri Cancel。
        crate::secure_input::reconcile_fallback_checked(&self.app)?;
        #[cfg(target_os = "linux")]
        {
            Ok(None)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let settings = get_settings(&self.app);
            Ok(if active {
                settings
                    .bindings
                    .get("cancel")
                    .cloned()
                    .map(|binding| Target {
                        backend: settings.keyboard_implementation,
                        binding,
                    })
            } else {
                None
            })
        }
    }

    fn register(&mut self, target: &Target) -> Result<(), String> {
        crate::input_permission::capture_epoch()?;
        match target.backend {
            KeyboardImplementation::Tauri => {
                tauri_impl::register_shortcut(&self.app, target.binding.clone())
            }
            KeyboardImplementation::HandyKeys => {
                handy_keys::register_shortcut(&self.app, target.binding.clone())
            }
        }
    }

    fn unregister(&mut self, target: &Target) -> Result<(), String> {
        if target.backend == KeyboardImplementation::Tauri {
            use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};
            let key = target
                .binding
                .current_binding
                .parse::<Shortcut>()
                .map_err(|error| error.to_string())?;
            if !self.app.global_shortcut().is_registered(key) {
                return Ok(());
            }
        }
        match target.backend {
            KeyboardImplementation::Tauri => {
                tauri_impl::unregister_shortcut(&self.app, target.binding.clone())
            }
            KeyboardImplementation::HandyKeys => {
                handy_keys::unregister_shortcut(&self.app, target.binding.clone())
            }
        }
    }
}

struct Worker {
    shared: Arc<Shared>,
}
static INITIALIZE: Mutex<()> = Mutex::new(());

fn shared(app: &AppHandle) -> Arc<Shared> {
    if let Some(worker) = app.try_state::<Worker>() {
        return worker.shared.clone();
    }
    // 仅保护一次性的轻量 worker 安装，不持有此锁等待任何 OS 注册。
    let _initialize = INITIALIZE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    if let Some(worker) = app.try_state::<Worker>() {
        return worker.shared.clone();
    }
    let shared = Arc::new(Shared::default());
    app.manage(Worker {
        shared: shared.clone(),
    });
    let worker_shared = shared.clone();
    let backend = LiveBackend {
        app: app.clone(),
        fallback_active: None,
    };
    std::thread::spawn(move || run(worker_shared, backend));
    shared
}

pub(super) fn set_active(app: &AppHandle, active: bool) {
    shared(app).update(Some(active));
}

/// 调用方已经持有协调器空闲屏障；必须确认 worker 自己撤销了 Cancel，
/// 不能把外部 unregister_all 当作其 installed 缓存的回执。
pub(super) fn confirm_idle(app: &AppHandle) -> Result<(), String> {
    let Some(worker) = app.try_state::<Worker>() else {
        return Ok(());
    };
    confirm_shared_idle(&worker.shared, Duration::from_millis(750))
}

fn confirm_shared_idle(shared: &Shared, timeout: Duration) -> Result<(), String> {
    let mut desired = shared
        .desired
        .lock()
        .map_err(|_| "cancel_state_unavailable")?;
    if desired.active || desired.shutdown {
        return Err("cancel_still_active".into());
    }
    desired.epoch = desired.epoch.wrapping_add(1);
    desired.idle_epoch = None;
    let epoch = desired.epoch;
    shared.changed.notify_all();
    let (desired, _) = shared
        .changed
        .wait_timeout_while(desired, timeout, |state| {
            !state.active
                && !state.shutdown
                && state.epoch == epoch
                && state.idle_epoch != Some(epoch)
        })
        .map_err(|_| "cancel_state_unavailable")?;
    if desired.active
        || desired.shutdown
        || desired.epoch != epoch
        || desired.idle_epoch != Some(epoch)
    {
        return Err("cancel_retirement_unconfirmed".into());
    }
    Ok(())
}

pub(super) fn refresh(app: &AppHandle) {
    // 没有录音生命周期请求时不为设置切换额外创建 worker。
    if let Some(worker) = app.try_state::<Worker>() {
        worker.shared.update(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::thread::JoinHandle;

    fn target(backend: KeyboardImplementation, key: &str) -> Target {
        Target {
            backend,
            binding: ShortcutBinding {
                id: "cancel".into(),
                name: "synthetic".into(),
                description: "synthetic".into(),
                default_binding: "Escape".into(),
                current_binding: key.into(),
            },
        }
    }

    struct FakeBackend {
        target: Arc<Mutex<Target>>,
        installed: Arc<Mutex<Vec<Target>>>,
        events: Sender<String>,
        release: Receiver<()>,
        block_on: Option<&'static str>,
        fail_unregister: bool,
    }

    impl FakeBackend {
        fn begin(&mut self, operation: &'static str, target: &Target) {
            self.events
                .send(format!(
                    "{operation}:{:?}:{}",
                    target.backend, target.binding.current_binding
                ))
                .unwrap();
            if self.block_on == Some(operation) {
                self.block_on = None;
                self.release.recv_timeout(Duration::from_secs(2)).unwrap();
            }
        }
    }

    impl Backend for FakeBackend {
        fn resolve(&mut self, active: bool) -> Result<Option<Target>, String> {
            Ok(active.then(|| self.target.lock().unwrap().clone()))
        }
        fn register(&mut self, target: &Target) -> Result<(), String> {
            self.begin("register", target);
            self.installed.lock().unwrap().push(target.clone());
            self.events.send("registered".into()).unwrap();
            Ok(())
        }
        fn unregister(&mut self, target: &Target) -> Result<(), String> {
            self.begin("unregister", target);
            if self.fail_unregister {
                self.fail_unregister = false;
                self.events.send("unregister-failed".into()).unwrap();
                return Err("synthetic refusal".into());
            }
            self.installed.lock().unwrap().retain(|item| item != target);
            self.events.send("unregistered".into()).unwrap();
            Ok(())
        }
    }

    struct Harness {
        shared: Arc<Shared>,
        target: Arc<Mutex<Target>>,
        installed: Arc<Mutex<Vec<Target>>>,
        events: Receiver<String>,
        release: Sender<()>,
        worker: Option<JoinHandle<()>>,
    }
    impl Harness {
        fn new(block_on: Option<&'static str>, fail_unregister: bool) -> Self {
            let shared = Arc::new(Shared::default());
            let target = Arc::new(Mutex::new(target(KeyboardImplementation::Tauri, "Escape")));
            let installed = Arc::new(Mutex::new(Vec::new()));
            let (events, received) = mpsc::channel();
            let (release, released) = mpsc::channel();
            let backend = FakeBackend {
                target: target.clone(),
                installed: installed.clone(),
                events,
                release: released,
                block_on,
                fail_unregister,
            };
            let worker_shared = shared.clone();
            let worker = std::thread::spawn(move || run(worker_shared, backend));
            Self {
                shared,
                target,
                installed,
                events: received,
                release,
                worker: Some(worker),
            }
        }
        fn expect(&self, expected: &str) {
            assert_eq!(
                self.events.recv_timeout(Duration::from_secs(2)).unwrap(),
                expected
            );
        }
        fn quiet(&self) {
            assert!(matches!(
                self.events.recv_timeout(Duration::from_millis(20)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ));
        }
    }
    impl Drop for Harness {
        fn drop(&mut self) {
            self.shared.desired.lock().unwrap().shutdown = true;
            self.shared.changed.notify_one();
            let _ = self.release.send(());
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }

    #[test]
    fn idle_receipt_waits_for_actual_unregister_not_just_inactive_desire() {
        let h = Harness::new(Some("unregister"), false);
        h.shared.update(Some(true));
        h.expect("register:Tauri:Escape");
        h.expect("registered");
        assert!(confirm_shared_idle(&h.shared, Duration::from_millis(5)).is_err());
        h.shared.update(Some(false));
        h.expect("unregister:Tauri:Escape");
        assert!(confirm_shared_idle(&h.shared, Duration::from_millis(5)).is_err());
        assert_eq!(h.installed.lock().unwrap().len(), 1);
        h.release.send(()).unwrap();
        h.expect("unregistered");
        confirm_shared_idle(&h.shared, Duration::from_secs(1)).unwrap();
        assert!(h.installed.lock().unwrap().is_empty());
    }

    #[test]
    fn stopped_during_blocked_register_does_not_leave_escape_registered() {
        let h = Harness::new(Some("register"), false);
        h.shared.update(Some(true));
        h.expect("register:Tauri:Escape");
        h.shared.update(Some(false));
        h.quiet();
        h.release.send(()).unwrap();
        h.expect("registered");
        h.expect("unregister:Tauri:Escape");
        h.expect("unregistered");
        assert!(h.installed.lock().unwrap().is_empty());
        h.quiet();
    }

    #[test]
    fn latest_start_survives_older_blocked_unregister_and_coalesces_requests() {
        let h = Harness::new(Some("unregister"), false);
        h.shared.update(Some(true));
        h.expect("register:Tauri:Escape");
        h.expect("registered");
        h.shared.update(Some(false));
        h.expect("unregister:Tauri:Escape");
        h.shared.update(Some(true));
        h.shared.update(Some(false));
        h.shared.update(Some(true));
        h.quiet();
        h.release.send(()).unwrap();
        h.expect("unregistered");
        h.expect("register:Tauri:Escape");
        h.expect("registered");
        assert_eq!(h.installed.lock().unwrap().len(), 1);
        h.quiet();
    }

    #[test]
    fn backend_switch_cleans_original_backend_even_after_its_register_finishes_late() {
        let h = Harness::new(Some("register"), false);
        h.shared.update(Some(true));
        h.expect("register:Tauri:Escape");
        *h.target.lock().unwrap() = target(KeyboardImplementation::HandyKeys, "Escape");
        h.shared.update(None);
        h.release.send(()).unwrap();
        h.expect("registered");
        h.expect("unregister:Tauri:Escape");
        h.expect("unregistered");
        h.expect("register:HandyKeys:Escape");
        h.expect("registered");
        assert_eq!(
            *h.installed.lock().unwrap(),
            vec![target(KeyboardImplementation::HandyKeys, "Escape")]
        );
        h.quiet();
    }

    #[test]
    fn binding_change_and_backend_rollback_do_not_keep_old_registrations() {
        let h = Harness::new(None, false);
        h.shared.update(Some(true));
        h.expect("register:Tauri:Escape");
        h.expect("registered");
        *h.target.lock().unwrap() = target(KeyboardImplementation::HandyKeys, "Shift+Escape");
        h.shared.update(None);
        h.expect("unregister:Tauri:Escape");
        h.expect("unregistered");
        h.expect("register:HandyKeys:Shift+Escape");
        h.expect("registered");
        *h.target.lock().unwrap() = target(KeyboardImplementation::Tauri, "Escape");
        h.shared.update(None);
        h.expect("unregister:HandyKeys:Shift+Escape");
        h.expect("unregistered");
        h.expect("register:Tauri:Escape");
        h.expect("registered");
        assert_eq!(
            *h.installed.lock().unwrap(),
            vec![target(KeyboardImplementation::Tauri, "Escape")]
        );
    }

    #[test]
    fn failed_old_unregister_is_retried_before_registering_new_backend() {
        let h = Harness::new(None, true);
        h.shared.update(Some(true));
        h.expect("register:Tauri:Escape");
        h.expect("registered");
        *h.target.lock().unwrap() = target(KeyboardImplementation::HandyKeys, "Escape");
        h.shared.update(None);
        h.expect("unregister:Tauri:Escape");
        h.expect("unregister-failed");
        h.expect("unregister:Tauri:Escape");
        h.expect("unregistered");
        h.expect("register:HandyKeys:Escape");
        h.expect("registered");
        assert_eq!(h.installed.lock().unwrap().len(), 1);
    }

    #[test]
    fn repeated_desired_state_does_not_reregister_or_unregister_new_session() {
        let h = Harness::new(None, false);
        h.shared.update(Some(true));
        h.expect("register:Tauri:Escape");
        h.expect("registered");
        for _ in 0..10000 {
            h.shared.update(Some(true));
        }
        h.quiet();
        assert_eq!(h.installed.lock().unwrap().len(), 1);
        h.shared.update(Some(false));
        h.expect("unregister:Tauri:Escape");
        h.expect("unregistered");
        for _ in 0..10000 {
            h.shared.update(Some(false));
        }
        h.quiet();
        assert!(h.installed.lock().unwrap().is_empty());
    }
}
