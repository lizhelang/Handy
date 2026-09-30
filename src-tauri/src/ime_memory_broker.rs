//! 主线程 WordSpan 许可与后台精确撤销；原字段正文不进入恢复队列。
use super::*;
use inputia_handy_runtime::{
    memory_commit::CommitIdentity,
    memory_word_span::{
        PreparedWordSpan, RevokedWordSpan, SpanEdit, SpanProgress, VerifiedWordSpan,
        WordSpanCheckpoint,
    },
    service::HistoryService,
};
use tauri::Manager;

fn pending() -> &'static std::sync::Mutex<BTreeMap<String, RevokedWordSpan>> {
    static PENDING: std::sync::OnceLock<std::sync::Mutex<BTreeMap<String, RevokedWordSpan>>> =
        std::sync::OnceLock::new();
    PENDING.get_or_init(Default::default)
}
pub(super) fn retain_revocations(tokens: Vec<RevokedWordSpan>) {
    // 中毒时保留已恢复内容；队列仅含服务产生的精确身份，没有正文或外部动作。
    let mut queue = pending()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for token in tokens {
        queue.insert(token.operation_id().into(), token);
    }
}
fn identity(
    owner: &str,
    server: &str,
    target: &HostTargetToken,
    policy_epoch: u64,
) -> Result<CommitIdentity, String> {
    Ok(CommitIdentity {
        client_instance: owner.into(),
        server_instance: server.into(),
        permission_epoch: crate::input_permission::capture_epoch()?,
        policy_epoch,
        target: target.clone(),
    })
}
fn validate(broker: &mut Broker, identity: &CommitIdentity) -> Result<(), String> {
    broker
        .validate(
            &identity.client_instance,
            &identity.server_instance,
            &identity.target,
            identity.permission_epoch,
            TargetBridgePurpose::Personalization,
            None,
        )
        .map(|_| ())
}

pub fn flush_word_span_revocations(
    app: &tauri::AppHandle,
    history: &HistoryService,
) -> Result<bool, String> {
    let active = on_main(app, |broker| {
        retain_revocations(broker.memory_spans.collect_expired());
        Ok(broker.memory_spans.has_pending_or_active())
    })?;
    let tokens: Vec<_> = pending()
        .lock()
        .map_err(|_| "memory_span_revocation_pending")?
        .values()
        .cloned()
        .collect();
    for token in tokens {
        history.memory_revoke_word_span(token.clone())?;
        let acknowledge = token.clone();
        on_main(app, move |broker| {
            broker.memory_spans.acknowledge_revocation(&acknowledge);
            Ok(())
        })?;
        pending()
            .lock()
            .map_err(|_| "memory_span_revocation_pending")?
            .remove(token.operation_id());
    }
    Ok(active
        || !pending()
            .lock()
            .map_err(|_| "memory_span_revocation_pending")?
            .is_empty())
}
#[derive(Default)]
struct RecoverySignal {
    running: bool,
    generation: u64,
}
fn signal() -> &'static std::sync::Mutex<RecoverySignal> {
    static SIGNAL: std::sync::OnceLock<std::sync::Mutex<RecoverySignal>> =
        std::sync::OnceLock::new();
    SIGNAL.get_or_init(Default::default)
}
fn start_recovery(app: &tauri::AppHandle) -> Result<(), String> {
    {
        let mut state = signal()
            .lock()
            .map_err(|_| "memory_span_recovery_unavailable")?;
        state.generation = state
            .generation
            .checked_add(1)
            .ok_or("memory_span_recovery_unavailable")?;
        if state.running {
            return Ok(());
        }
        state.running = true;
    }
    let app = app.clone();
    let result = std::thread::Builder::new()
        .name("inputia-memory-spans".into())
        .spawn(move || {
            struct Running(bool);
            impl Drop for Running {
                fn drop(&mut self) {
                    if self.0 {
                        signal().lock().unwrap_or_else(|e| e.into_inner()).running = false;
                    }
                }
            }
            let mut running = Running(true);
            loop {
                std::thread::sleep(Duration::from_millis(250));
                let observed = signal()
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .generation;
                let Some(manager) =
                    app.try_state::<Arc<crate::managers::integration::IntegrationManager>>()
                else {
                    return;
                };
                if matches!(
                    flush_word_span_revocations(&app, &manager.service),
                    Ok(false)
                ) {
                    // 退出与新的prepare/checkpoint通知共用同一把锁，不丢“刚检查完就创建”的唤醒。
                    let mut state = signal().lock().unwrap_or_else(|e| e.into_inner());
                    if state.generation == observed {
                        state.running = false;
                        running.0 = false;
                        return;
                    }
                }
            }
        });
    if result.is_err() {
        signal().lock().unwrap_or_else(|e| e.into_inner()).running = false;
        return Err("memory_span_recovery_unavailable".into());
    }
    Ok(())
}

