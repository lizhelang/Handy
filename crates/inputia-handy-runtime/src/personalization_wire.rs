//! 已认证输入法连接上的学习与联想合同；知识库CLI不暴露这些写操作。
use crate::{
    protocol::ProtocolError,
    voice_protocol::{
        HostTargetToken, TargetBridgeCommand, TargetBridgePurpose, TargetBridgeRequest, VoicePeer,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const CAPABILITY: &str = "personalization_v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PersonalizationRequest {
    pub request_id: String,
    pub client_instance: String,
    pub server_instance: String,
    pub policy_epoch: u64,
    pub personalization: PersonalizationCommand,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PersonalizationCommand {
    Policy,
    Admit {
        target: Box<HostTargetToken>,
        learning_epoch: u64,
        context_id: String,
        context: String,
        #[serde(default)]
        input_code: String,
        #[serde(default)]
        schema_id: String,
        text: String,
        prediction_id: String,
    },
    Query {
        target: Box<HostTargetToken>,
        learning_epoch: u64,
        input_code: String,
        #[serde(default)]
        schema_id: String,
        context: String,
        context_id: String,
        candidates: Vec<Value>,
        limit: usize,
    },
    Feedback {
        target: Box<HostTargetToken>,
        learning_epoch: u64,
        event_id: String,
        context_id: String,
        input_code: String,
        #[serde(default)]
        schema_id: String,
        text: String,
        previous: String,
        explicit_selection: bool,
        original_rank: usize,
        operation: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PersonalizationReply {
    pub status: String,
    pub request_id: String,
    pub server_instance: String,
    pub enabled: bool,
    pub epoch: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub privacy_barrier: Option<crate::voice_protocol::VoicePolicyBarrier>,
}

impl PersonalizationRequest {
    /// 限定当前对端、策略与原字段；服务端另行核验活的AX字段与学习世代。
    pub fn validate_for(&self, peer: &VoicePeer<'_>) -> Result<(), ProtocolError> {
        let clean = |s: &str, n: usize| s.len() <= n && !s.chars().any(char::is_control);
        let bounded_context = |s: &str| s.chars().count() <= 64 && clean(s, 1024);
        let bridge = match &self.personalization {
            PersonalizationCommand::Policy => TargetBridgeCommand::Status,
            PersonalizationCommand::Admit {
                target,
                learning_epoch,
                context_id,
                context,
                input_code,
                schema_id,
                text,
                prediction_id,
            } => {
                if *learning_epoch == 0
                    || context_id != &target.target_id
                    || target.field_id.as_deref() != Some(target.target_id.as_str())
                    || !bounded_context(context)
                    || !clean(input_code, 256)
                    || !clean(schema_id, 128)
                    || (!input_code.is_empty() && (schema_id.is_empty() || !input_code.is_ascii()))
                    || !clean(text, 512)
                    || text.trim().is_empty()
                    || !clean(prediction_id, 256)
                    || prediction_id.is_empty()
                {
                    return Err(ProtocolError::InvalidEnvelope);
                }
                TargetBridgeCommand::Validate {
                    target: (**target).clone(),
                    purpose: TargetBridgePurpose::Personalization,
                    operation_id: None,
                }
            }
            PersonalizationCommand::Query {
                target,
                learning_epoch,
                input_code,
                schema_id,
                context,
                context_id,
                candidates,
                limit,
            } => {
                if *learning_epoch == 0
                    || !clean(input_code, 256)
                    || !clean(schema_id, 128)
                    || !bounded_context(context)
                    || context_id != &target.target_id
                    || target.field_id.as_deref() != Some(target.target_id.as_str())
                    || candidates.len() > 64
                    || !(1..=5).contains(limit)
                    || serde_json::to_vec(candidates).map_or(true, |v| v.len() > 24_000)
                {
                    return Err(ProtocolError::InvalidEnvelope);
                }
                TargetBridgeCommand::Validate {
                    target: (**target).clone(),
                    purpose: TargetBridgePurpose::Personalization,
                    operation_id: None,
                }
            }
            PersonalizationCommand::Feedback {
                target,
                learning_epoch,
                event_id,
                context_id,
                input_code,
                schema_id,
                text,
                previous,
                original_rank,
                operation,
                ..
            } => {
                if *learning_epoch == 0
                    || !clean(event_id, 128)
                    || event_id.is_empty()
                    || !clean(input_code, 256)
                    || !clean(schema_id, 128)
                    || !clean(text, 512)
                    || text.trim().is_empty()
                    || !bounded_context(previous)
                    || context_id != &target.target_id
                    || target.field_id.as_deref() != Some(target.target_id.as_str())
                    || *original_rank > 1000
                    || !matches!(operation.as_str(), "accept" | "undo" | "reject")
                {
                    return Err(ProtocolError::InvalidEnvelope);
                }
                TargetBridgeCommand::Validate {
                    target: (**target).clone(),
                    purpose: TargetBridgePurpose::Personalization,
                    operation_id: None,
                }
            }
        };
        TargetBridgeRequest {
            request_id: self.request_id.clone(),
            client_instance: self.client_instance.clone(),
            server_instance: self.server_instance.clone(),
            policy_epoch: self.policy_epoch,
            target_bridge: bridge,
        }
        .validate_for(peer)
    }
}
