//! 仅供已认证连接的后台 dispatcher；不拥有录音状态机，不开放 WebView 命令。
//! 持久 claim 提交后只向唯一 Coordinator 发送一次动作。未知回执只能查询。
//! 连接服务仍须把握手 instance 与同一 socket 的 audit 身份绑定，重连时复核，
//! 并验证策略/遗忘清除回执；此模块不能替代该认证握手，也不创建监听端口。

use inputia_handy_runtime::protocol::{Handshake, MAX_FRAME_BYTES};
use inputia_handy_runtime::{
    output_ledger::{OutputAction, OutputIntent, OutputOutcome, OutputOwner, OutputState},
    service::{HistoryService, OutputPermit},
    voice_ledger::SessionRecord,
    voice_protocol::{
        HostOutputReceipt, VoiceCommand, VoiceDelivery, VoiceOutputCommand, VoiceOutputReply,
        VoiceOutputRequest, VoicePeer, VoicePhase, VoiceRequest, VoiceSessionView,
        VoiceTermsVersion,
    },
};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

// 同一Handy进程仅一个录音actor。只锁资格事务至入队，不持锁等待actor回执。
static VOICE_ENQUEUE_ORDER: std::sync::Mutex<()> = std::sync::Mutex::new(());

enum PeerProof {
    Authenticated(crate::native_pair_auth::VerifiedPeer),
    #[cfg(test)]
    Synthetic,
}

pub struct AuthenticatedVoiceConnection {
    client: String,
    server: String,
    applied_version: Option<VoiceTermsVersion>,
    // 持有真实认证结果，不能从请求反序列化此上下文。
    proof: PeerProof,
}

impl AuthenticatedVoiceConnection {
    pub(crate) fn authorize_menu(
        &self,
        request: &inputia_handy_runtime::voice_protocol::MenuRequest,
        history: &HistoryService,
    ) -> Result<(), DispatchError> {
        let version = history
            .voice_terms_version()
            .map_err(|_| DispatchError::Unknown)?;
        request
            .validate_for(&VoicePeer {
                client_instance: &self.client,
                server_instance: &self.server,
                policy_epoch: version.policy_epoch,
                policy_applied: self.applied_version.as_ref() == Some(&version),
            })
            .map_err(|_| DispatchError::Unauthorized)
    }
    /// 发出新屏障或同步失败时立即关闭新Start授权；Stop/Cancel仍能关闭本人会话。
    pub fn invalidate_policy(&mut self) {
        self.applied_version = None;
    }
    /// server 调用者须将握手绑定至同一已认证存活 socket，不能传 VoiceRequest 字段。
    /// 策略应用回执通过同连接单独确认；初始连接没有业务授权。
    pub fn from_verified_peer(
        verified: crate::native_pair_auth::VerifiedPeer,
        client: &Handshake,
        server: &Handshake,
        history: &HistoryService,
    ) -> Result<Self, DispatchError> {
        if verified.role() != crate::native_pair_auth::PeerRole::Inputia
            || client.profile_id != server.profile_id
            || client.protocol_major != 1
            || server.protocol_major != 1
            || !valid_id(&client.instance_id)
            || !valid_id(&server.instance_id)
        {
            return Err(DispatchError::Unauthorized);
        }
        history
            .bind_voice_peer(client.instance_id.clone(), *verified.audit_token())
            .map_err(|_| DispatchError::Unauthorized)?;
        Ok(Self {
            client: client.instance_id.clone(),
            server: server.instance_id.clone(),
            applied_version: None,
            proof: PeerProof::Authenticated(verified),
        })
    }

    /// 仅 server 已验证的策略/遗忘应用回执可调用，不能由普通业务请求更新。
    pub fn acknowledge_policy(
        &mut self,
        applied: VoiceTermsVersion,
        current: &VoiceTermsVersion,
    ) -> Result<(), DispatchError> {
        if &applied != current
            || self.applied_version.as_ref().is_some_and(|old| {
                applied.policy_epoch < old.policy_epoch
                    || applied.learning_generation < old.learning_generation
            })
        {
            return Err(DispatchError::Unauthorized);
        }
        self.applied_version = Some(applied);
        Ok(())
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DispatchError {
    Unauthorized,
    MissingSession,
    /// 不能确定持久请求/派发/事实写入是否完成；禁止重发副作用。
    Unknown,
    CoordinatorRejected,
}

pub trait VoiceCoordinatorPort {
    /// 生产实现必须仅入队：不能等待actor或重入dispatcher，否则会违反入队顺序锁合同。
    fn control(&self, request: VoiceRequest) -> Receiver<Result<VoiceSessionView, String>>;
    fn view(&self, session: &str) -> Receiver<Result<VoiceSessionView, String>>;
}

impl VoiceCoordinatorPort for crate::transcription_coordinator::TranscriptionCoordinator {
    fn control(&self, request: VoiceRequest) -> Receiver<Result<VoiceSessionView, String>> {
        self.control_voice(request)
    }
    fn view(&self, session: &str) -> Receiver<Result<VoiceSessionView, String>> {
        self.voice_session_view(session)
    }
}

pub struct VoiceDispatcher<'a, C: VoiceCoordinatorPort> {
    history: &'a HistoryService,
    coordinator: &'a C,
    reply_timeout: Duration,
}

impl<'a, C: VoiceCoordinatorPort> VoiceDispatcher<'a, C> {
    pub fn new(history: &'a HistoryService, coordinator: &'a C) -> Self {
        Self {
            history,
            coordinator,
            reply_timeout: Duration::from_secs(2),
        }
    }

    fn authorize(
        &self,
        context: &AuthenticatedVoiceConnection,
        request: &VoiceRequest,
    ) -> Result<(), DispatchError> {
        match &context.proof {
            PeerProof::Authenticated(peer)
                if peer.role() != crate::native_pair_auth::PeerRole::Inputia =>
            {
                return Err(DispatchError::Unauthorized);
            }
            _ => {}
        }
        let version = self
            .history
            .voice_terms_version()
            .map_err(|_| DispatchError::Unknown)?;
        if let VoiceCommand::Start { terms, .. } = &request.command {
            // 请求自报最新版本不能替代本连接真正完成的清理回执。
            if context.applied_version.as_ref() != Some(terms) {
                return Err(DispatchError::Unauthorized);
            }
        }
        request
            .validate_for(&VoicePeer {
                client_instance: &context.client,
                server_instance: &context.server,
                policy_epoch: version.policy_epoch,
                policy_applied: context.applied_version.as_ref() == Some(&version),
            })
            .map_err(|_| DispatchError::Unauthorized)
    }

