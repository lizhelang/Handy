//! 旧学习域的服务消息。类型校验不代表真实目标、来源证据或管理操作已经获准。
use crate::{
    protocol::ProtocolError,
    voice_protocol::{
        HostTargetToken, TargetBridgeCommand, TargetBridgePurpose, TargetBridgeRequest, VoicePeer,
    },
};
use inputia_core::{
    memory_snapshot::{MemoryQuery, MemorySnapshot},
    MemoryTerm,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const CAPABILITY: &str = "memory_domain_v1";
pub const MAX_QUERY_TEXT_BYTES: usize = 64 * 1_024;
pub const MAX_SNAPSHOT_WIRE_BYTES: usize = 192 * 1_024;
pub const MAX_SNAPSHOT_AGE_MS: u64 = 2_000;
pub const MAX_IMPORT_LIMIT: usize = 2_000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MemoryRequest {
    pub request_id: String,
    pub client_instance: String,
    pub server_instance: String,
    pub policy_epoch: u64,
    pub memory_domain: MemoryCommand,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MemoryCommand {
    Policy {},
    Query {
        query_id: String,
        query_generation: u64,
        target: Box<HostTargetToken>,
        composing: String,
        query: MemoryQuery,
    },
    PrepareWordSpan {
        target: Box<HostTargetToken>,
    },
    RecordWordSpan {
        target: Box<HostTargetToken>,
        span_id: String,
        sequence: u64,
        edit: crate::memory_word_span::SpanEdit,
    },
    CheckpointWordSpan {
        target: Box<HostTargetToken>,
        span_id: String,
        operation_id: String,
        through_sequence: u64,
        finish: bool,
    },
    RetireWordSpan {
        span_id: String,
    },
    PrepareCommit {
        target: Box<HostTargetToken>,
        request: crate::memory_commit::FixedPlansRequest,
    },
    ConfirmCommit {
        target: Box<HostTargetToken>,
        operation_id: String,
        commit_id: String,
        plan_id: String,
    },
    /// 仅表达已提交词的意图；真实提交与全词/suffix对应关系必须由主程序核实。
    LearnTyped {
        operation_id: String,
        event_id: String,
        target: Box<HostTargetToken>,
        text: String,
    },
    /// 不允许客户端提供源路径、源正文或自报的已核实来源。
    Import {
        operation_id: String,
        selection: ImportSelection,
        limit: usize,
    },
    Outcome {
        operation_id: String,
    },
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ImportSelection {
    History,
    Clipboard,
    Both,
}

fn identifier(value: &str) -> bool {
    inputia_core::integration::events::Identifier::parse(value.to_owned()).is_ok()
}
fn bounded_text(value: &str, limit: usize) -> bool {
    value.len() <= limit && !value.chars().any(char::is_control)
}
impl MemoryRequest {
    /// 核验当前认证连接和策略；Outcome可查询旧版本结果，但不能提升为当前有效学习。
    pub fn validate_for(&self, peer: &VoicePeer<'_>) -> Result<(), ProtocolError> {
        if !identifier(&self.request_id)
            || self.client_instance != peer.client_instance
            || self.server_instance != peer.server_instance
            || !identifier(&self.client_instance)
            || !identifier(&self.server_instance)
            || self.policy_epoch == 0
            || self.policy_epoch > peer.policy_epoch
        {
            return Err(ProtocolError::InvalidEnvelope);
        }
        if let MemoryCommand::RetireWordSpan { span_id } = &self.memory_domain {
            return if identifier(span_id) {
                Ok(())
            } else {
                Err(ProtocolError::InvalidEnvelope)
            };
        }
        if let MemoryCommand::Outcome { operation_id } = &self.memory_domain {
            return if identifier(operation_id) {
                Ok(())
            } else {
                Err(ProtocolError::InvalidEnvelope)
            };
        }
        if self.policy_epoch != peer.policy_epoch || !peer.policy_applied {
            return Err(ProtocolError::PolicyRefreshRequired);
        }
        let target = match &self.memory_domain {
            MemoryCommand::Policy {} => None,
            MemoryCommand::Query {
                query_id,
                query_generation,
                target,
                composing,
                query,
            } => {
                if !identifier(query_id)
                    || *query_generation == 0
                    || !bounded_text(composing, 512)
                    || query.validate().is_err()
                {
                    return Err(ProtocolError::InvalidEnvelope);
                }
                if let MemoryQuery::Rank { candidate_texts } = query {
                    if candidate_texts.len() > 64
                        || candidate_texts.iter().map(String::len).sum::<usize>()
                            > MAX_QUERY_TEXT_BYTES
                    {
                        return Err(ProtocolError::InvalidEnvelope);
                    }
                }
                Some(target)
            }
            MemoryCommand::PrepareWordSpan { target } => Some(target),
            MemoryCommand::RecordWordSpan {
                target,
                span_id,
                sequence,
                edit,
            } => {
                let valid_edit = match edit {
                    crate::memory_word_span::SpanEdit::Append { text } => {
                        !text.is_empty()
                            && text.len() <= crate::memory_word_span::MAX_SPAN_BYTES
                            && !text
                                .chars()
                                .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
                    }
                    crate::memory_word_span::SpanEdit::TailBackspace { units } => {
                        (1..=crate::memory_word_span::MAX_SPAN_UNITS as u64).contains(units)
                    }
                };
                if !identifier(span_id) || *sequence == 0 || !valid_edit {
                    return Err(ProtocolError::InvalidEnvelope);
                }
                Some(target)
            }
            MemoryCommand::CheckpointWordSpan {
                target,
                span_id,
                operation_id,
                through_sequence,
                finish,
            } => {
                let suffix = if *finish { ":seal" } else { "" };
                if !identifier(span_id)
                    || !identifier(operation_id)
                    || *through_sequence == 0
                    || operation_id != &format!("word-span:{span_id}:{through_sequence}{suffix}")
                {
                    return Err(ProtocolError::InvalidEnvelope);
                }
                Some(target)
            }
            MemoryCommand::RetireWordSpan { .. } => unreachable!(),
            MemoryCommand::PrepareCommit { target, request } => {
                if request.plans.is_empty()
                    || request.plans.len() > 64
                    || request.replacement.end().is_err()
                    || request.replaced_text.len() > 8_192
                    || request.retained_prefix.len() > 8_192
                    || request.plans.iter().any(|plan| {
                        !identifier(&plan.candidate_id)
                            || plan.inserted_text.is_empty()
                            || !bounded_text(&plan.inserted_text, 8_192)
                    })
                    || serde_json::to_vec(request).map_or(true, |raw| raw.len() > 96 * 1_024)
                {
                    return Err(ProtocolError::InvalidEnvelope);
                }
                Some(target)
            }
            MemoryCommand::ConfirmCommit {
                target,
                operation_id,
                commit_id,
                plan_id,
            } => {
                if operation_id != &format!("commit:{commit_id}:{plan_id}")
                    || !identifier(operation_id)
                    || !identifier(commit_id)
                    || !identifier(plan_id)
                {
                    return Err(ProtocolError::InvalidEnvelope);
                }
                Some(target)
            }
            MemoryCommand::LearnTyped {
                operation_id,
                event_id,
                target,
                text,
            } => {
                if !identifier(operation_id)
                    || !identifier(event_id)
                    || text.trim().is_empty()
                    || !bounded_text(text, 8_192)
                {
                    return Err(ProtocolError::InvalidEnvelope);
                }
                Some(target)
            }
            MemoryCommand::Import {
                operation_id,
                limit,
                ..
            } => {
                if !identifier(operation_id) || !(1..=MAX_IMPORT_LIMIT).contains(limit) {
                    return Err(ProtocolError::InvalidEnvelope);
                }
                None
            }
            MemoryCommand::Outcome { .. } => unreachable!(),
        };
        if let Some(target) = target {
            if target.field_id.as_deref() != Some(target.target_id.as_str()) {
                return Err(ProtocolError::InvalidEnvelope);
            }
            TargetBridgeRequest {
                request_id: self.request_id.clone(),
                client_instance: self.client_instance.clone(),
                server_instance: self.server_instance.clone(),
                policy_epoch: self.policy_epoch,
                target_bridge: TargetBridgeCommand::Validate {
                    target: (**target).clone(),
                    purpose: TargetBridgePurpose::Personalization,
                    operation_id: None,
                },
            }
            .validate_for(peer)?;
        }
        Ok(())
    }
    /// 完整查询身份摘要，绑定目标、组合状态、连接世代与策略，不将它用作词语隐私HMAC。
    pub fn query_digest(&self) -> Result<String, ProtocolError> {
        if !matches!(self.memory_domain, MemoryCommand::Query { .. }) {
            return Err(ProtocolError::InvalidEnvelope);
        }
        let bytes = serde_json::to_vec(self).map_err(|_| ProtocolError::InvalidEnvelope)?;
        if bytes.len() > MAX_SNAPSHOT_WIRE_BYTES {
            return Err(ProtocolError::FrameTooLarge);
        }
        let MemoryCommand::Query {
            query_id,
            query_generation,
            target,
            composing,
            query,
        } = &self.memory_domain
        else {
            return Err(ProtocolError::InvalidEnvelope);
        };
        query
            .validate()
            .map_err(|_| ProtocolError::InvalidEnvelope)?;
        let mut hash = Sha256::new();
        hash.update(b"inputia-memory-query-v1\0");
        // UTF-8字符串前置u64大端字节长度；整数原样u64大端；Option为0/1标签。
        // 明确字段顺序，与JSON对象键顺序、slash或Unicode转义无关。
        for value in [
            &self.request_id,
            &self.client_instance,
            &self.server_instance,
        ] {
            digest_string(&mut hash, value);
        }
        digest_u64(&mut hash, self.policy_epoch);
        digest_string(&mut hash, query_id);
        digest_u64(&mut hash, *query_generation);
        for value in [
            &target.target_id,
            &target.host_instance,
            &target.controller_id,
        ] {
            digest_string(&mut hash, value);
        }
        digest_u64(&mut hash, target.activation_generation);
        digest_option(&mut hash, target.field_id.as_deref());
        digest_u64(&mut hash, target.selection_generation);
        digest_u64(&mut hash, target.composition_generation);
        digest_option(&mut hash, target.source_app.as_deref());
        digest_string(&mut hash, composing);
        match query {
            MemoryQuery::Rank { candidate_texts } => {
                digest_string(&mut hash, "rank");
                digest_u64(&mut hash, candidate_texts.len() as u64);
                for value in candidate_texts {
                    digest_string(&mut hash, value);
                }
            }
            MemoryQuery::Completion { prefix, limit } => {
                digest_string(&mut hash, "completion");
                digest_string(&mut hash, prefix);
                digest_u64(&mut hash, *limit as u64);
            }
            MemoryQuery::EnglishCompletion { prefix, limit } => {
                digest_string(&mut hash, "english_completion");
                digest_string(&mut hash, prefix);
                digest_u64(&mut hash, *limit as u64);
            }
            MemoryQuery::Clipboard { limit } => {
                digest_string(&mut hash, "clipboard");
                digest_u64(&mut hash, *limit as u64);
            }
            MemoryQuery::VoiceHotwords { limit } => {
                digest_string(&mut hash, "voice_hotwords");
                digest_u64(&mut hash, *limit as u64);
            }
        }
        Ok(format!("{:x}", hash.finalize()))
    }
}

fn digest_u64(hash: &mut Sha256, value: u64) {
    hash.update(value.to_be_bytes());
}
fn digest_string(hash: &mut Sha256, value: &str) {
    digest_u64(hash, value.len() as u64);
    hash.update(value.as_bytes());
}
fn digest_option(hash: &mut Sha256, value: Option<&str>) {
    hash.update([u8::from(value.is_some())]);
    if let Some(value) = value {
        digest_string(hash, value);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MemoryReply {
    pub status: String,
    pub request_id: String,
    pub server_instance: String,
    pub profile_id: String,
    pub policy_epoch: u64,
    pub result: Option<MemoryResult>,
    pub code: Option<String>,
    pub privacy_barrier: Option<crate::voice_protocol::VoicePolicyBarrier>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum MemoryResult {
    PreparedWordSpan {
        permit: crate::memory_word_span::PreparedWordSpan,
    },
    WordSpanProgress {
        progress: crate::memory_word_span::SpanProgress,
    },
    WordSpanRetired {},
    PreparedCommit {
        permit: crate::memory_commit::PreparedFixedCommit,
    },
    Policy {
        domain: crate::legacy_memory::MemoryDomainStatus,
    },
    Snapshot {
        snapshot: MemorySnapshotReply,
    },
    Learn {
        receipt: crate::legacy_memory::MemoryMutationReceipt,
    },
    Import {
        operation: crate::legacy_memory::MemoryImportStatus,
    },
    Outcome {
        operation: Option<crate::legacy_memory::MemoryOperationStatus>,
    },
}

/// 该DTO必须由已认证连接发出；只读字段正确不代表数据来源可信。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MemorySnapshotReply {
    pub format_version: u32,
    pub request_id: String,
    pub server_instance: String,
    pub profile_id: String,
    pub policy_epoch: u64,
    pub domain_uuid: String,
    pub generation: u64,
    pub query_id: String,
    pub query_generation: u64,
    pub query_digest: String,
    pub query: MemoryQuery,
    pub terms: Vec<MemoryTerm>,
    pub lease_id: String,
    /// 从请求开始时计时，不从收到响应时重新发放。
    pub max_age_ms: u64,
}
impl MemorySnapshotReply {
    pub fn validate_for(
        &self,
        request: &MemoryRequest,
        profile_id: &str,
    ) -> Result<MemorySnapshot, ProtocolError> {
        let MemoryCommand::Query {
            query_id,
            query_generation,
            query,
            ..
        } = &request.memory_domain
        else {
            return Err(ProtocolError::InvalidEnvelope);
        };
        if self.format_version != 1
            || self.request_id != request.request_id
            || self.server_instance != request.server_instance
            || self.profile_id != profile_id
            || self.policy_epoch != request.policy_epoch
            || self.query_id != *query_id
            || self.query_generation != *query_generation
            || self.query != *query
            || self.query_digest != request.query_digest()?
            || !identifier(&self.domain_uuid)
            || !identifier(&self.lease_id)
            || self.generation == 0
            || !(1..=MAX_SNAPSHOT_AGE_MS).contains(&self.max_age_ms)
            || serde_json::to_vec(self).map_or(true, |bytes| bytes.len() > MAX_SNAPSHOT_WIRE_BYTES)
        {
            return Err(ProtocolError::InvalidEnvelope);
        }
        MemorySnapshot::new(self.query.clone(), self.terms.clone())
            .map_err(|_| ProtocolError::InvalidEnvelope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn peer() -> VoicePeer<'static> {
        VoicePeer {
            server_instance: "server-1",
            client_instance: "host-1",
            policy_epoch: 2,
            policy_applied: true,
        }
    }
    fn request() -> MemoryRequest {
        MemoryRequest {
            request_id: "request-1".into(),
            client_instance: "host-1".into(),
            server_instance: "server-1".into(),
            policy_epoch: 2,
            memory_domain: MemoryCommand::Query {
                query_id: "query-1".into(),
                query_generation: 1,
                target: Box::new(HostTargetToken {
                    target_id: "field-1".into(),
                    host_instance: "host-1".into(),
                    controller_id: "controller-1".into(),
                    activation_generation: 1,
                    field_id: Some("field-1".into()),
                    selection_generation: 1,
                    composition_generation: 1,
                    source_app: Some("com.example.editor".into()),
                }),
                composing: "ni".into(),
                query: MemoryQuery::Rank {
                    candidate_texts: vec!["你".into(), "泥".into()],
                },
            },
        }
    }
    #[test]
    fn digest_matches_cross_language_utf8_golden_vectors() {
        let vectors: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/memory-query-digest-v1.json"
        ))
        .unwrap();
        for vector in vectors.as_array().unwrap() {
            let request: MemoryRequest = serde_json::from_value(vector["request"].clone()).unwrap();
            assert_eq!(request.query_digest().unwrap(), vector["sha256"]);
        }
    }
    #[test]
    fn peer_epoch_target_and_management_payload_are_not_self_authorized() {
        let mut value = request();
        value.validate_for(&peer()).unwrap();
        value.policy_epoch = 1;
        assert!(value.validate_for(&peer()).is_err());
        value.memory_domain = MemoryCommand::Outcome {
            operation_id: "operation-1".into(),
        };
        value.validate_for(&peer()).unwrap();
        value.client_instance = "other".into();
        assert!(value.validate_for(&peer()).is_err());
        for raw in [
            r#"{"kind":"policy","text":"secret"}"#,
            r#"{"kind":"import","operation_id":"op-1","selection":"both","limit":10,"path":"/tmp/other.db"}"#,
            r#"{"kind":"learn_typed","operation_id":"op-1","event_id":"event-1","source":"voice","verified":true}"#,
        ] {
            assert!(serde_json::from_str::<MemoryCommand>(raw).is_err());
        }
    }
    #[test]
    fn commit_confirmation_binds_original_server_plan_and_rejects_new_text() {
        let mut value = request();
        let MemoryCommand::Query { target, .. } = value.memory_domain else {
            unreachable!()
        };
        value.memory_domain = MemoryCommand::PrepareCommit {
            target: target.clone(),
            request: crate::memory_commit::FixedPlansRequest {
                replacement: crate::memory_commit::TextRange {
                    location: 2,
                    length: 0,
                },
                replaced_text: String::new(),
                retained_prefix: "in".into(),
                plans: vec![crate::memory_commit::FixedPlan {
                    candidate_id: "candidate-1".into(),
                    inserted_text: "putia".into(),
                }],
            },
        };
        value.validate_for(&peer()).unwrap();
        value.memory_domain = MemoryCommand::ConfirmCommit {
            target,
            operation_id: "commit:permit-1:plan-1".into(),
            commit_id: "permit-1".into(),
            plan_id: "plan-1".into(),
        };
        value.validate_for(&peer()).unwrap();
        let mut raw = serde_json::to_value(&value.memory_domain).unwrap();
        raw["text"] = "different".into();
        assert!(serde_json::from_value::<MemoryCommand>(raw).is_err());
        if let MemoryCommand::ConfirmCommit { operation_id, .. } = &mut value.memory_domain {
            *operation_id = "commit:permit-1:plan-2".into();
        }
        assert!(value.validate_for(&peer()).is_err());
    }
    #[test]
    fn word_span_seal_has_distinct_operation_identity_and_retirement_is_metadata_only() {
        let mut value = request();
        let MemoryCommand::Query { target, .. } = value.memory_domain else {
            unreachable!()
        };
        value.memory_domain = MemoryCommand::CheckpointWordSpan {
            target,
            span_id: "span-1".into(),
            operation_id: "word-span:span-1:3:seal".into(),
            through_sequence: 3,
            finish: true,
        };
        value.validate_for(&peer()).unwrap();
        if let MemoryCommand::CheckpointWordSpan { finish, .. } = &mut value.memory_domain {
            *finish = false;
        }
        assert!(value.validate_for(&peer()).is_err());
        value.memory_domain = MemoryCommand::RetireWordSpan {
            span_id: "span-1".into(),
        };
        value.policy_epoch = 1;
        value.validate_for(&peer()).unwrap();
        value.client_instance = "other-client".into();
        assert!(value.validate_for(&peer()).is_err());
        let raw = r#"{"kind":"retire_word_span","span_id":"span-1","text":"cannot-submit"}"#;
        assert!(serde_json::from_str::<MemoryCommand>(raw).is_err());
    }
    #[test]
    fn query_identity_and_snapshot_budget_bound_delayed_results() {
        let value = request();
        let MemoryCommand::Query { query, .. } = &value.memory_domain else {
            unreachable!()
        };
        let reply = MemorySnapshotReply {
            format_version: 1,
            request_id: value.request_id.clone(),
            server_instance: value.server_instance.clone(),
            profile_id: "profile-1".into(),
            policy_epoch: 2,
            domain_uuid: "domain-1".into(),
            generation: 1,
            query_id: "query-1".into(),
            query_generation: 1,
            query_digest: value.query_digest().unwrap(),
            query: query.clone(),
            terms: vec![MemoryTerm {
                text: "泥".into(),
                typed_count: 3,
                voice_count: 0,
                clipboard_count: 0,
                last_used_tick: 1,
            }],
            lease_id: "reader-1".into(),
            max_age_ms: 100,
        };
        reply.validate_for(&value, "profile-1").unwrap();
        assert!(reply.validate_for(&value, "profile-2").is_err());
        let mut changed = value.clone();
        if let MemoryCommand::Query { composing, .. } = &mut changed.memory_domain {
            *composing = "nihao".into();
        }
        assert!(reply.validate_for(&changed, "profile-1").is_err());
        let mut late = reply.clone();
        late.max_age_ms = 2_001;
        assert!(late.validate_for(&value, "profile-1").is_err());
        let mut forged = reply.clone();
        forged.terms[0].text = "未请求".into();
        assert!(forged.validate_for(&value, "profile-1").is_err());
    }
}
