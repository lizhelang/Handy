//! Host 与唯一 TranscriptionCoordinator 之间的会话合同。
//! 这里只定义消息与授权输入，不拥有另一套录音状态机，也不执行输出。

use crate::output_ledger::OutputState;
use crate::protocol::ProtocolError;
use serde::{Deserialize, Serialize};

pub const IME_TARGET_CAPABILITY: &str = "ime_target_broker_v1";
pub const VOICE_CAPABILITY: &str = "voice_sessions_v1";
pub const SHARED_TERMS_CAPABILITY: &str = "shared_terms_v1";
pub const MAX_DELIVERY_TEXT_BYTES: usize = 192 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostTargetToken {
    pub target_id: String,
    pub host_instance: String,
    pub controller_id: String,
    pub activation_generation: u64,
    /// 不可获得可验证字段身份时为 None；不得以 bundle ID 代替。
    pub field_id: Option<String>,
    pub selection_generation: u64,
    pub composition_generation: u64,
    pub source_app: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceTermsVersion {
    pub policy_epoch: u64,
    pub learning_generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceShortcutActivation {
    Toggle,
    PushToTalk,
    HoldOrToggle,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostShortcutEdge {
    pub trigger_id: String,
    pub starts_session: bool,
    pub lease_id: String,
    pub lease_epoch: u64,
    pub binding_id: String,
    pub hotkey_string: String,
    pub is_pressed: bool,
    pub activation: VoiceShortcutActivation,
    pub pressed_at_unix_ms: u64,
    pub hold_threshold_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum VoiceCommand {
    Start {
        target: HostTargetToken,
        post_process: bool,
        terms: VoiceTermsVersion,
    },
    HostShortcut {
        target: HostTargetToken,
        post_process: bool,
        terms: VoiceTermsVersion,
        edge: HostShortcutEdge,
    },
    Stop,
    Cancel,
    Status,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceRequest {
    pub request_id: String,
    pub session_id: String,
    /// 重连后必须使用本次握手得到的进程实例，不能向新服务重放旧 Start。
    pub server_instance: String,
    pub client_instance: String,
    pub policy_epoch: u64,
    pub command: VoiceCommand,
}

/// 来自已完成签名/profile/策略握手的连接上下文，不能直接从消息正文创建。
pub struct VoicePeer<'a> {
    pub server_instance: &'a str,
    pub client_instance: &'a str,
    pub policy_epoch: u64,
    pub policy_applied: bool,
}

fn id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

impl VoiceRequest {
    pub fn validate_for(&self, peer: &VoicePeer<'_>) -> Result<(), ProtocolError> {
        if !id(&self.request_id)
            || !id(&self.session_id)
            || !id(&self.server_instance)
            || !id(&self.client_instance)
            || self.server_instance != peer.server_instance
            || self.client_instance != peer.client_instance
        {
            return Err(ProtocolError::InvalidEnvelope);
        }
        if self.policy_epoch > peer.policy_epoch {
            return Err(ProtocolError::PolicyRefreshRequired);
        }
        if let Some((target, terms)) = self.strict_start_identity() {
            if !peer.policy_applied
                || self.policy_epoch != peer.policy_epoch
                || terms.policy_epoch != peer.policy_epoch
            {
                return Err(ProtocolError::PolicyRefreshRequired);
            }
            if target.host_instance != peer.client_instance
                || !id(&target.target_id)
                || !id(&target.controller_id)
                || target.field_id.as_ref().is_some_and(|field| !id(field))
                || target.source_app.as_ref().is_some_and(|app| !id(app))
            {
                return Err(ProtocolError::InvalidEnvelope);
            }
        }
        if let VoiceCommand::HostShortcut { target, edge, .. } = &self.command {
            if target.host_instance != peer.client_instance
                || !id(&target.target_id)
                || !id(&target.controller_id)
                || target.field_id.as_ref().is_some_and(|field| !id(field))
                || target.source_app.as_ref().is_some_and(|app| !id(app))
                || !id(&edge.trigger_id)
                || !id(&edge.lease_id)
                || edge.lease_epoch == 0
                || !id(&edge.binding_id)
                || !id(&edge.hotkey_string)
            {
                return Err(ProtocolError::InvalidEnvelope);
            }
        }
        // Stop/Cancel 可在策略撤销后关闭既有采集；session归属仍须由Coordinator核验。
        Ok(())
    }

    pub fn start_identity(&self) -> Option<(&HostTargetToken, &VoiceTermsVersion)> {
        match &self.command {
            VoiceCommand::Start { target, terms, .. }
            | VoiceCommand::HostShortcut { target, terms, .. } => Some((target, terms)),
            VoiceCommand::Stop | VoiceCommand::Cancel | VoiceCommand::Status => None,
        }
    }

    pub fn strict_start_identity(&self) -> Option<(&HostTargetToken, &VoiceTermsVersion)> {
        match &self.command {
            VoiceCommand::Start { target, terms, .. } => Some((target, terms)),
            VoiceCommand::HostShortcut {
                target,
                terms,
                edge,
                ..
            } if edge.starts_session => Some((target, terms)),
            VoiceCommand::HostShortcut { .. }
            | VoiceCommand::Stop
            | VoiceCommand::Cancel
            | VoiceCommand::Status => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoicePhase {
    Preparing,
    Recording,
    Processing,
    PendingTarget,
    Dispatched,
    Confirmed,
    Uncertain,
    Cancelled,
    Failed,
    Interrupted,
}

/// 仅为 Coordinator/输出账本事实的投影；启动进程成功不是 Recording。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceSessionView {
    pub session_id: String,
    pub generation: u64,
    pub phase: VoicePhase,
    pub target_id: Option<String>,
    pub item_id: Option<String>,
    pub output_operation_id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceReplyError {
    Unauthorized,
    MissingSession,
    Unknown,
    CoordinatorRejected,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum VoiceReply {
    Session {
        request_id: String,
        view: VoiceSessionView,
    },
    Rejected {
        request_id: String,
        code: VoiceReplyError,
    },
}

impl VoiceReply {
    pub fn validate_for(&self, request: &VoiceRequest) -> Result<(), ProtocolError> {
        let (request_id, view) = match self {
            Self::Session { request_id, view } => (request_id, Some(view)),
            Self::Rejected { request_id, .. } => (request_id, None),
        };
        if !id(request_id)
            || request_id != &request.request_id
            || view.is_some_and(|view| view.session_id != request.session_id)
        {
            return Err(ProtocolError::InvalidEnvelope);
        }
        Ok(())
    }
}

/// 正文只通过认证私有连接传输；故意不实现 Debug，防止顺手记录全文。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceDelivery {
    pub operation_id: String,
    pub session_id: String,
    pub item_id: String,
    pub revision: u64,
    pub policy_epoch: u64,
    pub target_id: String,
    pub text: String,
}

impl VoiceDelivery {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if !id(&self.operation_id)
            || !id(&self.session_id)
            || !id(&self.target_id)
            || self.item_id.is_empty()
            || self.item_id.len() > 1024
            || self.item_id.chars().any(char::is_control)
            || self.text.len() > MAX_DELIVERY_TEXT_BYTES
        {
            return Err(ProtocolError::InvalidEnvelope);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostOutputReceipt {
    /// 已调用 IMK insertText 但不能证明目标控件落字。
    Dispatched,
    Confirmed,
    PendingTarget,
    Uncertain,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum VoiceOutputCommand {
    Fetch {},
    Receipt {
        operation_id: String,
        receipt: HostOutputReceipt,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceOutputRequest {
    pub request_id: String,
    pub session_id: String,
    pub server_instance: String,
    pub client_instance: String,
    pub policy_epoch: u64,
    pub output: VoiceOutputCommand,
}

impl VoiceOutputRequest {
    pub fn validate_for(&self, peer: &VoicePeer<'_>) -> Result<(), ProtocolError> {
        if !id(&self.request_id)
            || !id(&self.session_id)
            || !id(&self.server_instance)
            || !id(&self.client_instance)
            || self.server_instance != peer.server_instance
            || self.client_instance != peer.client_instance
            || self.policy_epoch != peer.policy_epoch
            || !peer.policy_applied
        {
            return Err(ProtocolError::InvalidEnvelope);
        }
        if let VoiceOutputCommand::Receipt { operation_id, .. } = &self.output {
            if !id(operation_id) {
                return Err(ProtocolError::InvalidEnvelope);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum VoiceWireRequest {
    Personalization(crate::personalization_wire::PersonalizationRequest),
    TypedCapture(TypedCaptureRequest),
    TargetBridge(TargetBridgeRequest),
    Control(VoiceRequest),
    Output(VoiceOutputRequest),
    Menu(MenuRequest),
    HostShortcut(HostShortcutRequest),
    SharedTerms(SharedTermsRequest),
}

pub const TYPED_CAPTURE_CAPABILITY: &str = "typed_capture_v1";

/// 仅在已认证的输入法连接上传递确认提交的正文；不接受键盘事件或待选文字。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypedCaptureRequest {
    pub request_id: String,
    pub client_instance: String,
    pub server_instance: String,
    pub policy_epoch: u64,
    pub typed_capture: TypedCaptureCommand,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TypedCaptureCommand {
    Policy,
    Commit {
        capture_epoch: u64,
        event_id: String,
        segment_id: String,
        text: String,
        draft: Box<HostTargetToken>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypedCaptureReply {
    pub status: String,
    pub request_id: String,
    pub server_instance: String,
    pub enabled: bool,
    pub epoch: u64,
    pub saved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

impl TypedCaptureRequest {
    /// 同时核验连接世代、隐私屏障及有界载荷；字段身份仍由主程序重新捕获。
    pub fn validate_for(&self, peer: &VoicePeer<'_>) -> Result<(), ProtocolError> {
        let target_bridge = match &self.typed_capture {
            TypedCaptureCommand::Policy => TargetBridgeCommand::Status,
            TypedCaptureCommand::Commit {
                capture_epoch,
                event_id,
                segment_id,
                text,
                draft,
            } => {
                if *capture_epoch == 0
                    || !id(event_id)
                    || !id(segment_id)
                    || text.trim().is_empty()
                    || text.len() > 8192
                    || text
                        .chars()
                        .any(|c| c.is_control() && c != '\n' && c != '\t')
                {
                    return Err(ProtocolError::InvalidEnvelope);
                }
                if draft.field_id.as_deref() != Some(draft.target_id.as_str()) {
                    return Err(ProtocolError::InvalidEnvelope);
                }
                TargetBridgeCommand::Validate {
                    target: (**draft).clone(),
                    purpose: TargetBridgePurpose::TypedCapture,
                    operation_id: None,
                }
            }
        };
        TargetBridgeRequest {
            request_id: self.request_id.clone(),
            client_instance: self.client_instance.clone(),
            server_instance: self.server_instance.clone(),
            policy_epoch: self.policy_epoch,
            target_bridge,
        }
        .validate_for(peer)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedTermsLease {
    pub lease_id: String,
    pub lease_epoch: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedTermsRequest {
    pub request_id: String,
    pub client_instance: String,
    pub server_instance: String,
    pub policy_epoch: u64,
    pub shared_terms: SharedTermsLease,
}

impl SharedTermsRequest {
    pub fn validate_for(&self, peer: &VoicePeer<'_>) -> Result<(), ProtocolError> {
        if !id(&self.request_id)
            || !id(&self.client_instance)
            || !id(&self.server_instance)
            || self.client_instance != peer.client_instance
            || self.server_instance != peer.server_instance
            || self.policy_epoch != peer.policy_epoch
            || !peer.policy_applied
            || !id(&self.shared_terms.lease_id)
            || self.shared_terms.lease_epoch == 0
        {
            return Err(ProtocolError::InvalidEnvelope);
        }
        Ok(())
    }
}

/// 用户显式词汇的有界副本；允许邮箱与混合词，不把自动学习词提升为显式来源。
pub fn explicit_hotword_terms(words: &[String]) -> Vec<String> {
    let mut terms = Vec::new();
    let mut bytes = 0;
    for word in words {
        let word = word.trim();
        if word.is_empty()
            || word.chars().count() > 128
            || word.contains("<|")
            || word.contains("|>")
            || word.chars().any(char::is_control)
            || terms.iter().any(|item| item == word)
        {
            continue;
        }
        if terms.len() >= 256 || bytes + word.len() > 16 * 1024 {
            break;
        }
        bytes += word.len();
        terms.push(word.to_owned());
    }
    terms
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum SharedTermsReply {
    SharedTerms {
        request_id: String,
        lease_id: String,
        lease_epoch: u64,
        version: VoiceTermsVersion,
        terms: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        explicit_terms: Vec<String>,
        max_age_ms: u64,
    },
    Rejected {
        request_id: String,
        code: VoiceReplyError,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostShortcutLease {
    pub lease_id: String,
    pub lease_epoch: u64,
    pub target: HostTargetToken,
    pub issued_at_unix_ms: u64,
    pub expires_at_unix_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostShortcutCommand {
    Register { lease: HostShortcutLease },
    Poll { max_wait_ms: u64 },
    Retire { lease_id: String, lease_epoch: u64 },
    Reject { trigger_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostShortcutRequest {
    pub request_id: String,
    pub client_instance: String,
    pub server_instance: String,
    pub policy_epoch: u64,
    pub shortcut: HostShortcutCommand,
}

impl HostShortcutRequest {
    pub fn validate_for(&self, peer: &VoicePeer<'_>) -> Result<(), ProtocolError> {
        if !id(&self.request_id)
            || !id(&self.client_instance)
            || !id(&self.server_instance)
            || self.client_instance != peer.client_instance
            || self.server_instance != peer.server_instance
            || self.policy_epoch != peer.policy_epoch
            || !peer.policy_applied
        {
            return Err(ProtocolError::InvalidEnvelope);
        }
        match &self.shortcut {
            HostShortcutCommand::Register { lease } => {
                if !id(&lease.lease_id)
                    || lease.lease_epoch == 0
                    || lease.issued_at_unix_ms >= lease.expires_at_unix_ms
                    || lease.target.host_instance != peer.client_instance
                    || !id(&lease.target.target_id)
                    || !id(&lease.target.controller_id)
                    || lease
                        .target
                        .field_id
                        .as_ref()
                        .is_some_and(|field| !id(field))
                    || lease.target.source_app.as_ref().is_some_and(|app| !id(app))
                {
                    return Err(ProtocolError::InvalidEnvelope);
                }
            }
            HostShortcutCommand::Poll { max_wait_ms } => {
                if *max_wait_ms > 30_000 {
                    return Err(ProtocolError::InvalidEnvelope);
                }
            }
            HostShortcutCommand::Retire {
                lease_id,
                lease_epoch,
            } => {
                if !id(lease_id) || *lease_epoch == 0 {
                    return Err(ProtocolError::InvalidEnvelope);
                }
            }
            HostShortcutCommand::Reject { trigger_id } => {
                if !id(trigger_id) {
                    return Err(ProtocolError::InvalidEnvelope);
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostShortcutTrigger {
    pub trigger_id: String,
    pub session_id: String,
    pub starts_session: bool,
    pub lease_id: String,
    pub lease_epoch: u64,
    pub target: HostTargetToken,
    pub binding_id: String,
    pub hotkey_string: String,
    pub is_pressed: bool,
    pub activation: VoiceShortcutActivation,
    pub pressed_at_unix_ms: u64,
    pub hold_threshold_ms: u64,
    pub server_instance: String,
    pub client_instance: String,
    pub policy_epoch: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostShortcutReply {
    Registered {
        request_id: String,
        lease_id: String,
        lease_epoch: u64,
    },
    Retired {
        request_id: String,
    },
    Trigger {
        request_id: String,
        trigger: Box<HostShortcutTrigger>,
    },
    Empty {
        request_id: String,
    },
    Rejected {
        request_id: String,
        code: VoiceReplyError,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MenuCommand {
    Status,
    CopyLatest,
    History,
    Settings,
    CheckUpdates,
    UnloadModel,
    SelectModel { model_id: String },
    QuitService,
}

impl MenuCommand {
    /// 录音时仍可查看历史/设置；仅引擎变更和退出与采集冲突。
    pub fn allowed_while_busy(&self) -> bool {
        !matches!(
            self,
            Self::UnloadModel | Self::SelectModel { .. } | Self::QuitService
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MenuRequest {
    pub request_id: String,
    pub client_instance: String,
    pub server_instance: String,
    pub policy_epoch: u64,
    pub menu: MenuCommand,
}

impl MenuRequest {
    pub fn validate_for(&self, peer: &VoicePeer<'_>) -> Result<(), ProtocolError> {
        if !id(&self.request_id)
            || !id(&self.client_instance)
            || !id(&self.server_instance)
            || self.client_instance != peer.client_instance
            || self.server_instance != peer.server_instance
            || self.policy_epoch != peer.policy_epoch
            || !peer.policy_applied
            || matches!(&self.menu, MenuCommand::SelectModel { model_id } if !id(model_id))
        {
            return Err(ProtocolError::InvalidEnvelope);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MenuModel {
    pub id: String,
    pub name: String,
    pub available: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum MenuReply {
    Menu {
        request_id: String,
        selected_model: String,
        models: Vec<MenuModel>,
        busy: bool,
    },
    Rejected {
        request_id: String,
        code: VoiceReplyError,
    },
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum VoiceOutputReply {
    Delivery {
        request_id: String,
        delivery: VoiceDelivery,
    },
    Output {
        request_id: String,
        operation_id: String,
        state: OutputState,
    },
    Rejected {
        request_id: String,
        code: VoiceReplyError,
    },
}

impl VoiceOutputReply {
    pub fn validate_for(&self, request: &VoiceOutputRequest) -> Result<(), ProtocolError> {
        match self {
            Self::Delivery {
                request_id,
                delivery,
            } => {
                if request_id != &request.request_id || delivery.session_id != request.session_id {
                    return Err(ProtocolError::InvalidEnvelope);
                }
                delivery.validate()
            }
            Self::Output {
                request_id,
                operation_id,
                ..
            } => {
                if request_id != &request.request_id || !id(operation_id) {
                    return Err(ProtocolError::InvalidEnvelope);
                }
                Ok(())
            }
            Self::Rejected { request_id, .. } => {
                if request_id != &request.request_id {
                    return Err(ProtocolError::InvalidEnvelope);
                }
                Ok(())
            }
        }
    }
}

/// 认证连接上的全量失效屏障；不携带词库正文，不让旧离线快照绕过遗忘传播。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoicePolicyBarrier {
    pub barrier_id: String,
    pub version: VoiceTermsVersion,
    pub clear_shared_personalization: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoicePolicyAcknowledgement {
    pub barrier_id: String,
    pub version: VoiceTermsVersion,
    pub shared_cache_cleared: bool,
    pub offline_queue_revalidated: bool,
}

impl VoicePolicyBarrier {
    pub fn new(version: VoiceTermsVersion) -> Result<Self, ProtocolError> {
        let mut nonce = [0u8; 32];
        getrandom::getrandom(&mut nonce).map_err(|_| ProtocolError::PeerIdentity)?;
        Ok(Self {
            barrier_id: nonce.iter().map(|byte| format!("{byte:02x}")).collect(),
            version,
            clear_shared_personalization: true,
        })
    }

    /// current必须重新从服务读取，不能使用发出请求时缓存的版本。
    pub fn validate_ack(
        &self,
        ack: &VoicePolicyAcknowledgement,
        current: &VoiceTermsVersion,
    ) -> Result<(), ProtocolError> {
        if !self.clear_shared_personalization
            || !id(&self.barrier_id)
            || ack.barrier_id != self.barrier_id
            || ack.version != self.version
            || current != &self.version
            || !ack.shared_cache_cleared
            || !ack.offline_queue_revalidated
        {
            return Err(ProtocolError::PolicyRefreshRequired);
        }
        Ok(())
    }
}

/// Signed voice socket only. A draft is never itself an output authorization.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetBridgeRequest {
    pub request_id: String,
    pub client_instance: String,
    pub server_instance: String,
    pub policy_epoch: u64,
    pub target_bridge: TargetBridgeCommand,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TargetBridgeCommand {
    Status,
    Capture {
        draft: HostTargetToken,
    },
    Validate {
        target: HostTargetToken,
        purpose: TargetBridgePurpose,
        #[serde(default)]
        operation_id: Option<String>,
    },
    Release {
        target_id: String,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetBridgePurpose {
    Personalization,
    TypedCapture,
    Start,
    Dispatch,
    SharedTerms,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetBridgeSelection {
    pub location: i64,
    pub length: i64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetBridgeReply {
    pub status: String,
    pub request_id: String,
    pub ready: bool,
    pub server_instance: String,
    pub permission_epoch: u64,
    pub valid_for_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<HostTargetToken>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection: Option<TargetBridgeSelection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dispatch_nonce: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}
impl TargetBridgeRequest {
    pub fn validate_for(&self, peer: &VoicePeer<'_>) -> Result<(), ProtocolError> {
        let valid_target = |target: &HostTargetToken| {
            target.host_instance == peer.client_instance
                && id(&target.target_id)
                && id(&target.controller_id)
                && target.source_app.as_ref().is_some_and(|source| id(source))
                && target.field_id.as_ref().is_none_or(|field| id(field))
        };
        if !id(&self.request_id)
            || !id(&self.client_instance)
            || !id(&self.server_instance)
            || self.client_instance != peer.client_instance
            || self.server_instance != peer.server_instance
            || self.policy_epoch != peer.policy_epoch
            || !peer.policy_applied
        {
            return Err(ProtocolError::InvalidEnvelope);
        }
        let valid = match &self.target_bridge {
            TargetBridgeCommand::Status => true,
            TargetBridgeCommand::Capture { draft } => valid_target(draft),
            TargetBridgeCommand::Release { target_id } => id(target_id),
            TargetBridgeCommand::Validate {
                target,
                purpose,
                operation_id,
            } => {
                valid_target(target)
                    && operation_id.as_ref().is_none_or(|operation| id(operation))
                    && (*purpose != TargetBridgePurpose::Dispatch || operation_id.is_some())
            }
        };
        if valid {
            Ok(())
        } else {
            Err(ProtocolError::InvalidEnvelope)
        }
    }
}