    fn owned_record(
        &self,
        context: &AuthenticatedVoiceConnection,
        session: &str,
    ) -> Result<SessionRecord, DispatchError> {
        if !valid_id(session) {
            return Err(DispatchError::Unauthorized);
        }
        let record = self
            .history
            .voice_session(session.into())
            .map_err(|_| DispatchError::Unknown)?
            .ok_or(DispatchError::MissingSession)?;
        if record.start.client_instance != context.client
            || (!record.retired && record.start.server_instance != context.server)
        {
            return Err(DispatchError::Unauthorized);
        }
        Ok(record)
    }

    /// 超时/断线均不重发；receiver drop 使尚未开始的 Coordinator Start 可以拒绝执行。
    pub fn dispatch(
        &self,
        context: &AuthenticatedVoiceConnection,
        request: VoiceRequest,
    ) -> Result<VoiceSessionView, DispatchError> {
        self.authorize(context, &request)?;
        if matches!(request.command, VoiceCommand::Status) {
            return self.status(context, &request.session_id);
        }
        let order = VOICE_ENQUEUE_ORDER
            .lock()
            .map_err(|_| DispatchError::Unknown)?;
        if !matches!(request.command, VoiceCommand::Start { .. }) {
            self.owned_record(context, &request.session_id)?;
        }
        let record = self
            .history
            .prepare_voice_request(
                request.clone(),
                context.client.clone(),
                context.server.clone(),
                context
                    .applied_version
                    .as_ref()
                    .map(|version| version.policy_epoch),
            )
            .map_err(|_| DispatchError::Unknown)?;
        if record.retired {
            return Ok(record.view);
        }
        let claimed = self
            .history
            .claim_voice_request(
                request.clone(),
                context.client.clone(),
                context.server.clone(),
                context
                    .applied_version
                    .as_ref()
                    .map(|version| version.policy_epoch),
            )
            .map_err(|_| DispatchError::Unknown)?;
        if !claimed {
            drop(order);
            return self.status(context, &request.session_id);
        }
        let is_start = matches!(request.command, VoiceCommand::Start { .. });
        let receiver = self.coordinator.control(request);
        drop(order);
        match receiver.recv_timeout(self.reply_timeout) {
            Ok(Ok(view)) => self.project(context, record, view),
            Ok(Err(_)) => {
                // 仅本次已取得执行权的 Start 收到明确拒绝，才能终结持久 Preparing。
                // 断线/超时没有此事实；Stop/Cancel 拒绝也不能改写已有会话终态。
                if is_start {
                    let mut failed = record.view.clone();
                    failed.generation = failed
                        .generation
                        .checked_add(1)
                        .ok_or(DispatchError::Unknown)?;
                    failed.phase = VoicePhase::Failed;
                    self.project(context, record, failed)?;
                }
                Err(DispatchError::CoordinatorRejected)
            }
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {
                Err(DispatchError::Unknown)
            }
        }
    }

    pub fn fetch_output(
        &self,
        context: &AuthenticatedVoiceConnection,
        request: VoiceOutputRequest,
    ) -> Result<(VoiceOutputReply, Option<(OutputIntent, OutputPermit)>), DispatchError> {
        self.authorize_output_fetch(context, &request)?;
        if !matches!(request.output, VoiceOutputCommand::Fetch {}) {
            return Err(DispatchError::Unauthorized);
        }
        let record = self.owned_record(context, &request.session_id)?;
        if record.retired {
            return Err(DispatchError::MissingSession);
        }
        if self
            .history
            .voice_cancellation_requested(request.session_id.clone())
            .map_err(|_| DispatchError::Unknown)?
        {
            return Err(DispatchError::Unknown);
        }
        let VoiceCommand::Start { target, .. } = &record.start.command else {
            return Err(DispatchError::Unknown);
        };
        if target.field_id.is_none() || record.view.phase != VoicePhase::PendingTarget {
            return Err(DispatchError::Unauthorized);
        }
        let output = self
            .history
            .voice_result(request.session_id.clone())
            .map_err(|_| DispatchError::Unknown)?
            .ok_or(DispatchError::MissingSession)?;
        self.validate_voice_output(&record, &output.intent)?;
        if output.state != OutputState::Prepared {
            return Ok((
                VoiceOutputReply::Output {
                    request_id: request.request_id,
                    operation_id: output.intent.operation_id,
                    state: output.state,
                },
                None,
            ));
        }
        let item = self
            .history
            .indexed_item(output.intent.item_id.clone())
            .map_err(|_| DispatchError::Unknown)?
            .ok_or(DispatchError::Unknown)?;
        if item.revision != output.intent.revision {
            return Err(DispatchError::Unknown);
        }
        let text = item.snapshot.text.ok_or(DispatchError::Unknown)?;
        if text.is_empty() {
            return Err(DispatchError::Unknown);
        }
        let delivery = VoiceDelivery {
            operation_id: output.intent.operation_id.clone(),
            session_id: record.start.session_id.clone(),
            item_id: output.intent.item_id.clone(),
            revision: output.intent.revision,
            policy_epoch: output.intent.policy_epoch,
            target_id: target.target_id.clone(),
            text,
        };
        if delivery.validate().is_err() {
            let _ = self
                .history
                .finish_output(output.intent.clone(), OutputOutcome::NotDispatchedRejected);
            return Err(DispatchError::Unknown);
        }
        let reply = VoiceOutputReply::Delivery {
            request_id: request.request_id.clone(),
            delivery,
        };
        if serde_json::to_vec(&reply)
            .map_err(|_| DispatchError::Unknown)?
            .len()
            > MAX_FRAME_BYTES
        {
            return Err(DispatchError::Unauthorized);
        }
        let Some(permit) = self
            .history
            .claim_output_with_permit(output.intent.clone())
            .map_err(|_| DispatchError::Unknown)?
        else {
            let current = self
                .history
                .output_record(output.intent.operation_id.clone())
                .map_err(|_| DispatchError::Unknown)?
                .ok_or(DispatchError::Unknown)?;
            return Ok((
                VoiceOutputReply::Output {
                    request_id: request.request_id,
                    operation_id: current.intent.operation_id,
                    state: current.state,
                },
                None,
            ));
        };
        Ok((reply, Some((output.intent, permit))))
    }

