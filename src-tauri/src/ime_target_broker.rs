//! Authenticated IME target capabilities. AX ownership stays on the app main run loop.
use crate::unified_target::{self, PendingReason, ProcessInstance, TargetRegistry};
use inputia_handy_runtime::voice_protocol::{
    HostTargetToken, TargetBridgeCommand, TargetBridgePurpose, TargetBridgeReply,
    TargetBridgeRequest, TargetBridgeSelection,
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    io::Read,
    sync::{
        atomic::{AtomicBool, AtomicU8, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

const LEASE_TTL: Duration = Duration::from_secs(120);
const PROOF_MS: u64 = 250;
const CALL_TIMEOUT: Duration = Duration::from_millis(900);

struct Grant {
    owner: String,
    server: String,
    target: HostTargetToken,
    permission_epoch: u64,
    process: ProcessInstance,
    expires: Instant,
    // Bounded by one delivery per captured voice target. Repeated validation is not a renewal.
    dispatched: Option<String>,
}
impl Grant {
    fn authorize(
        &self,
        owner: &str,
        server: &str,
        target: &HostTargetToken,
        epoch: u64,
        now: Instant,
    ) -> Result<(), String> {
        if self.owner != owner || self.server != server || self.target != *target {
            return Err("target_owner_mismatch".into());
        }
        if self.permission_epoch != epoch || now >= self.expires {
            return Err("target_expired".into());
        }
        Ok(())
    }
    fn claim_dispatch(&mut self, operation: &str) -> Result<String, String> {
        if operation.is_empty() || self.dispatched.is_some() {
            return Err("target_dispatch_replayed".into());
        }
        let nonce = opaque()?;
        self.dispatched = Some(operation.into());
        Ok(nonce)
    }
}
struct Broker {
    native: TargetRegistry,
    grants: BTreeMap<String, Grant>,
    memory_commits: inputia_handy_runtime::memory_commit::CommitRegistry,
    memory_spans: inputia_handy_runtime::memory_word_span::WordSpanRegistry,
}
#[path = "ime_memory_broker.rs"]
mod memory_spans;
pub use memory_spans::{
    acknowledge_memory_word_span, checkpoint_memory_word_span, flush_word_span_revocations,
    prepare_memory_word_span, record_memory_word_span, retire_memory_word_span,
};

impl Drop for Broker {
    fn drop(&mut self) {
        memory_spans::retain_revocations(self.memory_spans.revoke_all());
    }
}
thread_local! { static BROKER: RefCell<Option<Broker>> = const { RefCell::new(None) }; }

fn opaque() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|_| "target_entropy_unavailable")?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
fn reason(error: PendingReason) -> String {
    format!("target_{error:?}").to_lowercase()
}
fn checked_source(source: &str) -> Result<ProcessInstance, String> {
    let (process, title) = unified_target::foreground_source(source).map_err(reason)?;
    if inputia_core::AppPolicy::default()
        .excludes(&inputia_core::AppContext::new(source).with_window_title(Some(title)))
    {
        return Err("target_sensitive_source".into());
    }
    Ok(process)
}
fn history_only_capture_failure(error: PendingReason) -> bool {
    matches!(
        error,
        PendingReason::UnsupportedControl | PendingReason::UnobservableControl
    )
}
impl Broker {
    fn new() -> Result<Self, String> {
        Ok(Self {
            native: TargetRegistry::new().map_err(reason)?,
            grants: BTreeMap::new(),
            memory_commits: Default::default(),
            memory_spans: Default::default(),
        })
    }
    fn prune(&mut self, epoch: u64) {
        let expired: Vec<_> = self
            .grants
            .iter()
            .filter(|(_, grant)| grant.expires <= Instant::now() || grant.permission_epoch != epoch)
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            self.release_id(&id);
        }
        let _ = self.native.prune();
    }
    fn release_id(&mut self, id: &str) {
        self.memory_commits.retire_target(id);
        memory_spans::retain_revocations(self.memory_spans.retire_target(id));
        self.grants.remove(id);
        let _ = self.native.forget(id);
    }
    fn capture(
        &mut self,
        owner: &str,
        server: &str,
        mut draft: HostTargetToken,
        epoch: u64,
    ) -> Result<(HostTargetToken, Option<TargetBridgeSelection>), String> {
        crate::input_permission::check_epoch(epoch)?;
        self.prune(epoch);
        if self.grants.len() >= 32 {
            return Err("target_registry_full".into());
        }
        let source = draft.source_app.as_deref().ok_or("target_source_missing")?;
        let process = checked_source(source)?;
        if inputia_core::AppPolicy::default().excludes(&inputia_core::AppContext::new(source)) {
            return Err("target_sensitive_source".into());
        }
        let captured = self.native.capture(LEASE_TTL);
        let (id, selection) = match captured {
            Ok(snapshot) => {
                if snapshot.process != process {
                    let _ = self.native.forget(&snapshot.opaque_id);
                    return Err("target_source_changed".into());
                }
                (
                    snapshot.opaque_id,
                    Some(TargetBridgeSelection {
                        location: snapshot.selection.location as i64,
                        length: snapshot.selection.length as i64,
                    }),
                )
            }
            // A foreground-only proof may start history-only recording. It can never dispatch or disclose shared terms.
            Err(error) if history_only_capture_failure(error) => {
                if checked_source(source)? != process {
                    return Err("target_source_changed".into());
                }
                (opaque()?, None)
            }
            Err(error) => return Err(reason(error)),
        };
        if let Err(error) = crate::input_permission::check_epoch(epoch) {
            let _ = self.native.forget(&id);
            return Err(error);
        }
        draft.target_id = id.clone();
        draft.field_id = selection.as_ref().map(|_| id.clone());
        draft.host_instance = owner.into();
        self.grants.insert(
            id,
            Grant {
                owner: owner.into(),
                server: server.into(),
                target: draft.clone(),
                permission_epoch: epoch,
                process,
                expires: Instant::now() + LEASE_TTL,
                dispatched: None,
            },
        );
        Ok((draft, selection))
    }
    fn validate(
        &mut self,
        owner: &str,
        server: &str,
        target: &HostTargetToken,
        epoch: u64,
        purpose: TargetBridgePurpose,
        operation: Option<&str>,
    ) -> Result<Option<String>, String> {
        crate::input_permission::check_epoch(epoch)?;
        self.prune(epoch);
        let grant = self
            .grants
            .get_mut(&target.target_id)
            .ok_or("target_unknown")?;
        grant.authorize(owner, server, target, epoch, Instant::now())?;
        let source = target
            .source_app
            .as_deref()
            .ok_or("target_source_missing")?;
        if checked_source(source)? != grant.process {
            return Err("target_process_changed".into());
        }
        if target.field_id.is_some() {
            if matches!(
                purpose,
                TargetBridgePurpose::TypedCapture | TargetBridgePurpose::Personalization
            ) {
                self.native
                    .validate_typed_field(&target.target_id)
                    .map_err(reason)?;
            } else {
                self.native.validate(&target.target_id).map_err(reason)?;
            }
        } else if purpose != TargetBridgePurpose::Start {
            return Err("target_history_only".into());
        }
        crate::input_permission::check_epoch(epoch)?;
        if purpose == TargetBridgePurpose::Dispatch {
            return grant
                .claim_dispatch(operation.ok_or("target_operation_missing")?)
                .map(Some);
        }
        Ok(None)
    }
}

fn on_main<T: Send + 'static>(
    app: &tauri::AppHandle,
    action: impl FnOnce(&mut Broker) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    static PENDING: AtomicBool = AtomicBool::new(false);
    struct Pending;
    impl Drop for Pending {
        fn drop(&mut self) {
            PENDING.store(false, Ordering::Release);
        }
    }
    if PENDING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err("target_query_busy".into());
    }
    let pending = Pending;
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    // 0 queued, 1 running, 2 reply delivered, 3 timed out.
    let claim = Arc::new(AtomicU8::new(0));
    let work_claim = claim.clone();
    let deadline = Instant::now() + CALL_TIMEOUT;
    app.run_on_main_thread(move || {
        let _pending = pending;
        if Instant::now() >= deadline
            || work_claim
                .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
        {
            return;
        }
        BROKER.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.is_none() {
                match Broker::new() {
                    Ok(value) => *slot = Some(value),
                    Err(error) => {
                        let _ = tx.send(Err(error));
                        return;
                    }
                }
            }
            let broker = slot.as_mut().expect("broker initialized above");
            let before: BTreeSet<_> = broker.grants.keys().cloned().collect();
            let result =
                unified_target::with_query_budget(Duration::from_millis(300), || action(broker));
            // A timed-out capture cannot leak a capability/observer into the next request.
            if work_claim.load(Ordering::SeqCst) == 3 || Instant::now() >= deadline {
                broker.memory_commits.clear();
                memory_spans::retain_revocations(broker.memory_spans.revoke_all());
                let late: Vec<_> = broker
                    .grants
                    .keys()
                    .filter(|id| !before.contains(*id))
                    .cloned()
                    .collect();
                for id in late {
                    broker.release_id(&id);
                }
                return;
            }
            if tx.send(result).is_err()
                || work_claim
                    .compare_exchange(1, 2, Ordering::SeqCst, Ordering::SeqCst)
                    .is_err()
            {
                broker.memory_commits.clear();
                memory_spans::retain_revocations(broker.memory_spans.revoke_all());
                let late: Vec<_> = broker
                    .grants
                    .keys()
                    .filter(|id| !before.contains(*id))
                    .cloned()
                    .collect();
                for id in late {
                    broker.release_id(&id);
                }
            }
        });
    })
    .map_err(|_| "target_main_unavailable")?;
    match rx.recv_timeout(CALL_TIMEOUT) {
        Ok(result) => result,
        Err(_) => {
            if claim.swap(3, Ordering::SeqCst) == 2 {
                // Publishing state 2 happens after the bounded channel send, so this cannot wait.
                rx.try_recv()
                    .unwrap_or_else(|_| Err("target_query_timeout".into()))
            } else {
                Err("target_query_timeout".into())
            }
        }
    }
}

