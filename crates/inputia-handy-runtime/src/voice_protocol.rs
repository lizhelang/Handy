//! Host 与唯一 TranscriptionCoordinator 之间的会话合同。
//! 这里只定义消息与授权输入，不拥有另一套录音状态机，也不执行输出。

use crate::protocol::ProtocolError;
use serde::{Deserialize, Serialize};

pub const VOICE_CAPABILITY: &str = "voice_sessions_v1";
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum VoiceCommand {
    Start {
        target: HostTargetToken,
        post_process: bool,
        terms: VoiceTermsVersion,
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
        if let VoiceCommand::Start { target, terms, .. } = &self.command {
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
        // Stop/Cancel 可在策略撤销后关闭既有采集；session归属仍须由Coordinator核验。
        Ok(())
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
