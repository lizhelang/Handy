//! 提交前后的有界范围证明；不插入文本，不从客户端的verified或字段包含关系生成证据。
//! 生产Observer必须在原生主线程持有同一字段，执行每次读前/后的身份与权限检查。
use crate::voice_protocol::HostTargetToken;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, &'static str>;
pub const MAX_TEXT_BYTES: usize = 8_192;
const MAX_PLANS: usize = 64;
const MAX_BATCH_BYTES: usize = 64 * 1_024;
const MAX_PERMITS: usize = 32;
const LEASE_MS: u64 = 1_500;
const ANCHOR_UNITS: u64 = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextRange {
    pub location: u64,
    pub length: u64,
}
impl TextRange {
    pub fn end(self) -> Result<u64> {
        self.location
            .checked_add(self.length)
            .ok_or("memory_range_invalid")
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldCheckpoint {
    /// 服务端按真实 AX 字段分配；多个 target grant 指向同字段时保持相同。
    pub field_instance: String,
    pub selection: TextRange,
    pub document_units: u64,
    pub focus_generation: u64,
    pub edit_generation: u64,
}
pub trait LearningObserver {
    fn checkpoint(&mut self) -> Result<FieldCheckpoint>;
    /// 精确UTF-16范围；锚点可处于代理对中间，只有完整学习结果才转换为String。
    fn read_range(&mut self, range: TextRange) -> Result<Vec<u16>>;
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitIdentity {
    pub client_instance: String,
    pub server_instance: String,
    pub permission_epoch: u64,
    pub policy_epoch: u64,
    pub target: HostTargetToken,
}
impl CommitIdentity {
    fn validate(&self) -> Result<()> {
        for value in [
            &self.client_instance,
            &self.server_instance,
            &self.target.target_id,
            &self.target.controller_id,
        ] {
            identifier(value)?;
        }
        if self.permission_epoch == 0
            || self.policy_epoch == 0
            || self.target.host_instance != self.client_instance
            || self.target.field_id.as_deref() != Some(self.target.target_id.as_str())
            || self.target.source_app.as_deref().is_none_or(str::is_empty)
        {
            return Err("memory_commit_identity_invalid");
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FixedPlan {
    pub candidate_id: String,
    pub inserted_text: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FixedPlansRequest {
    pub replacement: TextRange,
    pub replaced_text: String,
    pub retained_prefix: String,
    pub plans: Vec<FixedPlan>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PreparedPlan {
    pub candidate_id: String,
    pub plan_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PreparedFixedCommit {
    pub commit_id: String,
    pub plans: Vec<PreparedPlan>,
    pub max_age_ms: u64,
}

struct StoredPlan {
    text: String,
    range: TextRange,
    caret: u64,
    document_units: u64,
    right_start: u64,
}
struct FixedPermit {
    identity: CommitIdentity,
    deadline: Instant,
    before: FieldCheckpoint,
    left: (TextRange, Vec<u16>),
    right: Vec<u16>,
    plans: BTreeMap<String, StoredPlan>,
    confirmed: Option<(String, String, ConfirmedCommit)>,
}
/// 私有正文只能由原字段的精确观察构造，读取不支持或变化时没有成功结果。
#[derive(Clone)]
pub struct ConfirmedCommit {
    commit_id: String,
    identity: CommitIdentity,
    text: String,
}
impl ConfirmedCommit {
    pub fn text(&self) -> &str {
        &self.text
    }
    pub fn into_learning(
        self,
        operation_id: String,
    ) -> Result<(
        crate::legacy_memory::MemoryIntent,
        crate::legacy_memory::VerifiedMemoryEvidence,
    )> {
        identifier(&operation_id)?;
        let evidence = crate::legacy_memory::VerifiedMemoryEvidence::confirmed_commit(
            self.commit_id.clone(),
            self.identity.target.target_id.clone(),
            self.text.clone(),
            self.text.clone(),
            self.identity.policy_epoch,
            self.identity
                .target
                .source_app
                .clone()
                .ok_or("memory_commit_identity_invalid")?,
            &inputia_core::AppPolicy::default(),
        )
        .map_err(|_| "memory_commit_unverified")?;
        Ok((
            crate::legacy_memory::MemoryIntent {
                operation_id,
                event_id: self.commit_id,
                expected_epoch: self.identity.policy_epoch,
                source: crate::legacy_memory::MemoryOrigin::Typed,
                text: self.text,
            },
            evidence,
        ))
    }
}
#[derive(Default)]
pub struct CommitRegistry {
    fixed: BTreeMap<String, FixedPermit>,
}
impl CommitRegistry {
    pub fn clear(&mut self) {
        self.fixed.clear();
    }
    pub fn retain_current(&mut self, identity: &CommitIdentity) {
        let now = Instant::now();
        self.fixed.retain(|_, permit| {
            now < permit.deadline
                && permit.identity.permission_epoch == identity.permission_epoch
                && (permit.identity.client_instance != identity.client_instance
                    || permit.identity.server_instance != identity.server_instance
                    || permit.identity.policy_epoch == identity.policy_epoch)
        });
    }
    pub fn retire_commit(&mut self, commit_id: &str) {
        self.fixed.remove(commit_id);
    }
    pub fn constrain_deadline(&mut self, commit_id: &str, deadline: Instant) -> Result<()> {
        let permit = self
            .fixed
            .get_mut(commit_id)
            .ok_or("memory_commit_unknown")?;
        permit.deadline = permit.deadline.min(deadline);
        if Instant::now() >= permit.deadline {
            self.fixed.remove(commit_id);
            return Err("memory_commit_expired");
        }
        Ok(())
    }
    pub fn retire_target(&mut self, target: &str) {
        self.fixed
            .retain(|_, permit| permit.identity.target.target_id != target);
    }
    pub fn retire_owner(&mut self, owner: &str, server: &str) {
        self.fixed.retain(|_, permit| {
            permit.identity.client_instance != owner || permit.identity.server_instance != server
        });
    }
    pub fn prepare_fixed(
        &mut self,
        identity: CommitIdentity,
        request: FixedPlansRequest,
        observer: &mut impl LearningObserver,
    ) -> Result<PreparedFixedCommit> {
        self.prepare_at(identity, request, observer, Instant::now, opaque)
    }
    fn prepare_at(
        &mut self,
        identity: CommitIdentity,
        request: FixedPlansRequest,
        observer: &mut impl LearningObserver,
        clock: impl Fn() -> Instant,
        mut nonce: impl FnMut() -> Result<String>,
    ) -> Result<PreparedFixedCommit> {
        identity.validate()?;
        let started = clock();
        self.fixed.retain(|_, permit| started < permit.deadline);
        if request.plans.is_empty() || request.plans.len() > MAX_PLANS {
            return Err("memory_commit_budget");
        }
        let mut size = request
            .replaced_text
            .len()
            .checked_add(request.retained_prefix.len())
            .ok_or("memory_commit_budget")?;
        let mut ids = std::collections::BTreeSet::new();
        for plan in &request.plans {
            identifier(&plan.candidate_id)?;
            valid_text(&plan.inserted_text)?;
            size = size
                .checked_add(plan.inserted_text.len())
                .ok_or("memory_commit_budget")?;
            if !ids.insert(&plan.candidate_id) || size > MAX_BATCH_BYTES {
                return Err("memory_commit_budget");
            }
        }
        if request.replaced_text.len() > MAX_TEXT_BYTES
            || request.retained_prefix.len() > MAX_TEXT_BYTES
            || request.replaced_text.chars().any(char::is_control)
            || request.retained_prefix.chars().any(char::is_control)
        {
            return Err("memory_commit_text_invalid");
        }
        if !request.retained_prefix.is_empty()
            && (request.replacement.length != 0
                || !request
                    .retained_prefix
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
                || request.plans.iter().any(|plan| {
                    !plan
                        .inserted_text
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
                }))
        {
            return Err("memory_commit_prefix_invalid");
        }
        let before = observer.checkpoint()?;
        identifier(&before.field_instance)?;
        let end = request.replacement.end()?;
        // 覆盖真实选区，或覆盖caret所在的已读回marked片段；不能选择另一个预存相同词。
        let selection_matches = before.selection == request.replacement
            || (before.selection.length == 0
                && request.replacement.location <= before.selection.location
                && before.selection.location <= end);
        if !selection_matches
            || end > before.document_units
            || request.replacement.length != units(&request.replaced_text)
        {
            return Err("memory_commit_precondition_changed");
        }
        if read(observer, request.replacement)?
            != request.replaced_text.encode_utf16().collect::<Vec<_>>()
        {
            return Err("memory_commit_precondition_changed");
        }
        let prefix_units = units(&request.retained_prefix);
        let learn_start = request
            .replacement
            .location
            .checked_sub(prefix_units)
            .ok_or("memory_range_invalid")?;
        let prefix_range = TextRange {
            location: learn_start,
            length: prefix_units,
        };
        let prefix = read(observer, prefix_range)?;
        if prefix != request.retained_prefix.encode_utf16().collect::<Vec<_>>() {
            return Err("memory_commit_prefix_changed");
        }
        let left_range = TextRange {
            location: learn_start.saturating_sub(ANCHOR_UNITS),
            length: learn_start.min(ANCHOR_UNITS),
        };
        let right_range = TextRange {
            location: end,
            length: (before.document_units - end).min(ANCHOR_UNITS),
        };
        let left = read(observer, left_range)?;
        let right = read(observer, right_range)?;
        if observer.checkpoint()? != before {
            return Err("memory_commit_precondition_changed");
        }
        let mut plans = BTreeMap::new();
        let mut published = Vec::new();
        for plan in request.plans {
            // 原位置已经完全相同，无法由范围变化证明发生了新的提交。
            if plan.inserted_text == request.replaced_text {
                return Err("memory_commit_no_observed_change");
            }
            let text = format!("{}{}", request.retained_prefix, plan.inserted_text);
            valid_text(&text)?;
            // 英文结果必须覆盖完整词；空 prefix 也不能把旧词的尾部冒充一次完整提交。
            if text.bytes().all(word_byte)
                && (left.last().is_some_and(|unit| word_unit(*unit))
                    || right.first().is_some_and(|unit| word_unit(*unit)))
            {
                return Err("memory_commit_word_boundary");
            }
            let inserted_units = units(&plan.inserted_text);
            let caret = request
                .replacement
                .location
                .checked_add(inserted_units)
                .ok_or("memory_range_invalid")?;
            let document_units = (before.document_units - request.replacement.length)
                .checked_add(inserted_units)
                .ok_or("memory_range_invalid")?;
            let plan_id = nonce()?;
            if plans.contains_key(&plan_id) {
                return Err("memory_commit_entropy_failed");
            }
            plans.insert(
                plan_id.clone(),
                StoredPlan {
                    range: TextRange {
                        location: learn_start,
                        length: units(&text),
                    },
                    text,
                    caret,
                    document_units,
                    right_start: caret,
                },
            );
            published.push(PreparedPlan {
                candidate_id: plan.candidate_id,
                plan_id,
            });
        }
        let deadline = started + Duration::from_millis(LEASE_MS);
        if clock() >= deadline {
            return Err("memory_commit_expired");
        }
        let commit_id = nonce()?;
        if self.fixed.contains_key(&commit_id) {
            return Err("memory_commit_entropy_failed");
        }
        // 同一真实字段只有一份未确认计划。不同 target grant 不能重复计算同一次字段变化。
        self.fixed.retain(|_, permit| {
            permit.confirmed.is_some() || permit.before.field_instance != before.field_instance
        });
        if self.fixed.len() >= MAX_PERMITS {
            return Err("memory_commit_budget");
        }
        self.fixed.insert(
            commit_id.clone(),
            FixedPermit {
                identity,
                deadline,
                before,
                left: (left_range, left),
                right,
                plans,
                confirmed: None,
            },
        );
        Ok(PreparedFixedCommit {
            commit_id,
            plans: published,
            max_age_ms: LEASE_MS,
        })
    }
    pub fn confirm_fixed(
        &mut self,
        identity: &CommitIdentity,
        commit_id: &str,
        plan_id: &str,
        operation_id: &str,
        observer: &mut impl LearningObserver,
    ) -> Result<ConfirmedCommit> {
        self.confirm_at(
            identity,
            commit_id,
            plan_id,
            operation_id,
            observer,
            Instant::now,
        )
    }
    fn confirm_at(
        &mut self,
        identity: &CommitIdentity,
        commit_id: &str,
        plan_id: &str,
        operation_id: &str,
        observer: &mut impl LearningObserver,
        clock: impl Fn() -> Instant,
    ) -> Result<ConfirmedCommit> {
        identifier(operation_id)?;
        let permit = self
            .fixed
            .get_mut(commit_id)
            .ok_or("memory_commit_unknown")?;
        if &permit.identity != identity || clock() >= permit.deadline {
            return Err("memory_commit_expired");
        }
        if let Some((operation, plan, evidence)) = &permit.confirmed {
            return if operation == operation_id && plan == plan_id {
                Ok(evidence.clone())
            } else {
                Err("memory_commit_replayed")
            };
        }
        let plan = permit
            .plans
            .get(plan_id)
            .ok_or("memory_commit_unknown_plan")?;
        let after = observer.checkpoint()?;
        if after.field_instance != permit.before.field_instance
            || after.focus_generation != permit.before.focus_generation
            || after.edit_generation < permit.before.edit_generation
            || after.selection
                != (TextRange {
                    location: plan.caret,
                    length: 0,
                })
            || after.document_units != plan.document_units
        {
            return Err("memory_commit_postcondition_changed");
        }
        let actual = read(observer, plan.range)?;
        let actual = String::from_utf16(&actual).map_err(|_| "memory_commit_readback_invalid")?;
        if actual != plan.text
            || read(observer, permit.left.0)? != permit.left.1
            || read(
                observer,
                TextRange {
                    location: plan.right_start,
                    length: permit.right.len() as u64,
                },
            )? != permit.right
        {
            return Err("memory_commit_postcondition_changed");
        }
        if observer.checkpoint()? != after || clock() >= permit.deadline {
            return Err("memory_commit_postcondition_changed");
        }
        let evidence = ConfirmedCommit {
            commit_id: commit_id.into(),
            identity: identity.clone(),
            text: actual,
        };
        permit.confirmed = Some((operation_id.into(), plan_id.into(), evidence.clone()));
        Ok(evidence)
    }
}
fn word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"_-".contains(&byte)
}
fn word_unit(unit: u16) -> bool {
    u8::try_from(unit).is_ok_and(word_byte)
}
fn units(value: &str) -> u64 {
    value.encode_utf16().count() as u64
}
fn valid_text(value: &str) -> Result<()> {
    if value.trim().is_empty()
        || value.len() > MAX_TEXT_BYTES
        || value.chars().any(char::is_control)
    {
        Err("memory_commit_text_invalid")
    } else {
        Ok(())
    }
}
fn identifier(value: &str) -> Result<()> {
    inputia_core::integration::events::Identifier::parse(value)
        .map(|_| ())
        .map_err(|_| "memory_commit_identifier_invalid")
}
fn read(observer: &mut impl LearningObserver, range: TextRange) -> Result<Vec<u16>> {
    if range.length > MAX_TEXT_BYTES as u64 {
        return Err("memory_commit_budget");
    }
    if range.length == 0 {
        return Ok(Vec::new());
    }
    let value = observer.read_range(range)?;
    if value.len() as u64 != range.length {
        return Err("memory_commit_readback_invalid");
    }
    Ok(value)
}
fn opaque() -> Result<String> {
    let mut bytes = [0u8; 24];
    getrandom::getrandom(&mut bytes).map_err(|_| "memory_commit_entropy_failed")?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Field {
        units: Vec<u16>,
        meta: FieldCheckpoint,
        fail_read: bool,
        change_after_read: bool,
    }
    impl Field {
        fn new(text: &str, location: u64) -> Self {
            Self {
                units: text.encode_utf16().collect(),
                meta: FieldCheckpoint {
                    field_instance: "native-field-1".into(),
                    selection: TextRange {
                        location,
                        length: 0,
                    },
                    document_units: units(text),
                    focus_generation: 3,
                    edit_generation: 7,
                },
                fail_read: false,
                change_after_read: false,
            }
        }
        fn replace(&mut self, range: TextRange, text: &str) {
            self.units.splice(
                range.location as usize..range.end().unwrap() as usize,
                text.encode_utf16(),
            );
            self.meta.selection = TextRange {
                location: range.location + units(text),
                length: 0,
            };
            self.meta.document_units = self.units.len() as u64;
            self.meta.edit_generation += 1;
        }
    }
    impl LearningObserver for Field {
        fn checkpoint(&mut self) -> Result<FieldCheckpoint> {
            Ok(self.meta.clone())
        }
        fn read_range(&mut self, range: TextRange) -> Result<Vec<u16>> {
            if self.fail_read {
                return Err("fixture_unsupported");
            }
            let value = self
                .units
                .get(range.location as usize..range.end()? as usize)
                .ok_or("fixture_range")?
                .to_vec();
            if self.change_after_read {
                self.meta.edit_generation += 1;
            }
            Ok(value)
        }
    }
    fn identity() -> CommitIdentity {
        CommitIdentity {
            client_instance: "host-1".into(),
            server_instance: "server-1".into(),
            permission_epoch: 1,
            policy_epoch: 2,
            target: HostTargetToken {
                target_id: "field-1".into(),
                host_instance: "host-1".into(),
                controller_id: "controller-1".into(),
                activation_generation: 1,
                field_id: Some("field-1".into()),
                selection_generation: 1,
                composition_generation: 1,
                source_app: Some("com.example.editor".into()),
            },
        }
    }
    fn request() -> FixedPlansRequest {
        FixedPlansRequest {
            replacement: TextRange {
                location: 2,
                length: 2,
            },
            replaced_text: "ni".into(),
            retained_prefix: String::new(),
            plans: vec![FixedPlan {
                candidate_id: "candidate-1".into(),
                inserted_text: "你🙂".into(),
            }],
        }
    }
    #[test]
    fn exact_utf16_range_confirms_once_and_operation_retry_reuses_evidence() {
        let mut field = Field::new("前 ni 后", 4);
        let mut registry = CommitRegistry::default();
        let req = request();
        let permit = registry
            .prepare_fixed(identity(), req.clone(), &mut field)
            .unwrap();
        assert!(registry
            .confirm_fixed(
                &identity(),
                &permit.commit_id,
                &permit.plans[0].plan_id,
                "operation-1",
                &mut field
            )
            .is_err());
        field.replace(req.replacement, "你🙂");
        let evidence = registry
            .confirm_fixed(
                &identity(),
                &permit.commit_id,
                &permit.plans[0].plan_id,
                "operation-1",
                &mut field,
            )
            .unwrap();
        assert_eq!(evidence.text(), "你🙂");
        field.fail_read = true;
        assert!(registry
            .confirm_fixed(
                &identity(),
                &permit.commit_id,
                &permit.plans[0].plan_id,
                "operation-1",
                &mut field
            )
            .is_ok());
        assert!(registry
            .confirm_fixed(
                &identity(),
                &permit.commit_id,
                &permit.plans[0].plan_id,
                "operation-2",
                &mut field
            )
            .is_err());
        let (intent, _) = evidence.into_learning("operation-1".into()).unwrap();
        assert_eq!(intent.event_id, permit.commit_id);
        assert_eq!(intent.text, "你🙂");
    }
    #[test]
    fn english_suffix_learns_actual_prefix_case_and_whole_word() {
        let mut field = Field::new("X inp Y", 5);
        let req = FixedPlansRequest {
            replacement: TextRange {
                location: 5,
                length: 0,
            },
            replaced_text: String::new(),
            retained_prefix: "inp".into(),
            plans: vec![FixedPlan {
                candidate_id: "inputia".into(),
                inserted_text: "utia".into(),
            }],
        };
        let mut registry = CommitRegistry::default();
        let permit = registry
            .prepare_fixed(identity(), req.clone(), &mut field)
            .unwrap();
        field.replace(req.replacement, "utia");
        let evidence = registry
            .confirm_fixed(
                &identity(),
                &permit.commit_id,
                &permit.plans[0].plan_id,
                "operation-1",
                &mut field,
            )
            .unwrap();
        assert_eq!(evidence.text(), "inputia");
        assert_ne!(evidence.text(), "Inputia");
        assert_ne!(evidence.text(), "utia");
    }
    #[test]
    fn english_plans_reject_truncated_prefix_empty_prefix_and_right_continuation() {
        for (text, caret, prefix) in [("input", 5, "put"), ("input", 5, ""), ("inpTail", 3, "inp")]
        {
            let mut field = Field::new(text, caret);
            let request = FixedPlansRequest {
                replacement: TextRange {
                    location: caret,
                    length: 0,
                },
                replaced_text: String::new(),
                retained_prefix: prefix.into(),
                plans: vec![FixedPlan {
                    candidate_id: "word".into(),
                    inserted_text: "ia".into(),
                }],
            };
            assert_eq!(
                CommitRegistry::default().prepare_fixed(identity(), request, &mut field),
                Err("memory_commit_word_boundary")
            );
        }
    }
    #[test]
    fn same_native_field_under_distinct_grants_cannot_double_count_one_commit() {
        let mut field = Field::new("前 ni 后", 4);
        let mut registry = CommitRegistry::default();
        let first = registry
            .prepare_fixed(identity(), request(), &mut field)
            .unwrap();
        let mut other = identity();
        other.target.target_id = "field-2".into();
        other.target.field_id = Some("field-2".into());
        let second = registry
            .prepare_fixed(other.clone(), request(), &mut field)
            .unwrap();
        field.replace(request().replacement, "你🙂");
        assert!(registry
            .confirm_fixed(
                &identity(),
                &first.commit_id,
                &first.plans[0].plan_id,
                "operation-1",
                &mut field
            )
            .is_err());
        assert_eq!(
            registry
                .confirm_fixed(
                    &other,
                    &second.commit_id,
                    &second.plans[0].plan_id,
                    "operation-2",
                    &mut field
                )
                .unwrap()
                .text(),
            "你🙂"
        );
        assert!(registry
            .confirm_fixed(
                &other,
                &second.commit_id,
                &second.plans[0].plan_id,
                "operation-2",
                &mut field
            )
            .is_ok());
    }
    #[test]
    fn existing_same_text_wrong_field_or_changed_anchors_do_not_prove_commit() {
        let mut registry = CommitRegistry::default();
        let mut same = request();
        same.plans[0].inserted_text = "ni".into();
        assert!(registry
            .prepare_fixed(identity(), same, &mut Field::new("前 ni 后", 4))
            .is_err());
        for mismatch in 0..4 {
            let mut field = Field::new("前 ni 后", 4);
            let permit = registry
                .prepare_fixed(identity(), request(), &mut field)
                .unwrap();
            field.replace(request().replacement, "你🙂");
            let mut owner = identity();
            match mismatch {
                0 => owner.target.target_id = "other-field".into(),
                1 => field.units[0] = '错' as u16,
                2 => field.meta.selection.location += 1,
                _ => field.meta.focus_generation += 1,
            }
            assert!(registry
                .confirm_fixed(
                    &owner,
                    &permit.commit_id,
                    &permit.plans[0].plan_id,
                    "operation-1",
                    &mut field
                )
                .is_err());
        }
    }
    #[test]
    fn unsupported_or_mid_read_changes_never_produce_evidence() {
        let mut registry = CommitRegistry::default();
        let mut field = Field::new("前 ni 后", 4);
        field.fail_read = true;
        assert!(registry
            .prepare_fixed(identity(), request(), &mut field)
            .is_err());
        field.fail_read = false;
        field.change_after_read = true;
        assert!(registry
            .prepare_fixed(identity(), request(), &mut field)
            .is_err());
        field.change_after_read = false;
        let permit = registry
            .prepare_fixed(identity(), request(), &mut field)
            .unwrap();
        field.replace(request().replacement, "你🙂");
        field.change_after_read = true;
        assert!(registry
            .confirm_fixed(
                &identity(),
                &permit.commit_id,
                &permit.plans[0].plan_id,
                "operation-1",
                &mut field
            )
            .is_err());
    }
    #[test]
    fn expired_or_retired_permits_cannot_be_recreated_from_current_text() {
        let mut registry = CommitRegistry::default();
        let mut field = Field::new("前 ni 后", 4);
        let start = Instant::now();
        let mut serial = 0;
        let permit = registry
            .prepare_at(
                identity(),
                request(),
                &mut field,
                || start,
                || {
                    serial += 1;
                    Ok(format!("nonce-{serial}"))
                },
            )
            .unwrap();
        field.replace(request().replacement, "你🙂");
        assert!(registry
            .confirm_at(
                &identity(),
                &permit.commit_id,
                &permit.plans[0].plan_id,
                "operation-1",
                &mut field,
                || start + Duration::from_millis(LEASE_MS)
            )
            .is_err());
        registry.retire_target("field-1");
        assert!(registry
            .confirm_at(
                &identity(),
                &permit.commit_id,
                &permit.plans[0].plan_id,
                "operation-1",
                &mut field,
                || start
            )
            .is_err());
    }
}