pub fn check(
    app: &tauri::AppHandle,
    owner: &str,
    server: &str,
    target: &HostTargetToken,
    purpose: TargetBridgePurpose,
) -> Result<(), String> {
    let epoch = crate::input_permission::capture_epoch()?;
    let (owner, server, target) = (owner.to_owned(), server.to_owned(), target.clone());
    // Admission checks do not consume the dispatch nonce. Only the explicit operation-bound Validate does.
    on_main(app, move |broker| {
        broker
            .validate(&owner, &server, &target, epoch, purpose, None)
            .map(|_| ())
    })
}

/// 提交许可始终预取于后台；本函数不得由Host按键回调同步等待。
pub fn prepare_memory_commit(
    app: &tauri::AppHandle,
    owner: &str,
    server: &str,
    target: &HostTargetToken,
    policy_epoch: u64,
    request: inputia_handy_runtime::memory_commit::FixedPlansRequest,
    started: Instant,
) -> Result<inputia_handy_runtime::memory_commit::PreparedFixedCommit, String> {
    use inputia_handy_runtime::memory_commit::CommitIdentity;
    let permission_epoch = crate::input_permission::capture_epoch()?;
    let identity = CommitIdentity {
        client_instance: owner.into(),
        server_instance: server.into(),
        permission_epoch,
        policy_epoch,
        target: target.clone(),
    };
    let prepared = on_main(app, move |broker| {
        broker.validate(
            &identity.client_instance,
            &identity.server_instance,
            &identity.target,
            permission_epoch,
            TargetBridgePurpose::Personalization,
            None,
        )?;
        broker.memory_commits.retain_current(&identity);
        let mut observer = broker.native.learning_observer(&identity.target.target_id);
        let prepared = broker
            .memory_commits
            .prepare_fixed(identity.clone(), request, &mut observer)
            .map_err(str::to_owned)?;
        if let Err(error) = broker.validate(
            &identity.client_instance,
            &identity.server_instance,
            &identity.target,
            permission_epoch,
            TargetBridgePurpose::Personalization,
            None,
        ) {
            broker
                .memory_commits
                .retire_target(&identity.target.target_id);
            return Err(error);
        }
        broker
            .memory_commits
            .constrain_deadline(&prepared.commit_id, started + Duration::from_millis(1_500))
            .map_err(str::to_owned)?;
        Ok(prepared)
    })?;
    let cleanup_app = app.clone();
    let commit_id = prepared.commit_id.clone();
    let remaining = Duration::from_millis(1_500).saturating_sub(started.elapsed());
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(remaining).await;
        let _ = cleanup_app.run_on_main_thread(move || {
            BROKER.with(|slot| {
                if let Some(broker) = slot.borrow_mut().as_mut() {
                    broker.memory_commits.retire_commit(&commit_id);
                }
            });
        });
    });
    Ok(prepared)
}
/// 只选择提交前已经保存的计划；原字段精确读回后才能构造持久学习证据。
pub fn confirm_memory_commit(
    app: &tauri::AppHandle,
    owner: &str,
    server: &str,
    target: &HostTargetToken,
    policy_epoch: u64,
    commit_id: String,
    plan_id: String,
    operation_id: String,
) -> Result<inputia_handy_runtime::memory_commit::ConfirmedCommit, String> {
    use inputia_handy_runtime::memory_commit::CommitIdentity;
    let permission_epoch = crate::input_permission::capture_epoch()?;
    let identity = CommitIdentity {
        client_instance: owner.into(),
        server_instance: server.into(),
        permission_epoch,
        policy_epoch,
        target: target.clone(),
    };
    on_main(app, move |broker| {
        broker.validate(
            &identity.client_instance,
            &identity.server_instance,
            &identity.target,
            permission_epoch,
            TargetBridgePurpose::Personalization,
            None,
        )?;
        broker.memory_commits.retain_current(&identity);
        let mut observer = broker.native.learning_observer(&identity.target.target_id);
        let confirmed = broker
            .memory_commits
            .confirm_fixed(
                &identity,
                &commit_id,
                &plan_id,
                &operation_id,
                &mut observer,
            )
            .map_err(str::to_owned)?;
        broker.validate(
            &identity.client_instance,
            &identity.server_instance,
            &identity.target,
            permission_epoch,
            TargetBridgePurpose::Personalization,
            None,
        )?;
        Ok(confirmed)
    })
}