    pub fn receipt(
        &self,
        context: &AuthenticatedVoiceConnection,
        request: VoiceOutputRequest,
        dispatched: &OutputIntent,
    ) -> Result<VoiceOutputReply, DispatchError> {
        self.authorize_output_receipt(context, &request)?;
        let VoiceOutputCommand::Receipt {
            operation_id,
            receipt,
        } = &request.output
        else {
            return Err(DispatchError::Unauthorized);
        };
        if operation_id != &dispatched.operation_id {
            return Err(DispatchError::Unauthorized);
        }
        let record = self.owned_record(context, &request.session_id)?;
        self.validate_voice_output(&record, dispatched)?;
        let outcome = match receipt {
            HostOutputReceipt::Dispatched => OutputOutcome::DispatchedOnly,
            HostOutputReceipt::PendingTarget => OutputOutcome::NotDispatchedPendingTarget,
            HostOutputReceipt::Uncertain => OutputOutcome::Uncertain,
            HostOutputReceipt::Confirmed => return Err(DispatchError::Unauthorized),
        };
        let output = self
            .history
            .finish_output(dispatched.clone(), outcome)
            .map_err(|_| DispatchError::Unknown)?;
        Ok(VoiceOutputReply::Output {
            request_id: request.request_id,
            operation_id: output.intent.operation_id,
            state: output.state,
        })
    }

    /// 只查询持久事实及 Coordinator 当前视图，不调用 prepare/claim/control。
    pub fn status(
        &self,
        context: &AuthenticatedVoiceConnection,
        session: &str,
    ) -> Result<VoiceSessionView, DispatchError> {
        let record = self.owned_record(context, session)?;
        if record.retired {
            return self.with_output_projection(record.view);
        }
        match self
            .coordinator
            .view(session)
            .recv_timeout(self.reply_timeout)
        {
            Ok(Ok(view)) => {
                if self
                    .history
                    .voice_cancellation_requested(session.into())
                    .map_err(|_| DispatchError::Unknown)?
                    && matches!(
                        view.phase,
                        VoicePhase::Preparing
                            | VoicePhase::Recording
                            | VoicePhase::Processing
                            | VoicePhase::PendingTarget
                    )
                {
                    return Err(DispatchError::Unknown);
                }
                self.project(context, record, view)
            }
            // Coordinator 未观察到该session时返回持久Preparing，而非重新启动。
            Ok(Err(_)) => {
                if self
                    .history
                    .voice_cancellation_requested(session.into())
                    .map_err(|_| DispatchError::Unknown)?
                {
                    Err(DispatchError::Unknown)
                } else {
                    self.with_output_projection(record.view)
                }
            }
            Err(_) => Err(DispatchError::Unknown),
        }
    }

    fn project(
        &self,
        context: &AuthenticatedVoiceConnection,
        record: SessionRecord,
        view: VoiceSessionView,
    ) -> Result<VoiceSessionView, DispatchError> {
        if view.session_id != record.start.session_id {
            return Err(DispatchError::Unknown);
        }
        self.history
            .project_voice_session(context.client.clone(), context.server.clone(), view)
            .map_err(|_| DispatchError::Unknown)?;
        // 写入被旧generation忽略时也返回持久最新事实，不把旧回执回传成当前状态。
        let projected = self.owned_record(context, &record.start.session_id)?.view;
        self.with_output_projection(projected)
    }

    fn authorize_output_fetch(
        &self,
        context: &AuthenticatedVoiceConnection,
        request: &VoiceOutputRequest,
    ) -> Result<(), DispatchError> {
        match &context.proof {
            PeerProof::Authenticated(peer)
                if peer.role() != crate::native_pair_auth::PeerRole::Inputia =>
            {
                return Err(DispatchError::Unauthorized);
            }
            _ => {}
        }
        let version = self
            .history
            .voice_terms_version()
            .map_err(|_| DispatchError::Unknown)?;
        request
            .validate_for(&VoicePeer {
                client_instance: &context.client,
                server_instance: &context.server,
                policy_epoch: version.policy_epoch,
                policy_applied: context.applied_version.as_ref() == Some(&version),
            })
            .map_err(|_| DispatchError::Unauthorized)
    }

    fn authorize_output_receipt(
        &self,
        context: &AuthenticatedVoiceConnection,
        request: &VoiceOutputRequest,
    ) -> Result<(), DispatchError> {
        match &context.proof {
            PeerProof::Authenticated(peer)
                if peer.role() != crate::native_pair_auth::PeerRole::Inputia =>
            {
                return Err(DispatchError::Unauthorized);
            }
            _ => {}
        }
        let current_epoch = self
            .history
            .voice_terms_version()
            .map_err(|_| DispatchError::Unknown)?
            .policy_epoch;
        if !valid_id(&request.request_id)
            || !valid_id(&request.session_id)
            || request.server_instance != context.server
            || request.client_instance != context.client
            || request.policy_epoch > current_epoch
        {
            return Err(DispatchError::Unauthorized);
        }
        let VoiceOutputCommand::Receipt { operation_id, .. } = &request.output else {
            return Err(DispatchError::Unauthorized);
        };
        if !valid_id(operation_id) {
            return Err(DispatchError::Unauthorized);
        }
        Ok(())
    }

    fn validate_voice_output(
        &self,
        record: &SessionRecord,
        intent: &OutputIntent,
    ) -> Result<(), DispatchError> {
        let VoiceCommand::Start { target, .. } = &record.start.command else {
            return Err(DispatchError::Unknown);
        };
        if intent.owner != OutputOwner::Ime
            || intent.action != OutputAction::InsertText
            || intent.target_id.as_deref() != Some(target.target_id.as_str())
            || intent.policy_epoch != record.start.policy_epoch
        {
            return Err(DispatchError::Unauthorized);
        }
        Ok(())
    }