pub fn prepare_memory_word_span(
    app: &tauri::AppHandle,
    owner: &str,
    server: &str,
    target: &HostTargetToken,
    epoch: u64,
    started: Instant,
) -> Result<PreparedWordSpan, String> {
    if !pending()
        .lock()
        .map_err(|_| "memory_span_revocation_pending")?
        .is_empty()
    {
        return Err("memory_span_revocation_pending".into());
    }
    let identity = identity(owner, server, target, epoch)?;
    // 先启动到期处理，再创建许可，避免线程创建失败留下无人回收的正文。
    start_recovery(app)?;
    let result = on_main(app, move |broker| {
        validate(broker, &identity)?;
        let mut observer = broker.native.learning_observer(&identity.target.target_id);
        let permit = broker
            .memory_spans
            .prepare(identity.clone(), started, &mut observer)
            .map_err(|e| e.to_string())?;
        if let Err(error) = validate(broker, &identity) {
            retain_revocations(
                broker
                    .memory_spans
                    .retire_target(&identity.target.target_id),
            );
            return Err(error);
        }
        Ok(permit)
    });
    start_recovery(app)?;
    result
}
pub fn record_memory_word_span(
    app: &tauri::AppHandle,
    owner: &str,
    server: &str,
    target: &HostTargetToken,
    epoch: u64,
    span_id: String,
    sequence: u64,
    edit: SpanEdit,
) -> Result<SpanProgress, String> {
    let identity = identity(owner, server, target, epoch)?;
    on_main(app, move |broker| {
        validate(broker, &identity)?;
        broker
            .memory_spans
            .record(&identity, &span_id, sequence, edit)
            .map_err(|e| e.to_string())
    })
}
pub fn checkpoint_memory_word_span(
    app: &tauri::AppHandle,
    owner: &str,
    server: &str,
    target: &HostTargetToken,
    epoch: u64,
    span_id: String,
    request: WordSpanCheckpoint,
    started: Instant,
) -> Result<VerifiedWordSpan, String> {
    let identity = identity(owner, server, target, epoch)?;
    start_recovery(app)?;
    let result = on_main(app, move |broker| {
        validate(broker, &identity)?;
        let mut observer = broker.native.learning_observer(&identity.target.target_id);
        let evidence = broker
            .memory_spans
            .checkpoint(&identity, &span_id, &request, started, &mut observer)
            .map_err(|e| e.to_string())?;
        validate(broker, &identity)?;
        Ok(evidence)
    });
    start_recovery(app)?;
    result
}
pub fn retire_memory_word_span(
    app: &tauri::AppHandle,
    owner: &str,
    server: &str,
    span_id: String,
) -> Result<(), String> {
    let (owner, server) = (owner.to_owned(), server.to_owned());
    start_recovery(app)?;
    on_main(app, move |broker| {
        let token = broker
            .memory_spans
            .retire_span(&owner, &server, &span_id)
            .map_err(|e| e.to_string())?;
        retain_revocations(vec![token]);
        Ok(())
    })
}

/// 仅在域已耐久确认相同封段操作后退休临时证明；超时由精确撤销流程核对真实sealed记录。
pub fn acknowledge_memory_word_span(
    app: &tauri::AppHandle,
    proof: VerifiedWordSpan,
) -> Result<bool, String> {
    on_main(app, move |broker| {
        Ok(broker.memory_spans.acknowledge_seal(&proof))
    })
}