pub fn respond(app: &tauri::AppHandle, request: TargetBridgeRequest) -> TargetBridgeReply {
    let epoch = crate::input_permission::capture_epoch();
    let mut reply = TargetBridgeReply {
        status: "target_bridge".into(),
        request_id: request.request_id,
        ready: false,
        server_instance: request.server_instance.clone(),
        permission_epoch: epoch.as_ref().copied().unwrap_or(0),
        valid_for_ms: 0,
        target: None,
        selection: None,
        field_instance: None,
        dispatch_nonce: None,
        code: None,
    };
    if let TargetBridgeCommand::Release { target_id } = request.target_bridge {
        let owner = request.client_instance;
        let server = request.server_instance;
        let result = on_main(app, move |broker| {
            if broker
                .grants
                .get(&target_id)
                .is_some_and(|g| g.owner != owner || g.server != server)
            {
                return Err("target_owner_mismatch".into());
            }
            broker.release_id(&target_id);
            Ok(())
        });
        reply.ready = result.is_ok();
        reply.code = result.err();
        return reply;
    }
    let epoch = match epoch {
        Ok(epoch) => epoch,
        Err(error) => {
            reply.code = Some(error);
            return reply;
        }
    };
    let owner = request.client_instance;
    let server = request.server_instance;
    let proof_ms = match &request.target_bridge {
        TargetBridgeCommand::Status => crate::input_permission::proof_valid_for_ms(1000),
        TargetBridgeCommand::Capture { .. } => LEASE_TTL.as_millis() as u64,
        _ => PROOF_MS,
    };
    let result = match request.target_bridge {
        TargetBridgeCommand::Status => Ok((None, None, None, None)),
        TargetBridgeCommand::Capture { draft } => on_main(app, move |broker| {
            let (target, selection) = broker.capture(&owner, &server, draft, epoch)?;
            let field_instance = target
                .field_id
                .as_ref()
                .map(|_| {
                    broker
                        .native
                        .learning_field_instance(&target.target_id)
                        .map_err(reason)
                })
                .transpose()?;
            Ok((Some(target), selection, None, field_instance))
        }),
        TargetBridgeCommand::Validate {
            target,
            purpose,
            operation_id,
        } => on_main(app, move |broker| {
            let nonce = broker.validate(
                &owner,
                &server,
                &target,
                epoch,
                purpose,
                operation_id.as_deref(),
            )?;
            let field_instance = target
                .field_id
                .as_ref()
                .map(|_| {
                    broker
                        .native
                        .learning_field_instance(&target.target_id)
                        .map_err(reason)
                })
                .transpose()?;
            Ok((Some(target), None, nonce, field_instance))
        }),
        TargetBridgeCommand::Release { .. } => unreachable!(),
    };
    match result {
        Ok((target, selection, nonce, field_instance)) => {
            reply.ready = true;
            reply.valid_for_ms = proof_ms;
            reply.target = target;
            reply.selection = selection;
            reply.field_instance = field_instance;
            reply.dispatch_nonce = nonce;
        }
        Err(error) => reply.code = Some(error),
    }
    reply
}