    fn with_output_projection(
        &self,
        mut view: VoiceSessionView,
    ) -> Result<VoiceSessionView, DispatchError> {
        if let Some(output) = self
            .history
            .voice_result(view.session_id.clone())
            .map_err(|_| DispatchError::Unknown)?
        {
            view.target_id = output.intent.target_id.clone();
            view.item_id = Some(output.intent.item_id);
            view.output_operation_id = Some(output.intent.operation_id);
            view.phase = match output.state {
                OutputState::Prepared | OutputState::PendingTarget => VoicePhase::PendingTarget,
                // 账本claim不能证明Host已调用IMK；只有明确Host回执才展示已派发。
                OutputState::Dispatched => VoicePhase::Uncertain,
                OutputState::DispatchedOnly => VoicePhase::Dispatched,
                OutputState::Confirmed => VoicePhase::Confirmed,
                OutputState::Uncertain => VoicePhase::Uncertain,
                OutputState::Rejected if view.phase == VoicePhase::Cancelled => {
                    VoicePhase::Cancelled
                }
                OutputState::Rejected => VoicePhase::Failed,
            };
        }
        Ok(view)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use inputia_handy_runtime::output_ledger::OutputState;
    use inputia_handy_runtime::voice_protocol::*;
    use rusqlite::Connection;
    use std::sync::{mpsc, Mutex};

    #[derive(Default)]
    struct FakeCoordinator {
        controls: Mutex<Vec<VoiceRequest>>,
        current: Mutex<Option<VoiceSessionView>>,
        lost: Mutex<Vec<mpsc::Sender<Result<VoiceSessionView, String>>>>,
        lose_next: Mutex<bool>,
        disconnect_next: Mutex<bool>,
        reject_next: Mutex<bool>,
    }
    impl VoiceCoordinatorPort for FakeCoordinator {
        fn control(&self, request: VoiceRequest) -> Receiver<Result<VoiceSessionView, String>> {
            self.controls.lock().unwrap().push(request.clone());
            if std::mem::take(&mut *self.reject_next.lock().unwrap()) {
                let (sender, receiver) = mpsc::channel();
                sender.send(Err("synthetic refusal".into())).unwrap();
                return receiver;
            }
            let mut current = self.current.lock().unwrap();
            let generation = current.as_ref().map_or(1, |view| view.generation + 1);
            let phase = match request.command {
                VoiceCommand::Start { .. } => VoicePhase::Preparing,
                VoiceCommand::Stop => VoicePhase::Processing,
                VoiceCommand::Cancel => VoicePhase::Cancelled,
                VoiceCommand::Status => panic!("Status must never use control"),
            };
            let view = VoiceSessionView {
                session_id: request.session_id,
                generation,
                phase,
                target_id: Some("target".into()),
                item_id: None,
                output_operation_id: None,
            };
            *current = Some(view.clone());
            let (sender, receiver) = mpsc::channel();
            if std::mem::take(&mut *self.disconnect_next.lock().unwrap()) {
                drop(sender);
                return receiver;
            }
            if std::mem::take(&mut *self.lose_next.lock().unwrap()) {
                self.lost.lock().unwrap().push(sender);
            } else {
                sender.send(Ok(view)).unwrap();
            }
            receiver
        }
        fn view(&self, _session: &str) -> Receiver<Result<VoiceSessionView, String>> {
            let (sender, receiver) = mpsc::channel();
            sender
                .send(
                    self.current
                        .lock()
                        .unwrap()
                        .clone()
                        .ok_or("not observed".into()),
                )
                .unwrap();
            receiver
        }
    }
    fn context(server: &str) -> AuthenticatedVoiceConnection {
        AuthenticatedVoiceConnection {
            client: "host".into(),
            server: server.into(),
            applied_version: Some(VoiceTermsVersion {
                policy_epoch: 1,
                learning_generation: 0,
            }),
            proof: PeerProof::Synthetic,
        }
    }
    fn request(id: &str, server: &str) -> VoiceRequest {
        VoiceRequest {
            request_id: id.into(),
            session_id: "session".into(),
            client_instance: "host".into(),
            server_instance: server.into(),
            policy_epoch: 1,
            command: VoiceCommand::Start {
                post_process: false,
                target: HostTargetToken {
                    target_id: "target".into(),
                    host_instance: "host".into(),
                    controller_id: "controller".into(),
                    activation_generation: 1,
                    field_id: Some("field".into()),
                    selection_generation: 1,
                    composition_generation: 0,
                    source_app: None,
                },
                terms: VoiceTermsVersion {
                    policy_epoch: 1,
                    learning_generation: 0,
                },
            },
        }
    }
    fn service(root: &std::path::Path) -> HistoryService {
        for (file, sql) in [
            ("history.db", "CREATE TABLE IF NOT EXISTS transcription_history(id INTEGER PRIMARY KEY,file_name TEXT,timestamp INTEGER,saved INTEGER,title TEXT,transcription_text TEXT,post_processed_text TEXT)"),
            ("clipboard.db", "CREATE TABLE IF NOT EXISTS clipboard_history(id INTEGER PRIMARY KEY,content_type TEXT,full_text TEXT,title TEXT,is_favorite INTEGER,is_pinned INTEGER,created_at INTEGER,image_path TEXT,source_app TEXT)")
        ] { Connection::open(root.join(file)).unwrap().execute_batch(sql).unwrap(); }
        let service = HistoryService::start(root.into(), "fixture".into(), |_| {}).unwrap();
        service.synchronize().unwrap();
        service
    }
    fn request_count(root: &std::path::Path) -> i64 {
        Connection::open(root.join("integration.db"))
            .unwrap()
            .query_row("SELECT COUNT(*) FROM unified_voice_requests", [], |r| {
                r.get(0)
            })
            .unwrap()
    }
    fn prepare_pending_voice_output(
        root: &std::path::Path,
        history: &HistoryService,
        start: &VoiceRequest,
        text: &str,
    ) -> inputia_handy_runtime::output_ledger::OutputRecord {
        Connection::open(root.join("history.db"))
            .unwrap()
            .execute(
                "INSERT INTO transcription_history VALUES(1,'',1,0,'fixture',?1,NULL)",
                [text],
            )
            .unwrap();
        let output = history
            .prepare_saved_voice_result(start.clone(), 1, text.into())
            .unwrap();
        history
            .project_voice_session(
                start.client_instance.clone(),
                start.server_instance.clone(),
                VoiceSessionView {
                    session_id: start.session_id.clone(),
                    generation: 2,
                    phase: VoicePhase::PendingTarget,
                    target_id: output.intent.target_id.clone(),
                    item_id: Some(output.intent.item_id.clone()),
                    output_operation_id: Some(output.intent.operation_id.clone()),
                },
            )
            .unwrap();
        output
    }

    fn fetch_request(id: &str, start: &VoiceRequest) -> VoiceOutputRequest {
        VoiceOutputRequest {
            request_id: id.into(),
            session_id: start.session_id.clone(),
            server_instance: start.server_instance.clone(),
            client_instance: start.client_instance.clone(),
            policy_epoch: start.policy_epoch,
            output: VoiceOutputCommand::Fetch {},
        }
    }

    fn receipt_request(
        id: &str,
        start: &VoiceRequest,
        operation: &str,
        receipt: HostOutputReceipt,
    ) -> VoiceOutputRequest {
        VoiceOutputRequest {
            request_id: id.into(),
            session_id: start.session_id.clone(),
            server_instance: start.server_instance.clone(),
            client_instance: start.client_instance.clone(),
            policy_epoch: start.policy_epoch,
            output: VoiceOutputCommand::Receipt {
                operation_id: operation.into(),
                receipt,
            },
        }
    }

    #[test]
    fn fetch_claims_once_then_receipt_records_dispatched_without_replaying_body() {
        let root = tempfile::tempdir().unwrap();
        let history = service(root.path());
        let coordinator = FakeCoordinator::default();
        let dispatcher = VoiceDispatcher::new(&history, &coordinator);
        let ctx = context("server");
        let start = request("start", "server");
        dispatcher.dispatch(&ctx, start.clone()).unwrap();
        let output = prepare_pending_voice_output(root.path(), &history, &start, "synthetic");
        let (reply, sent) = dispatcher
            .fetch_output(&ctx, fetch_request("fetch", &start))
            .unwrap();
        let intent = sent.unwrap().0;
        assert_eq!(intent, output.intent);
        match reply {
            VoiceOutputReply::Delivery { delivery, .. } => {
                assert_eq!(delivery.text, "synthetic");
                assert_eq!(delivery.operation_id, output.intent.operation_id);
            }
            _ => panic!("expected delivery"),
        }
        assert_eq!(
            history
                .voice_result("session".into())
                .unwrap()
                .unwrap()
                .state,
            OutputState::Dispatched
        );
        let (again, sent_again) = dispatcher
            .fetch_output(&ctx, fetch_request("fetch-again", &start))
            .unwrap();
        assert!(sent_again.is_none());
        assert_eq!(
            dispatcher.status(&ctx, "session").unwrap().phase,
            VoicePhase::Uncertain
        );
        assert!(matches!(
            again,
            VoiceOutputReply::Output {
                state: OutputState::Dispatched,
                ..
            }
        ));
        let receipt = dispatcher
            .receipt(
                &ctx,
                receipt_request(
                    "receipt",
                    &start,
                    &output.intent.operation_id,
                    HostOutputReceipt::Dispatched,
                ),
                &intent,
            )
            .unwrap();
        assert!(matches!(
            receipt,
            VoiceOutputReply::Output {
                state: OutputState::DispatchedOnly,
                ..
            }
        ));
        assert_eq!(
            dispatcher.status(&ctx, "session").unwrap().phase,
            VoicePhase::Dispatched
        );
    }

    #[test]
    fn fetch_refuses_missing_field_cancelled_deleted_and_oversized_delivery_without_claim() {
        let cases = ["missing-field", "cancelled", "deleted", "oversized"];
        for case in cases {
            let root = tempfile::tempdir().unwrap();
            let history = service(root.path());
            let coordinator = FakeCoordinator::default();
            let dispatcher = VoiceDispatcher::new(&history, &coordinator);
            let ctx = context("server");
            let mut start = request("start", "server");
            if case == "missing-field" {
                if let VoiceCommand::Start { target, .. } = &mut start.command {
                    target.field_id = None;
                }
            }
            dispatcher.dispatch(&ctx, start.clone()).unwrap();
            let text = if case == "oversized" {
                "\"".repeat(inputia_handy_runtime::voice_protocol::MAX_DELIVERY_TEXT_BYTES)
            } else {
                "synthetic".into()
            };
            let output = prepare_pending_voice_output(root.path(), &history, &start, &text);
            if case == "cancelled" {
                assert!(history
                    .cancel_voice_result("session".into(), "host".into(), "server".into())
                    .unwrap());
            }
            if case == "deleted" {
                Connection::open(root.path().join("history.db"))
                    .unwrap()
                    .execute("DELETE FROM transcription_history WHERE id=1", [])
                    .unwrap();
                let _ = history.synchronize();
            }
            assert!(dispatcher
                .fetch_output(&ctx, fetch_request("fetch", &start))
                .is_err());
            let state = history
                .voice_result("session".into())
                .unwrap()
                .unwrap()
                .state;
            match case {
                "cancelled" => assert_eq!(state, OutputState::Rejected),
                _ => assert_eq!(state, output.state),
            }
        }
    }

    #[test]
    fn receipt_requires_the_same_connection_dispatched_intent() {
        let root = tempfile::tempdir().unwrap();
        let history = service(root.path());
        let coordinator = FakeCoordinator::default();
        let dispatcher = VoiceDispatcher::new(&history, &coordinator);
        let ctx = context("server");
        let start = request("start", "server");
        dispatcher.dispatch(&ctx, start.clone()).unwrap();
        let output = prepare_pending_voice_output(root.path(), &history, &start, "synthetic");
        assert!(matches!(
            dispatcher.receipt(
                &ctx,
                receipt_request(
                    "receipt",
                    &start,
                    "voice-not-the-same-operation",
                    HostOutputReceipt::Dispatched,
                ),
                &output.intent,
            ),
            Err(DispatchError::Unauthorized)
        ));
        assert_eq!(
            history
                .voice_result("session".into())
                .unwrap()
                .unwrap()
                .state,
            OutputState::Prepared
        );
    }

    #[test]
    fn uncertain_and_pending_target_receipts_close_the_claim_without_confirming() {
        for (receipt, state, phase) in [
            (
                HostOutputReceipt::Uncertain,
                OutputState::Uncertain,
                VoicePhase::Uncertain,
            ),
            (
                HostOutputReceipt::PendingTarget,
                OutputState::PendingTarget,
                VoicePhase::PendingTarget,
            ),
        ] {
            let root = tempfile::tempdir().unwrap();
            let history = service(root.path());
            let coordinator = FakeCoordinator::default();
            let dispatcher = VoiceDispatcher::new(&history, &coordinator);
            let ctx = context("server");
            let start = request("start", "server");
            dispatcher.dispatch(&ctx, start.clone()).unwrap();
            let output = prepare_pending_voice_output(root.path(), &history, &start, "synthetic");
            let (_, sent) = dispatcher
                .fetch_output(&ctx, fetch_request("fetch", &start))
                .unwrap();
            let intent = sent.unwrap().0;
            dispatcher
                .receipt(
                    &ctx,
                    receipt_request("receipt", &start, &output.intent.operation_id, receipt),
                    &intent,
                )
                .unwrap();
            assert_eq!(
                history
                    .voice_result("session".into())
                    .unwrap()
                    .unwrap()
                    .state,
                state
            );
            assert_eq!(dispatcher.status(&ctx, "session").unwrap().phase, phase);
        }
    }

    #[test]
    fn receipt_can_record_an_old_epoch_dispatch_after_policy_moves_forward() {
        let root = tempfile::tempdir().unwrap();
        let history = service(root.path());
        let coordinator = FakeCoordinator::default();
        let dispatcher = VoiceDispatcher::new(&history, &coordinator);
        let ctx = context("server");
        let start = request("start", "server");
        dispatcher.dispatch(&ctx, start.clone()).unwrap();
        let output = prepare_pending_voice_output(root.path(), &history, &start, "synthetic");
        let (_, sent) = dispatcher
            .fetch_output(&ctx, fetch_request("fetch", &start))
            .unwrap();
        let intent = sent.unwrap().0;
        Connection::open(root.path().join("integration.db"))
            .unwrap()
            .execute(
                "UPDATE integration_meta SET value='2' WHERE key='policy_epoch'",
                [],
            )
            .unwrap();
        let reply = dispatcher
            .receipt(
                &ctx,
                receipt_request(
                    "receipt",
                    &start,
                    &output.intent.operation_id,
                    HostOutputReceipt::Uncertain,
                ),
                &intent,
            )
            .unwrap();
        assert!(matches!(
            reply,
            VoiceOutputReply::Output {
                state: OutputState::Uncertain,
                ..
            }
        ));
    }

    #[test]
    fn cancel_checks_persistent_output_before_coordinator_can_report_cancelled() {
        for claimed_output in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let history = service(root.path());
            let coordinator = FakeCoordinator::default();
            let dispatcher = VoiceDispatcher::new(&history, &coordinator);
            let ctx = context("server");
            let start = request("start", "server");
            dispatcher.dispatch(&ctx, start.clone()).unwrap();
            Connection::open(root.path().join("history.db"))
                .unwrap()
                .execute(
                    "INSERT INTO transcription_history VALUES(1,'',1,0,'fixture','result',NULL)",
                    [],
                )
                .unwrap();
            let output = history
                .prepare_saved_voice_result(start.clone(), 1, "result".into())
                .unwrap();
            if claimed_output {
                assert!(history.claim_output(output.intent.clone()).unwrap());
            }
            let mut cancel = start;
            cancel.request_id = "cancel".into();
            cancel.command = VoiceCommand::Cancel;
            let response = dispatcher.dispatch(&ctx, cancel);
            if claimed_output {
                assert_eq!(response, Err(DispatchError::Unknown));
                assert_eq!(coordinator.controls.lock().unwrap().len(), 1);
            } else {
                assert_eq!(response.unwrap().phase, VoicePhase::Cancelled);
                assert_eq!(coordinator.controls.lock().unwrap().len(), 2);
                assert_eq!(
                    history
                        .voice_result("session".into())
                        .unwrap()
                        .unwrap()
                        .state,
                    inputia_handy_runtime::output_ledger::OutputState::Rejected
                );
            }
        }
    }