/// Host屏障ACK之外，还要清除此服务持有的提交计划和读回正文。
pub fn clear_memory_commits(
    app: &tauri::AppHandle,
    owner: &str,
    server: &str,
) -> Result<(), String> {
    let (owner, server) = (owner.to_owned(), server.to_owned());
    on_main(app, move |broker| {
        broker.memory_commits.retire_owner(&owner, &server);
        memory_spans::retain_revocations(broker.memory_spans.retire_owner(&owner, &server));
        Ok(())
    })
}

/// Retirement has its own single queue slot: it must run even if an earlier query timed out.
/// Receipt is sent only after every AX observer, lease and owner grant has been dropped on main.
pub fn invalidate_all(app: &tauri::AppHandle) -> Result<(), String> {
    static RETIRING: AtomicBool = AtomicBool::new(false);
    struct Retirement;
    impl Drop for Retirement {
        fn drop(&mut self) {
            RETIRING.store(false, Ordering::Release);
        }
    }
    if RETIRING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Err("target_retirement_pending".into());
    }
    let retirement = Retirement;
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    app.run_on_main_thread(move || {
        let _retirement = retirement;
        BROKER.with(clear_registry);
        crate::integration_output::invalidate_all_targets();
        let _ = tx.send(());
    })
    .map_err(|_| "target_retirement_unavailable")?;
    // A timeout leaves the cleanup queued. It does not publish a successful maintenance ACK.
    rx.recv_timeout(CALL_TIMEOUT)
        .map_err(|_| "target_retirement_timeout_restart_required".into())
}
fn clear_registry<T>(slot: &RefCell<Option<T>>) {
    let previous = slot.borrow_mut().take();
    drop(previous);
}