    #[test]
    fn cancellation_barrier_without_actor_receipt_is_not_reported_as_current_recording() {
        let root = tempfile::tempdir().unwrap();
        let history = service(root.path());
        let coordinator = FakeCoordinator::default();
        let dispatcher = VoiceDispatcher::new(&history, &coordinator);
        let ctx = context("server");
        dispatcher
            .dispatch(&ctx, request("start", "server"))
            .unwrap();
        assert!(history
            .cancel_voice_result("session".into(), "host".into(), "server".into())
            .unwrap());
        assert_eq!(
            dispatcher.status(&ctx, "session"),
            Err(DispatchError::Unknown)
        );
        assert_eq!(coordinator.controls.lock().unwrap().len(), 1);
    }

    #[test]
    fn failed_cancel_barrier_does_not_consume_request_and_same_id_can_retry() {
        let root = tempfile::tempdir().unwrap();
        let history = service(root.path());
        let coordinator = FakeCoordinator::default();
        let dispatcher = VoiceDispatcher::new(&history, &coordinator);
        let ctx = context("server");
        dispatcher
            .dispatch(&ctx, request("start", "server"))
            .unwrap();
        let conn = Connection::open(root.path().join("integration.db")).unwrap();
        conn.execute_batch("CREATE TRIGGER reject_cancel BEFORE INSERT ON unified_voice_cancellations BEGIN SELECT RAISE(ABORT,'fixture');END;").unwrap();
        let mut cancel = request("cancel", "server");
        cancel.command = VoiceCommand::Cancel;
        assert_eq!(
            dispatcher.dispatch(&ctx, cancel.clone()),
            Err(DispatchError::Unknown)
        );
        assert_eq!(
            conn.query_row(
                "SELECT claimed FROM unified_voice_requests WHERE request_id='cancel'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(coordinator.controls.lock().unwrap().len(), 1);
        conn.execute_batch("DROP TRIGGER reject_cancel").unwrap();
        assert_eq!(
            dispatcher.dispatch(&ctx, cancel).unwrap().phase,
            VoicePhase::Cancelled
        );
        assert_eq!(coordinator.controls.lock().unwrap().len(), 2);
    }

    #[test]
    fn cancel_cannot_enqueue_ahead_of_a_claimed_start_on_another_connection() {
        struct PausedPort {
            inner: FakeCoordinator,
            reached: std::sync::mpsc::Sender<()>,
            resume: Mutex<std::sync::mpsc::Receiver<()>>,
        }
        impl VoiceCoordinatorPort for PausedPort {
            fn control(&self, request: VoiceRequest) -> Receiver<Result<VoiceSessionView, String>> {
                if matches!(request.command, VoiceCommand::Start { .. }) {
                    self.reached.send(()).unwrap();
                    self.resume
                        .lock()
                        .unwrap()
                        .recv_timeout(Duration::from_secs(5))
                        .unwrap();
                }
                self.inner.control(request)
            }
            fn view(&self, session: &str) -> Receiver<Result<VoiceSessionView, String>> {
                self.inner.view(session)
            }
        }
        let root = tempfile::tempdir().unwrap();
        let history = service(root.path());
        let (reached, ready) = mpsc::channel();
        let (resume, blocked) = mpsc::channel();
        let port = PausedPort {
            inner: FakeCoordinator::default(),
            reached,
            resume: Mutex::new(blocked),
        };
        std::thread::scope(|scope| {
            let first = scope.spawn(|| {
                VoiceDispatcher::new(&history, &port)
                    .dispatch(&context("server"), request("start", "server"))
            });
            ready.recv_timeout(Duration::from_secs(5)).unwrap();
            let (attempt, attempted) = mpsc::channel();
            let history_ref = &history;
            let port_ref = &port;
            let second = scope.spawn(move || {
                let mut cancel = request("cancel", "server");
                cancel.command = VoiceCommand::Cancel;
                attempt.send(()).unwrap();
                VoiceDispatcher::new(history_ref, port_ref).dispatch(&context("server"), cancel)
            });
            attempted.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(port.inner.controls.lock().unwrap().is_empty());
            resume.send(()).unwrap();
            first.join().unwrap().unwrap();
            assert_eq!(second.join().unwrap().unwrap().phase, VoicePhase::Cancelled);
        });
        let controls = port.inner.controls.lock().unwrap();
        assert!(matches!(controls[0].command, VoiceCommand::Start { .. }));
        assert!(matches!(controls[1].command, VoiceCommand::Cancel));
    }

    #[test]
    fn same_epoch_new_generation_requires_a_new_connection_barrier_ack() {
        let root = tempfile::tempdir().unwrap();
        let history = service(root.path());
        let coordinator = FakeCoordinator::default();
        let dispatcher = VoiceDispatcher::new(&history, &coordinator);
        let mut ctx = context("server");
        Connection::open(root.path().join("integration.db"))
            .unwrap()
            .execute(
                "UPDATE integration_meta SET value='1' WHERE key='learning_generation'",
                [],
            )
            .unwrap();
        let mut fresh = request("start-new-version", "server");
        if let VoiceCommand::Start { terms, .. } = &mut fresh.command {
            terms.learning_generation = 1;
        }
        assert_eq!(
            dispatcher.dispatch(&ctx, fresh.clone()),
            Err(DispatchError::Unauthorized)
        );
        assert!(coordinator.controls.lock().unwrap().is_empty());
        assert_eq!(request_count(root.path()), 0);
        let current = history.voice_terms_version().unwrap();
        ctx.acknowledge_policy(current.clone(), &current).unwrap();
        dispatcher.dispatch(&ctx, fresh).unwrap();
        assert_eq!(coordinator.controls.lock().unwrap().len(), 1);
        ctx.invalidate_policy();
        let mut stop = request("stop-without-new-barrier", "server");
        stop.command = VoiceCommand::Stop;
        assert!(dispatcher.dispatch(&ctx, stop).is_ok());
    }

    #[test]
    fn hundred_request_ids_start_once_status_is_read_only_and_owner_checked() {
        let root = tempfile::tempdir().unwrap();
        let history = service(root.path());
        let coordinator = FakeCoordinator::default();
        let dispatcher = VoiceDispatcher::new(&history, &coordinator);
        let ctx = context("server");
        for index in 0..100 {
            dispatcher
                .dispatch(&ctx, request(&format!("start-{index}"), "server"))
                .unwrap();
        }
        assert_eq!(coordinator.controls.lock().unwrap().len(), 1);
        let count = request_count(root.path());
        for index in 0..100 {
            let mut status = request(&format!("status-{index}"), "server");
            status.command = VoiceCommand::Status;
            dispatcher.dispatch(&ctx, status).unwrap();
        }
        assert_eq!(request_count(root.path()), count);
        let mut intruder = context("server");
        intruder.client = "other".into();
        assert_eq!(
            dispatcher.status(&intruder, "session"),
            Err(DispatchError::Unauthorized)
        );
    }

    #[test]
    fn uncertain_receipt_only_queries_and_old_projection_cannot_move_backward() {
        let root = tempfile::tempdir().unwrap();
        let history = service(root.path());
        let coordinator = FakeCoordinator::default();
        *coordinator.lose_next.lock().unwrap() = true;
        let mut dispatcher = VoiceDispatcher::new(&history, &coordinator);
        dispatcher.reply_timeout = Duration::from_millis(1);
        let ctx = context("server");
        assert_eq!(
            dispatcher.dispatch(&ctx, request("first", "server")),
            Err(DispatchError::Unknown)
        );
        assert_eq!(
            history
                .voice_session("session".into())
                .unwrap()
                .unwrap()
                .view
                .phase,
            VoicePhase::Preparing
        );
        assert_eq!(
            dispatcher
                .dispatch(&ctx, request("retry", "server"))
                .unwrap()
                .generation,
            1
        );
        assert_eq!(coordinator.controls.lock().unwrap().len(), 1);
        let mut newer = coordinator.current.lock().unwrap().clone().unwrap();
        newer.generation = 2;
        newer.phase = VoicePhase::Recording;
        history
            .project_voice_session("host".into(), "server".into(), newer.clone())
            .unwrap();
        assert_eq!(dispatcher.status(&ctx, "session").unwrap(), newer);
    }

    #[test]
    fn restart_retires_facts_and_never_replays_start_or_old_stop() {
        let root = tempfile::tempdir().unwrap();
        let coordinator = FakeCoordinator::default();
        {
            let history = service(root.path());
            let dispatcher = VoiceDispatcher::new(&history, &coordinator);
            dispatcher
                .dispatch(&context("old-server"), request("first", "old-server"))
                .unwrap();
        }
        let history = service(root.path());
        let dispatcher = VoiceDispatcher::new(&history, &coordinator);
        let ctx = context("new-server");
        assert_eq!(
            dispatcher.status(&ctx, "session").unwrap().phase,
            VoicePhase::Interrupted
        );
        assert!(dispatcher
            .dispatch(&ctx, request("new-start", "new-server"))
            .is_err());
        let mut stop = request("stop", "new-server");
        stop.command = VoiceCommand::Stop;
        assert_eq!(
            dispatcher.dispatch(&ctx, stop).unwrap().phase,
            VoicePhase::Interrupted
        );
        assert_eq!(coordinator.controls.lock().unwrap().len(), 1);
    }

    #[test]
    fn disconnected_receipt_and_explicit_rejection_never_reissue_claimed_start() {
        for reject in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let history = service(root.path());
            let coordinator = FakeCoordinator::default();
            *coordinator.reject_next.lock().unwrap() = reject;
            *coordinator.disconnect_next.lock().unwrap() = !reject;
            let dispatcher = VoiceDispatcher::new(&history, &coordinator);
            let ctx = context("server");
            assert_eq!(
                dispatcher.dispatch(&ctx, request("first", "server")),
                Err(if reject {
                    DispatchError::CoordinatorRejected
                } else {
                    DispatchError::Unknown
                })
            );
            let persisted = history.voice_session("session".into()).unwrap().unwrap();
            assert_eq!(
                persisted.view.phase,
                if reject {
                    VoicePhase::Failed
                } else {
                    VoicePhase::Preparing
                }
            );
            assert_eq!(persisted.view.generation, if reject { 1 } else { 0 });
            let replay = dispatcher
                .dispatch(&ctx, request("second", "server"))
                .unwrap();
            if reject {
                assert_eq!(replay.phase, VoicePhase::Failed);
                assert_eq!(dispatcher.status(&ctx, "session").unwrap(), replay);
            }
            assert_eq!(coordinator.controls.lock().unwrap().len(), 1);
        }
    }

    #[test]
    fn rejected_stop_and_cancel_do_not_fabricate_failed_terminal_state() {
        let root = tempfile::tempdir().unwrap();
        let history = service(root.path());
        let coordinator = FakeCoordinator::default();
        let dispatcher = VoiceDispatcher::new(&history, &coordinator);
        let ctx = context("server");
        let initial = dispatcher
            .dispatch(&ctx, request("start", "server"))
            .unwrap();
        for (id, command) in [
            ("stop", VoiceCommand::Stop),
            ("cancel", VoiceCommand::Cancel),
        ] {
            *coordinator.reject_next.lock().unwrap() = true;
            let mut control = request(id, "server");
            control.command = command;
            assert_eq!(
                dispatcher.dispatch(&ctx, control),
                Err(DispatchError::CoordinatorRejected)
            );
            assert_eq!(
                history
                    .voice_session("session".into())
                    .unwrap()
                    .unwrap()
                    .view,
                initial
            );
        }
    }

    #[test]
    fn request_peer_spoof_and_semantic_conflict_cannot_reach_coordinator() {
        let root = tempfile::tempdir().unwrap();
        let history = service(root.path());
        let coordinator = FakeCoordinator::default();
        let dispatcher = VoiceDispatcher::new(&history, &coordinator);
        let ctx = context("server");
        let mut spoof = request("spoof", "other-server");
        assert_eq!(
            dispatcher.dispatch(&ctx, spoof.clone()),
            Err(DispatchError::Unauthorized)
        );
        spoof.server_instance = "server".into();
        spoof.client_instance = "other-host".into();
        assert_eq!(
            dispatcher.dispatch(&ctx, spoof),
            Err(DispatchError::Unauthorized)
        );
        dispatcher
            .dispatch(&ctx, request("first", "server"))
            .unwrap();
        let mut different = request("second", "server");
        if let VoiceCommand::Start { target, .. } = &mut different.command {
            target.target_id = "different".into();
        }
        assert!(dispatcher.dispatch(&ctx, different).is_err());
        assert_eq!(coordinator.controls.lock().unwrap().len(), 1);
    }

    #[test]
    fn withdrawn_policy_blocks_start_but_keeps_owned_stop_cancel_available() {
        let root = tempfile::tempdir().unwrap();
        let history = service(root.path());
        let coordinator = FakeCoordinator::default();
        let dispatcher = VoiceDispatcher::new(&history, &coordinator);
        let mut ctx = context("server");
        dispatcher
            .dispatch(&ctx, request("start", "server"))
            .unwrap();
        Connection::open(root.path().join("integration.db"))
            .unwrap()
            .execute(
                "UPDATE integration_meta SET value='2' WHERE key='policy_epoch'",
                [],
            )
            .unwrap();
        ctx.applied_version = None;
        assert_eq!(
            dispatcher.dispatch(&ctx, request("again", "server")),
            Err(DispatchError::Unauthorized)
        );
        for (id, command) in [
            ("stop", VoiceCommand::Stop),
            ("cancel", VoiceCommand::Cancel),
        ] {
            let mut control = request(id, "server");
            control.command = command;
            dispatcher.dispatch(&ctx, control).unwrap();
        }
        assert_eq!(coordinator.controls.lock().unwrap().len(), 3);
        assert!(ctx
            .acknowledge_policy(
                VoiceTermsVersion {
                    policy_epoch: 1,
                    learning_generation: 0
                },
                &VoiceTermsVersion {
                    policy_epoch: 2,
                    learning_generation: 0
                }
            )
            .is_err());
        ctx.acknowledge_policy(
            VoiceTermsVersion {
                policy_epoch: 2,
                learning_generation: 0,
            },
            &VoiceTermsVersion {
                policy_epoch: 2,
                learning_generation: 0,
            },
        )
        .unwrap();
    }
}