fn connections() -> &'static std::sync::Mutex<BTreeMap<(String, String), usize>> {
    static CONNECTIONS: std::sync::OnceLock<std::sync::Mutex<BTreeMap<(String, String), usize>>> =
        std::sync::OnceLock::new();
    CONNECTIONS.get_or_init(Default::default)
}
pub fn connected(owner: &str, server: &str) {
    let mut connections = connections().lock().unwrap();
    *connections
        .entry((owner.into(), server.into()))
        .or_default() += 1;
}
pub fn disconnected(app: &tauri::AppHandle, owner: &str, server: &str) {
    let key = (owner.to_owned(), server.to_owned());
    let remaining = {
        let mut connections = connections().lock().unwrap();
        let count = connections
            .get(&key)
            .copied()
            .unwrap_or(1)
            .saturating_sub(1);
        if count == 0 {
            connections.remove(&key);
        } else {
            connections.insert(key.clone(), count);
        }
        count
    };
    if remaining != 0 {
        return;
    }
    let _ = app.run_on_main_thread(move || {
        // A live sibling/reconnection owns the same authenticated process instance.
        if connections().lock().unwrap().contains_key(&key) {
            return;
        }
        BROKER.with(|slot| {
            if let Some(broker) = slot.borrow_mut().as_mut() {
                let ids: Vec<_> = broker
                    .grants
                    .iter()
                    .filter(|(_, g)| (&g.owner, &g.server) == (&key.0, &key.1))
                    .map(|(id, _)| id.clone())
                    .collect();
                for id in ids {
                    broker.release_id(&id);
                }
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    fn grant() -> Grant {
        Grant {
            owner: "ime".into(),
            server: "server".into(),
            permission_epoch: 7,
            process: ProcessInstance {
                pid: 1,
                start_seconds: 2,
                start_microseconds: 3,
            },
            expires: Instant::now() + Duration::from_secs(10),
            dispatched: None,
            target: HostTargetToken {
                target_id: "opaque".into(),
                host_instance: "ime".into(),
                controller_id: "controller".into(),
                activation_generation: 3,
                field_id: Some("opaque".into()),
                selection_generation: 4,
                composition_generation: 5,
                source_app: Some("com.test.editor".into()),
            },
        }
    }
    #[test]
    fn retirement_drops_registry_owners_before_ack_and_is_idempotent() {
        struct NativeOwner(Arc<AtomicBool>);
        impl Drop for NativeOwner {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let dropped = Arc::new(AtomicBool::new(false));
        let registry = RefCell::new(Some(NativeOwner(dropped.clone())));
        clear_registry(&registry);
        assert!(dropped.load(Ordering::SeqCst));
        assert!(registry.borrow().is_none());
        clear_registry(&registry);
        assert!(registry.borrow().is_none());
    }
    #[test]
    fn capture_failure_cannot_create_dispatchable_or_privacy_unknown_target() {
        assert!(history_only_capture_failure(
            PendingReason::UnobservableControl
        ));
        assert!(history_only_capture_failure(
            PendingReason::UnsupportedControl
        ));
        for error in [
            PendingReason::SecureInput,
            PendingReason::UnknownTarget,
            PendingReason::AccessibilityUnavailable,
            PendingReason::FocusChanged,
            PendingReason::Expired,
        ] {
            assert!(!history_only_capture_failure(error));
        }
    }
    #[test]
    fn broker_rejects_other_owner_server_and_modified_identity() {
        let grant = grant();
        assert!(grant
            .authorize("ime", "server", &grant.target, 7, Instant::now())
            .is_ok());
        assert!(grant
            .authorize("other", "server", &grant.target, 7, Instant::now())
            .is_err());
        assert!(grant
            .authorize("ime", "other", &grant.target, 7, Instant::now())
            .is_err());
        let mut target = grant.target.clone();
        target.selection_generation += 1;
        assert!(grant
            .authorize("ime", "server", &target, 7, Instant::now())
            .is_err());
        target = grant.target.clone();
        target.source_app = Some("arbitrary.app".into());
        assert!(grant
            .authorize("ime", "server", &target, 7, Instant::now())
            .is_err());
    }
    #[test]
    fn broker_expiry_and_permission_recovery_never_resurrect_token() {
        let grant = grant();
        assert!(grant
            .authorize("ime", "server", &grant.target, 8, Instant::now())
            .is_err());
        assert!(grant
            .authorize("ime", "server", &grant.target, 7, grant.expires)
            .is_err());
    }
    #[test]
    fn dispatch_claim_is_one_shot_even_with_new_operation_id() {
        let mut grant = grant();
        let nonce = grant.claim_dispatch("operation-one").unwrap();
        assert_eq!(nonce.len(), 64);
        assert!(grant.claim_dispatch("operation-one").is_err());
        assert!(grant.claim_dispatch("operation-two").is_err());
    }
}
