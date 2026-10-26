//! 连续英文的先许可、后读回证明。只保存一个有界活跃段，不修改原字段或从旧正文补造许可。
use crate::memory_commit::{CommitIdentity, FieldCheckpoint, LearningObserver, TextRange};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};

pub const MAX_SPAN_UNITS: usize = 8_192;
pub const MAX_SPAN_BYTES: usize = 64 * 1_024;
pub const MAX_SPAN_WORDS: usize = 128;
const MAX_EVENTS: usize = 1_024;
const MAX_CHECKPOINTS: usize = 32;
const MAX_SPANS: usize = 8;
const LEASE_MS: u64 = 1_500;
const ANCHOR_UNITS: u64 = 32;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SpanEdit {
    Append { text: String },
    TailBackspace { units: u64 },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WordSpanCheckpoint {
    pub operation_id: String,
    pub through_sequence: u64,
    pub finish: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedWordSpan {
    pub span_id: String,
    pub max_age_ms: u64,
    pub max_units: usize,
    pub next_sequence: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpanProgress {
    pub span_id: String,
    pub sequence: u64,
    pub transcript_units: usize,
    pub replayed: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WordSpanReason {
    Unknown,
    Sealed,
    IdentityChanged,
    Expired,
    BudgetExceeded,
    InvalidSequence,
    ConflictingReplay,
    InvalidEdit,
    BoundaryRequired,
    ReadbackUnsupported,
    FieldChanged,
    ReadbackChanged,
    NoObservedChange,
    RevocationPending,
    InvalidRequest,
}
impl WordSpanReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "memory_span_unknown",
            Self::Sealed => "memory_span_sealed",
            Self::IdentityChanged => "memory_span_identity_changed",
            Self::Expired => "memory_span_expired",
            Self::BudgetExceeded => "memory_span_budget_exceeded",
            Self::InvalidSequence => "memory_span_sequence_invalid",
            Self::ConflictingReplay => "memory_span_replay_conflict",
            Self::InvalidEdit => "memory_span_edit_invalid",
            Self::BoundaryRequired => "memory_span_boundary_required",
            Self::ReadbackUnsupported => "memory_span_readback_unsupported",
            Self::FieldChanged => "memory_span_field_changed",
            Self::ReadbackChanged => "memory_span_readback_changed",
            Self::NoObservedChange => "memory_span_no_observed_change",
            Self::RevocationPending => "memory_span_revocation_pending",
            Self::InvalidRequest => "memory_span_request_invalid",
        }
    }
}
#[derive(Clone, Debug)]
pub struct WordSpanFailure {
    pub reason: WordSpanReason,
    pub revocation: Option<RevokedWordSpan>,
}
impl std::fmt::Display for WordSpanFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.reason.as_str())
    }
}
impl std::error::Error for WordSpanFailure {}
type Result<T> = std::result::Result<T, WordSpanFailure>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpanWord {
    pub offset: u64,
    pub text: String,
}
/// 只能由 registry 的真实整段读回产生；不接受 wire 反序列化。
#[derive(Clone)]
pub struct VerifiedWordSpan {
    span_id: String,
    operation_id: String,
    identity: CommitIdentity,
    revision: u64,
    words: Vec<SpanWord>,
    sealed: bool,
}
impl VerifiedWordSpan {
    pub fn span_id(&self) -> &str {
        &self.span_id
    }
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }
    pub fn identity(&self) -> &CommitIdentity {
        &self.identity
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn sealed(&self) -> bool {
        self.sealed
    }
    pub fn words(&self) -> &[SpanWord] {
        &self.words
    }
}
/// 正文已清除；此 token 可在租约过期或隐私撤销后幂等撤销精确旧 span。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RevokedWordSpan {
    span_id: String,
    identity: Arc<CommitIdentity>,
    revision: u64,
    operation_id: String,
}
impl RevokedWordSpan {
    pub fn span_id(&self) -> &str {
        &self.span_id
    }
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }
    pub fn identity(&self) -> &CommitIdentity {
        &self.identity
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
}
struct SpanPermit {
    identity: CommitIdentity,
    before: FieldCheckpoint,
    left: (TextRange, Vec<u16>),
    right: Vec<u16>,
    deadline: Instant,
    text: Vec<u16>,
    events: BTreeMap<u64, SpanEdit>,
    sequence: u64,
    revision: u64,
    confirmed_text: Vec<u16>,
    last_checkpoint: FieldCheckpoint,
    checkpoints: BTreeMap<String, (u64, VerifiedWordSpan)>,
}
struct CompletedSpan {
    durable: bool,
    identity: CommitIdentity,
    request: WordSpanCheckpoint,
    evidence: VerifiedWordSpan,
    deadline: Instant,
}
#[derive(Default)]
pub struct WordSpanRegistry {
    spans: BTreeMap<String, SpanPermit>,
    revoked: BTreeMap<String, RevokedWordSpan>,
    completed: BTreeMap<String, CompletedSpan>,
}
impl WordSpanRegistry {
    pub fn prepare(
        &mut self,
        identity: CommitIdentity,
        request_started: Instant,
        observer: &mut impl LearningObserver,
    ) -> Result<PreparedWordSpan> {
        self.prepare_at(identity, request_started, observer, Instant::now, opaque)
    }
    fn prepare_at(
        &mut self,
        identity: CommitIdentity,
        request_started: Instant,
        observer: &mut impl LearningObserver,
        clock: impl Fn() -> Instant,
        nonce: impl FnOnce() -> std::result::Result<String, WordSpanReason>,
    ) -> Result<PreparedWordSpan> {
        validate_identity(&identity).map_err(failure)?;
        let now = clock();
        let deadline = request_started + Duration::from_millis(LEASE_MS);
        if request_started > now || now >= deadline {
            return Err(failure(WordSpanReason::Expired));
        }
        self.retire_completed(|p| now >= p.deadline);
        if self.spans.len() + self.revoked.len() + self.completed.len() >= MAX_SPANS {
            return Err(failure(WordSpanReason::BudgetExceeded));
        }
        let before = observer
            .checkpoint()
            .map_err(|_| failure(WordSpanReason::ReadbackUnsupported))?;
        if !identifier(&before.field_instance)
            || before.selection.length != 0
            || before.selection.location > before.document_units
        {
            return Err(failure(WordSpanReason::BoundaryRequired));
        }
        if self
            .revoked
            .values()
            .any(|p| p.identity.target.target_id == identity.target.target_id)
        {
            return Err(failure(WordSpanReason::RevocationPending));
        }
        if let Some(id) = self
            .spans
            .iter()
            .find(|(_, p)| p.before.field_instance == before.field_instance)
            .map(|(id, _)| id.clone())
        {
            return Err(self.invalidate(&id, WordSpanReason::RevocationPending));
        }
        let caret = before.selection.location;
        let left_range = TextRange {
            location: caret.saturating_sub(ANCHOR_UNITS),
            length: caret.min(ANCHOR_UNITS),
        };
        let left = read(observer, left_range).map_err(failure)?;
        let right = read(
            observer,
            TextRange {
                location: caret,
                length: (before.document_units - caret).min(ANCHOR_UNITS),
            },
        )
        .map_err(failure)?;
        if left.last().is_some_and(|c| word_unit(*c))
            || right.first().is_some_and(|c| word_unit(*c))
        {
            return Err(failure(WordSpanReason::BoundaryRequired));
        }
        if observer
            .checkpoint()
            .map_err(|_| failure(WordSpanReason::ReadbackUnsupported))?
            != before
            || clock() >= deadline
        {
            return Err(failure(WordSpanReason::FieldChanged));
        }
        let span_id = nonce().map_err(failure)?;
        if !identifier(&span_id)
            || self.spans.contains_key(&span_id)
            || self.revoked.contains_key(&span_id)
        {
            return Err(failure(WordSpanReason::InvalidRequest));
        }
        self.spans.insert(
            span_id.clone(),
            SpanPermit {
                identity,
                before: before.clone(),
                left: (left_range, left),
                right,
                deadline,
                text: vec![],
                events: BTreeMap::new(),
                sequence: 0,
                revision: 0,
                confirmed_text: vec![],
                last_checkpoint: before,
                checkpoints: BTreeMap::new(),
            },
        );
        Ok(PreparedWordSpan {
            span_id,
            max_age_ms: LEASE_MS,
            max_units: MAX_SPAN_UNITS,
            next_sequence: 1,
        })
    }
    pub fn record(
        &mut self,
        identity: &CommitIdentity,
        span_id: &str,
        sequence: u64,
        edit: SpanEdit,
    ) -> Result<SpanProgress> {
        self.record_at(identity, span_id, sequence, edit, Instant::now())
    }
    fn record_at(
        &mut self,
        identity: &CommitIdentity,
        id: &str,
        sequence: u64,
        edit: SpanEdit,
        now: Instant,
    ) -> Result<SpanProgress> {
        if self.completed.contains_key(id) {
            return Err(failure(WordSpanReason::Sealed));
        }
        let result = (|| -> std::result::Result<SpanProgress, WordSpanReason> {
            let p = self.spans.get_mut(id).ok_or(WordSpanReason::Unknown)?;
            current(p, identity, now)?;
            if let Some(old) = p.events.get(&sequence) {
                if old != &edit {
                    return Err(WordSpanReason::ConflictingReplay);
                }
                return Ok(SpanProgress {
                    span_id: id.into(),
                    sequence,
                    transcript_units: p.text.len(),
                    replayed: true,
                });
            }
            if sequence
                != p.sequence
                    .checked_add(1)
                    .ok_or(WordSpanReason::BudgetExceeded)?
            {
                return Err(WordSpanReason::InvalidSequence);
            }
            if p.events.len() >= MAX_EVENTS {
                return Err(WordSpanReason::BudgetExceeded);
            }
            let mut text = p.text.clone();
            match &edit {
                SpanEdit::Append { text: appended } => {
                    if appended.is_empty()
                        || appended.len() > MAX_SPAN_BYTES
                        || appended
                            .chars()
                            .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
                    {
                        return Err(WordSpanReason::InvalidEdit);
                    }
                    text.extend(appended.encode_utf16());
                }
                SpanEdit::TailBackspace { units } => {
                    let units = usize::try_from(*units).map_err(|_| WordSpanReason::InvalidEdit)?;
                    if units == 0 || units > text.len() {
                        return Err(WordSpanReason::InvalidEdit);
                    }
                    text.truncate(text.len() - units);
                }
            }
            if text.len() > MAX_SPAN_UNITS {
                return Err(WordSpanReason::BudgetExceeded);
            }
            let decoded = String::from_utf16(&text).map_err(|_| WordSpanReason::InvalidEdit)?;
            if decoded.len() > MAX_SPAN_BYTES {
                return Err(WordSpanReason::BudgetExceeded);
            }
            if words(&text, p.before.selection.location)?.len() > MAX_SPAN_WORDS {
                return Err(WordSpanReason::BudgetExceeded);
            }
            p.text = text;
            p.sequence = sequence;
            p.events.insert(sequence, edit);
            Ok(SpanProgress {
                span_id: id.into(),
                sequence,
                transcript_units: p.text.len(),
                replayed: false,
            })
        })();
        result.map_err(|reason| self.invalidate(id, reason))
    }
    pub fn checkpoint(
        &mut self,
        identity: &CommitIdentity,
        span_id: &str,
        request: &WordSpanCheckpoint,
        request_started: Instant,
        observer: &mut impl LearningObserver,
    ) -> Result<VerifiedWordSpan> {
        self.checkpoint_at(
            identity,
            span_id,
            request,
            request_started,
            observer,
            Instant::now,
        )
    }
    fn checkpoint_at(
        &mut self,
        identity: &CommitIdentity,
        id: &str,
        request: &WordSpanCheckpoint,
        request_started: Instant,
        observer: &mut impl LearningObserver,
        clock: impl Fn() -> Instant,
    ) -> Result<VerifiedWordSpan> {
        let operation_id = request.operation_id.as_str();
        let sequence = request.through_sequence;
        if let Some(done) = self.completed.get(id) {
            return if &done.identity == identity
                && &done.request == request
                && clock() < done.deadline
            {
                Ok(done.evidence.clone())
            } else {
                Err(failure(WordSpanReason::Sealed))
            };
        }
        let result = (|| -> std::result::Result<VerifiedWordSpan, WordSpanReason> {
            let p = self.spans.get_mut(id).ok_or(WordSpanReason::Unknown)?;
            let now = clock();
            current(p, identity, now)?;
            if !identifier(operation_id)
                || operation_id
                    != format!(
                        "word-span:{id}:{sequence}{}",
                        if request.finish { ":seal" } else { "" }
                    )
                || request_started > now
            {
                return Err(WordSpanReason::InvalidRequest);
            }
            if let Some((old, evidence)) = p.checkpoints.get(operation_id) {
                return if *old == sequence {
                    Ok(evidence.clone())
                } else {
                    Err(WordSpanReason::ConflictingReplay)
                };
            }
            if sequence != p.sequence || sequence == 0 {
                return Err(WordSpanReason::InvalidSequence);
            }
            if p.checkpoints.len() >= MAX_CHECKPOINTS {
                return Err(WordSpanReason::BudgetExceeded);
            }
            if p.text == p.confirmed_text && !request.finish {
                return Err(WordSpanReason::NoObservedChange);
            }
            if request.finish && p.text.last().is_none_or(|c| word_unit(*c)) {
                return Err(WordSpanReason::BoundaryRequired);
            }
            let end = p
                .before
                .selection
                .location
                .checked_add(p.text.len() as u64)
                .ok_or(WordSpanReason::InvalidEdit)?;
            let expected_units = p
                .before
                .document_units
                .checked_add(p.text.len() as u64)
                .ok_or(WordSpanReason::InvalidEdit)?;
            let after = observer
                .checkpoint()
                .map_err(|_| WordSpanReason::ReadbackUnsupported)?;
            if after.field_instance != p.before.field_instance
                || after.focus_generation != p.before.focus_generation
                || after.edit_generation < p.last_checkpoint.edit_generation
                || after.selection
                    != (TextRange {
                        location: end,
                        length: 0,
                    })
                || after.document_units != expected_units
            {
                return Err(WordSpanReason::FieldChanged);
            }
            if read(
                observer,
                TextRange {
                    location: p.before.selection.location,
                    length: p.text.len() as u64,
                },
            )? != p.text
                || read(observer, p.left.0)? != p.left.1
                || read(
                    observer,
                    TextRange {
                        location: end,
                        length: p.right.len() as u64,
                    },
                )? != p.right
            {
                return Err(WordSpanReason::ReadbackChanged);
            }
            if observer
                .checkpoint()
                .map_err(|_| WordSpanReason::ReadbackUnsupported)?
                != after
                || clock() >= p.deadline
            {
                return Err(WordSpanReason::FieldChanged);
            }
            let renewed = request_started + Duration::from_millis(LEASE_MS);
            if clock() >= renewed {
                return Err(WordSpanReason::Expired);
            }
            let words = words(&p.text, p.before.selection.location)?;
            let revision = p
                .revision
                .checked_add(1)
                .ok_or(WordSpanReason::BudgetExceeded)?;
            let evidence = VerifiedWordSpan {
                span_id: id.into(),
                operation_id: operation_id.into(),
                identity: identity.clone(),
                revision,
                words,
                sealed: request.finish,
            };
            p.revision = revision;
            p.confirmed_text = p.text.clone();
            p.last_checkpoint = after;
            p.deadline = renewed;
            p.checkpoints
                .insert(operation_id.into(), (sequence, evidence.clone()));
            Ok(evidence)
        })();
        match result {
            Ok(evidence) => {
                if evidence.sealed {
                    let p = self
                        .spans
                        .remove(id)
                        .ok_or_else(|| failure(WordSpanReason::Unknown))?;
                    self.completed.insert(
                        id.into(),
                        CompletedSpan {
                            identity: p.identity,
                            request: request.clone(),
                            evidence: evidence.clone(),
                            deadline: p.deadline,
                            durable: false,
                        },
                    );
                }
                Ok(evidence)
            }
            Err(reason) => Err(self.invalidate(id, reason)),
        }
    }
    pub fn constrain_deadline(&mut self, id: &str, deadline: Instant) -> Result<()> {
        let current = if let Some(p) = self.spans.get_mut(id) {
            &mut p.deadline
        } else if let Some(p) = self.completed.get_mut(id) {
            &mut p.deadline
        } else {
            return Err(failure(WordSpanReason::Unknown));
        };
        *current = (*current).min(deadline);
        if Instant::now() >= *current {
            return Err(self.invalidate(id, WordSpanReason::Expired));
        }
        Ok(())
    }
    /// 调用方仅在持久域返回匹配成功回执后确认；丢失 ACK 的 sealed 由域回执保护。
    pub fn acknowledge_seal(&mut self, proof: &VerifiedWordSpan) -> bool {
        if let Some(p) = self.completed.get_mut(proof.span_id()) {
            if p.evidence.revision == proof.revision
                && p.evidence.operation_id == proof.operation_id
                && p.identity == proof.identity
                && proof.sealed
            {
                p.durable = true;
                return true;
            }
        }
        false
    }
    fn retire_completed(&mut self, test: impl Fn(&CompletedSpan) -> bool) {
        let ids: Vec<_> = self
            .completed
            .iter()
            .filter(|(_, p)| test(p))
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            self.invalidate(&id, WordSpanReason::Expired);
        }
    }
    fn invalidate(&mut self, id: &str, reason: WordSpanReason) -> WordSpanFailure {
        if let Some(done) = self.completed.remove(id) {
            if !done.durable {
                self.revoked.insert(
                    id.into(),
                    RevokedWordSpan {
                        span_id: id.into(),
                        operation_id: format!("span-revoke-{id}"),
                        identity: Arc::new(done.identity),
                        revision: done.evidence.revision.saturating_add(1),
                    },
                );
            }
        }

        if let Some(p) = self.spans.remove(id) {
            let token = RevokedWordSpan {
                span_id: id.into(),
                operation_id: format!("span-revoke-{id}"),
                identity: Arc::new(p.identity),
                revision: p.revision.saturating_add(1),
            };
            self.revoked.insert(id.into(), token);
        }
        WordSpanFailure {
            reason,
            revocation: self.revoked.get(id).cloned(),
        }
    }
    pub fn collect_expired(&mut self) -> Vec<RevokedWordSpan> {
        self.expire_at(Instant::now())
    }
    fn expire_at(&mut self, now: Instant) -> Vec<RevokedWordSpan> {
        self.retire_completed(|p| now >= p.deadline);
        let ids: Vec<_> = self
            .spans
            .iter()
            .filter(|(_, p)| now >= p.deadline)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            self.invalidate(&id, WordSpanReason::Expired);
        }
        self.pending_revocations()
    }
    pub fn retire_span(&mut self, client: &str, server: &str, id: &str) -> Result<RevokedWordSpan> {
        let identity = self
            .spans
            .get(id)
            .map(|p| &p.identity)
            .or_else(|| self.revoked.get(id).map(|p| p.identity.as_ref()))
            .or_else(|| self.completed.get(id).map(|p| &p.identity))
            .ok_or_else(|| failure(WordSpanReason::Unknown))?;
        if identity.client_instance != client || identity.server_instance != server {
            return Err(failure(WordSpanReason::IdentityChanged));
        }
        self.invalidate(id, WordSpanReason::IdentityChanged)
            .revocation
            .ok_or_else(|| failure(WordSpanReason::Unknown))
    }
    pub fn retire_target(&mut self, target: &str) -> Vec<RevokedWordSpan> {
        self.retire_completed(|p| p.identity.target.target_id == target);
        self.retire_where(|p| p.identity.target.target_id == target)
    }
    pub fn retire_owner(&mut self, client: &str, server: &str) -> Vec<RevokedWordSpan> {
        self.retire_completed(|p| {
            p.identity.client_instance == client && p.identity.server_instance == server
        });
        self.retire_where(|p| {
            p.identity.client_instance == client && p.identity.server_instance == server
        })
    }
    pub fn revoke_all(&mut self) -> Vec<RevokedWordSpan> {
        self.retire_completed(|_| true);
        self.retire_where(|_| true)
    }
    fn retire_where(&mut self, test: impl Fn(&SpanPermit) -> bool) -> Vec<RevokedWordSpan> {
        let ids: Vec<_> = self
            .spans
            .iter()
            .filter(|(_, p)| test(p))
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            self.invalidate(&id, WordSpanReason::IdentityChanged);
        }
        self.pending_revocations()
    }
    pub fn has_pending_or_active(&self) -> bool {
        !self.spans.is_empty() || !self.revoked.is_empty() || !self.completed.is_empty()
    }
    pub fn pending_revocations(&self) -> Vec<RevokedWordSpan> {
        self.revoked.values().cloned().collect()
    }
    pub fn acknowledge_revocation(&mut self, token: &RevokedWordSpan) -> bool {
        if self.revoked.get(token.span_id()) == Some(token) {
            self.revoked.remove(token.span_id());
            true
        } else {
            false
        }
    }
}
fn failure(reason: WordSpanReason) -> WordSpanFailure {
    WordSpanFailure {
        reason,
        revocation: None,
    }
}
fn current(
    p: &SpanPermit,
    identity: &CommitIdentity,
    now: Instant,
) -> std::result::Result<(), WordSpanReason> {
    if &p.identity != identity {
        return Err(WordSpanReason::IdentityChanged);
    }
    if now >= p.deadline {
        return Err(WordSpanReason::Expired);
    }
    Ok(())
}
fn validate_identity(i: &CommitIdentity) -> std::result::Result<(), WordSpanReason> {
    if [
        &i.client_instance,
        &i.server_instance,
        &i.target.target_id,
        &i.target.controller_id,
    ]
    .iter()
    .any(|s| !identifier(s))
        || i.permission_epoch == 0
        || i.policy_epoch == 0
        || i.target.host_instance != i.client_instance
        || i.target.field_id.as_deref() != Some(i.target.target_id.as_str())
        || i.target.source_app.as_deref().is_none_or(str::is_empty)
    {
        return Err(WordSpanReason::InvalidRequest);
    }
    Ok(())
}
fn identifier(v: &str) -> bool {
    inputia_core::integration::events::Identifier::parse(v).is_ok()
}
fn read(
    o: &mut impl LearningObserver,
    r: TextRange,
) -> std::result::Result<Vec<u16>, WordSpanReason> {
    let text = o
        .read_range(r)
        .map_err(|_| WordSpanReason::ReadbackUnsupported)?;
    if text.len() as u64 != r.length {
        return Err(WordSpanReason::ReadbackChanged);
    }
    Ok(text)
}
fn word_unit(v: u16) -> bool {
    u8::try_from(v).is_ok_and(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
}
fn words(text: &[u16], base: u64) -> std::result::Result<Vec<SpanWord>, WordSpanReason> {
    let mut result = vec![];
    let mut start = None;
    for (i, unit) in text.iter().enumerate() {
        if word_unit(*unit) {
            start.get_or_insert(i);
        } else if let Some(begin) = start.take() {
            let word = &text[begin..i];
            if word.len() >= 2
                && word
                    .iter()
                    .any(|c| u8::try_from(*c).is_ok_and(|b| b.is_ascii_alphabetic()))
            {
                result.push(SpanWord {
                    offset: base
                        .checked_add(begin as u64)
                        .ok_or(WordSpanReason::InvalidEdit)?,
                    text: String::from_utf16(word).map_err(|_| WordSpanReason::InvalidEdit)?,
                });
                if result.len() > MAX_SPAN_WORDS {
                    return Err(WordSpanReason::BudgetExceeded);
                }
            }
        }
    }
    // 未读到本次新增的结束边界，尾部词仍是未完成输入。
    Ok(result)
}
fn opaque() -> std::result::Result<String, WordSpanReason> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes).map_err(|_| WordSpanReason::InvalidRequest)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::legacy_memory::{LegacyMemory, LegacyMemoryContext};
    use crate::voice_protocol::HostTargetToken;
    use rusqlite::Connection;
    struct Observer {
        text: Vec<u16>,
        caret: u64,
        field: String,
        focus: u64,
        edit: u64,
        unsupported: bool,
    }
    impl Observer {
        fn new() -> Self {
            Self {
                text: vec![],
                caret: 0,
                field: "actual-field".into(),
                focus: 1,
                edit: 1,
                unsupported: false,
            }
        }
        fn append(&mut self, s: &str) {
            let units: Vec<_> = s.encode_utf16().collect();
            self.text.splice(
                self.caret as usize..self.caret as usize,
                units.iter().copied(),
            );
            self.caret += units.len() as u64;
            self.edit += 1;
        }
        fn backspace(&mut self, n: usize) {
            let end = self.caret as usize;
            self.text.drain(end - n..end);
            self.caret -= n as u64;
            self.edit += 1;
        }
    }
    impl LearningObserver for Observer {
        fn checkpoint(&mut self) -> std::result::Result<FieldCheckpoint, &'static str> {
            Ok(FieldCheckpoint {
                field_instance: self.field.clone(),
                selection: TextRange {
                    location: self.caret,
                    length: 0,
                },
                document_units: self.text.len() as u64,
                focus_generation: self.focus,
                edit_generation: self.edit,
            })
        }
        fn read_range(&mut self, r: TextRange) -> std::result::Result<Vec<u16>, &'static str> {
            if self.unsupported {
                return Err("unsupported");
            }
            self.text
                .get(r.location as usize..r.end()? as usize)
                .map(|v| v.to_vec())
                .ok_or("range")
        }
    }
    fn identity() -> CommitIdentity {
        CommitIdentity {
            client_instance: "host".into(),
            server_instance: "server".into(),
            permission_epoch: 1,
            policy_epoch: 1,
            target: HostTargetToken {
                target_id: "target".into(),
                host_instance: "host".into(),
                controller_id: "controller".into(),
                activation_generation: 1,
                field_id: Some("target".into()),
                selection_generation: 1,
                composition_generation: 1,
                source_app: Some("com.example.editor".into()),
            },
        }
    }
    fn begin(registry: &mut WordSpanRegistry, o: &mut Observer) -> String {
        registry
            .prepare(identity(), Instant::now(), o)
            .unwrap()
            .span_id
    }
    fn append(registry: &mut WordSpanRegistry, id: &str, seq: u64, text: &str, o: &mut Observer) {
        registry
            .record(&identity(), id, seq, SpanEdit::Append { text: text.into() })
            .unwrap();
        o.append(text);
    }
    fn request(id: &str, seq: u64, finish: bool) -> WordSpanCheckpoint {
        WordSpanCheckpoint {
            operation_id: format!("word-span:{id}:{seq}{}", if finish { ":seal" } else { "" }),
            through_sequence: seq,
            finish,
        }
    }
    fn check(
        registry: &mut WordSpanRegistry,
        id: &str,
        seq: u64,
        finish: bool,
        o: &mut Observer,
    ) -> VerifiedWordSpan {
        registry
            .checkpoint(
                &identity(),
                id,
                &request(id, seq, finish),
                Instant::now(),
                o,
            )
            .unwrap()
    }
    fn domain(temp: &tempfile::TempDir) -> LegacyMemory {
        LegacyMemory::open(
            LegacyMemoryContext::fixture(temp.path().join("memory.db"), "test".into()),
            [2; 32],
            1,
            None,
        )
        .unwrap()
    }
    fn count(temp: &tempfile::TempDir, text: &str) -> u32 {
        Connection::open(temp.path().join("memory.db"))
            .unwrap()
            .query_row(
                "SELECT COALESCE((SELECT typed_count FROM inputia_terms WHERE text=?1),0)",
                [text],
                |r| r.get(0),
            )
            .unwrap()
    }
    #[test]
    fn append_and_cross_checkpoint_backspace_replace_the_complete_revision_once() {
        let temp = tempfile::tempdir().unwrap();
        let mut memory = domain(&temp);
        let mut r = WordSpanRegistry::default();
        let mut o = Observer::new();
        let id = begin(&mut r, &mut o);
        append(&mut r, &id, 1, "hello hello ", &mut o);
        let first = check(&mut r, &id, 1, false, &mut o);
        memory.apply_word_span(&first).unwrap();
        assert_eq!(count(&temp, "hello"), 2);
        assert!(memory.apply_word_span(&first).unwrap().replayed);
        assert!(
            r.record(
                &identity(),
                &id,
                1,
                SpanEdit::Append {
                    text: "hello hello ".into()
                }
            )
            .unwrap()
            .replayed
        );
        r.record(&identity(), &id, 2, SpanEdit::TailBackspace { units: 6 })
            .unwrap();
        o.backspace(6);
        let second = check(&mut r, &id, 2, false, &mut o);
        memory.apply_word_span(&second).unwrap();
        assert_eq!(count(&temp, "hello"), 1);
        r.record(&identity(), &id, 3, SpanEdit::TailBackspace { units: 6 })
            .unwrap();
        o.backspace(6);
        let empty = check(&mut r, &id, 3, false, &mut o);
        assert!(empty.words().is_empty());
        memory.apply_word_span(&empty).unwrap();
        assert_eq!(count(&temp, "hello"), 0);
        append(&mut r, &id, 4, "world ", &mut o);
        let final_proof = check(&mut r, &id, 4, true, &mut o);
        memory.apply_word_span(&final_proof).unwrap();
        assert_eq!(count(&temp, "world"), 1);
    }
    #[test]
    fn checkpoint_replay_never_accepts_another_owner_target_or_epoch() {
        for finish in [false, true] {
            for mismatch in 0..4 {
                let mut registry = WordSpanRegistry::default();
                let mut observer = Observer::new();
                let id = begin(&mut registry, &mut observer);
                append(&mut registry, &id, 1, "verified ", &mut observer);
                let proof = check(&mut registry, &id, 1, finish, &mut observer);
                if finish {
                    assert!(registry.acknowledge_seal(&proof));
                }
                let mut other = identity();
                match mismatch {
                    0 => {
                        other.client_instance = "another-host".into();
                        other.target.host_instance = "another-host".into();
                    }
                    1 => {
                        other.target.target_id = "another-target".into();
                        other.target.field_id = Some("another-target".into());
                    }
                    2 => other.policy_epoch += 1,
                    _ => other.server_instance = "another-server".into(),
                }
                let rejected = registry.checkpoint(
                    &other,
                    &id,
                    &request(&id, 1, finish),
                    Instant::now(),
                    &mut observer,
                );
                assert!(rejected.is_err(), "finish={finish}, mismatch={mismatch}");
            }
        }
    }
    #[test]
    fn seal_is_durable_replayable_and_never_relearns_the_old_prefix() {
        let temp = tempfile::tempdir().unwrap();
        let mut memory = domain(&temp);
        let mut r = WordSpanRegistry::default();
        let mut o = Observer::new();
        let id = begin(&mut r, &mut o);
        append(&mut r, &id, 1, "stable ", &mut o);
        let active = check(&mut r, &id, 1, false, &mut o);
        memory.apply_word_span(&active).unwrap();
        let sealed = check(&mut r, &id, 1, true, &mut o);
        assert!(sealed.sealed());
        memory.apply_word_span(&sealed).unwrap();
        assert!(r.acknowledge_seal(&sealed));
        let retry = check(&mut r, &id, 1, true, &mut o);
        assert!(memory.apply_word_span(&retry).unwrap().replayed);
        assert_eq!(
            r.record(
                &identity(),
                &id,
                2,
                SpanEdit::Append {
                    text: "late".into()
                }
            )
            .unwrap_err()
            .reason,
            WordSpanReason::Sealed
        );
        assert!(r
            .expire_at(Instant::now() + Duration::from_secs(3))
            .is_empty());
        assert!(!r.has_pending_or_active());
        drop(memory);
        let mut memory = domain(&temp);
        assert_eq!(count(&temp, "stable"), 1);
        let fresh = begin(&mut r, &mut o);
        append(&mut r, &fresh, 1, "new ", &mut o);
        let proof = check(&mut r, &fresh, 1, true, &mut o);
        assert_eq!(proof.words()[0].text, "new");
        memory.apply_word_span(&proof).unwrap();
        assert_eq!(count(&temp, "stable"), 1);
        let mut mid = Observer::new();
        mid.append("oldprefix");
        assert_eq!(
            r.prepare(identity(), Instant::now(), &mut mid)
                .unwrap_err()
                .reason,
            WordSpanReason::BoundaryRequired
        );
    }
    #[test]
    fn expiry_and_restart_revoke_unsealed_weights_and_reject_late_proofs() {
        let temp = tempfile::tempdir().unwrap();
        let mut memory = domain(&temp);
        let mut r = WordSpanRegistry::default();
        let mut o = Observer::new();
        let id = begin(&mut r, &mut o);
        append(&mut r, &id, 1, "temporary ", &mut o);
        let proof = check(&mut r, &id, 1, false, &mut o);
        memory.apply_word_span(&proof).unwrap();
        let tokens = r.expire_at(Instant::now() + Duration::from_secs(3));
        assert_eq!(tokens.len(), 1);
        assert_eq!(r.pending_revocations(), tokens);
        memory.revoke_word_span(&tokens[0]).unwrap();
        assert_eq!(count(&temp, "temporary"), 0);
        assert!(memory.revoke_word_span(&tokens[0]).unwrap().replayed);
        assert!(r.acknowledge_revocation(&tokens[0]));
        let id = begin(&mut r, &mut o);
        append(&mut r, &id, 1, "crash ", &mut o);
        let first = check(&mut r, &id, 1, false, &mut o);
        memory.apply_word_span(&first).unwrap();
        append(&mut r, &id, 2, "late ", &mut o);
        let late = check(&mut r, &id, 2, false, &mut o);
        drop(memory);
        let mut memory = domain(&temp);
        assert_eq!(count(&temp, "crash"), 0);
        assert!(memory.apply_word_span(&late).is_err());
        assert_eq!(count(&temp, "late"), 0);
    }
    #[test]
    fn replacement_is_one_transaction_and_failed_receipt_does_not_erase_old_weights() {
        let temp = tempfile::tempdir().unwrap();
        let mut memory = domain(&temp);
        let mut r = WordSpanRegistry::default();
        let mut o = Observer::new();
        let id = begin(&mut r, &mut o);
        append(&mut r, &id, 1, "before ", &mut o);
        let first = check(&mut r, &id, 1, false, &mut o);
        memory.apply_word_span(&first).unwrap();
        r.record(&identity(), &id, 2, SpanEdit::TailBackspace { units: 7 })
            .unwrap();
        o.backspace(7);
        append(&mut r, &id, 3, "after ", &mut o);
        let after = check(&mut r, &id, 3, false, &mut o);
        let db = Connection::open(temp.path().join("memory.db")).unwrap();
        db.execute_batch("CREATE TRIGGER fail_span_receipt BEFORE INSERT ON memory_operations BEGIN SELECT RAISE(ABORT,'fixture'); END;").unwrap();
        assert!(memory.apply_word_span(&after).is_err());
        assert_eq!(count(&temp, "before"), 1);
        assert_eq!(count(&temp, "after"), 0);
        db.execute_batch("DROP TRIGGER fail_span_receipt;").unwrap();
        memory.apply_word_span(&after).unwrap();
        assert_eq!(count(&temp, "before"), 0);
        assert_eq!(count(&temp, "after"), 1);
    }
    #[test]
    fn changed_epoch_field_unsupported_readback_and_conflicting_replay_have_no_success() {
        for mode in 0..5 {
            let mut r = WordSpanRegistry::default();
            let mut o = Observer::new();
            let id = begin(&mut r, &mut o);
            append(&mut r, &id, 1, "hello ", &mut o);
            let mut owner = identity();
            match mode {
                0 => owner.policy_epoch += 1,
                1 => o.field = "different".into(),
                2 => o.unsupported = true,
                3 => o.text[0] = b'j' as u16,
                _ => owner.permission_epoch += 1,
            };
            let error = r
                .checkpoint(&owner, &id, &request(&id, 1, false), Instant::now(), &mut o)
                .err()
                .unwrap();
            assert!(error.revocation.is_some());
            assert_eq!(r.pending_revocations().len(), 1);
        }
        let mut r = WordSpanRegistry::default();
        let mut o = Observer::new();
        let id = begin(&mut r, &mut o);
        append(&mut r, &id, 1, "hello ", &mut o);
        assert_eq!(
            r.record(
                &identity(),
                &id,
                1,
                SpanEdit::Append {
                    text: "changed ".into()
                }
            )
            .unwrap_err()
            .reason,
            WordSpanReason::ConflictingReplay
        );
    }
    #[test]
    fn bounded_transcript_and_real_readback_control_renewal_and_sealing() {
        let mut r = WordSpanRegistry::default();
        let mut o = Observer::new();
        let start = Instant::now();
        let id = r
            .prepare_at(identity(), start, &mut o, || start, || Ok("bounded".into()))
            .unwrap()
            .span_id;
        r.record_at(
            &identity(),
            &id,
            1,
            SpanEdit::Append {
                text: "a".repeat(4000),
            },
            start,
        )
        .unwrap();
        o.append(&"a".repeat(4000));
        let midway = start + Duration::from_millis(1400);
        r.checkpoint_at(
            &identity(),
            &id,
            &request(&id, 1, false),
            midway,
            &mut o,
            || midway,
        )
        .unwrap();
        let later = start + Duration::from_millis(2000);
        r.record_at(
            &identity(),
            &id,
            2,
            SpanEdit::Append { text: "b ".into() },
            later,
        )
        .unwrap();
        o.append("b ");
        assert_eq!(
            r.checkpoint_at(
                &identity(),
                &id,
                &request(&id, 2, true),
                later,
                &mut o,
                || later
            )
            .unwrap()
            .words()[0]
                .text
                .len(),
            4001
        );
        let mut r = WordSpanRegistry::default();
        let mut o = Observer::new();
        let id = begin(&mut r, &mut o);
        assert_eq!(
            r.record(
                &identity(),
                &id,
                1,
                SpanEdit::Append {
                    text: "x".repeat(MAX_SPAN_UNITS + 1)
                }
            )
            .unwrap_err()
            .reason,
            WordSpanReason::BudgetExceeded
        );
        let mut r = WordSpanRegistry::default();
        let id = begin(&mut r, &mut o);
        append(&mut r, &id, 1, "unfinished", &mut o);
        assert_eq!(
            r.checkpoint(
                &identity(),
                &id,
                &request(&id, 1, true),
                Instant::now(),
                &mut o
            )
            .err()
            .unwrap()
            .reason,
            WordSpanReason::BoundaryRequired
        );
    }
    #[test]
    fn clearing_privacy_caches_drops_sealed_body_but_keeps_active_revocations() {
        let mut r = WordSpanRegistry::default();
        let mut o = Observer::new();
        let id = begin(&mut r, &mut o);
        append(&mut r, &id, 1, "private ", &mut o);
        let sealed = check(&mut r, &id, 1, true, &mut o);
        assert!(r.acknowledge_seal(&sealed));
        let active = begin(&mut r, &mut o);
        append(&mut r, &active, 1, "pending ", &mut o);
        check(&mut r, &active, 1, false, &mut o);
        let tokens = r.revoke_all();
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].span_id(), active);
        assert!(r.completed.is_empty());
        assert!(r.spans.is_empty());
    }

    #[test]
    fn seal_commit_windows_use_real_domain_receipts_and_preserve_no_unsealed_orphans() {
        for committed in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let mut memory = domain(&temp);
            let mut r = WordSpanRegistry::default();
            let mut o = Observer::new();
            let id = begin(&mut r, &mut o);
            append(&mut r, &id, 1, "durable ", &mut o);
            let active = check(&mut r, &id, 1, false, &mut o);
            memory.apply_word_span(&active).unwrap();
            let sealed = check(&mut r, &id, 1, true, &mut o);
            if committed {
                memory.apply_word_span(&sealed).unwrap();
            }
            // 模拟 finish 后域提交前断开，或域提交后 ACK/回复丢失。
            let pending = r.expire_at(Instant::now() + Duration::from_secs(3));
            assert_eq!(pending.len(), 1);
            memory.revoke_word_span(&pending[0]).unwrap();
            assert_eq!(count(&temp, "durable"), u32::from(committed));
            assert!(r.acknowledge_revocation(&pending[0]));
            assert!(!r.has_pending_or_active());
            assert!(memory.apply_word_span(&active).unwrap().replayed);
            assert_eq!(count(&temp, "durable"), u32::from(committed));
            if committed {
                assert!(memory.apply_word_span(&sealed).unwrap().replayed);
            } else {
                assert!(memory.apply_word_span(&sealed).is_err());
            }
            drop(memory);
            let _reopened = domain(&temp);
            assert_eq!(count(&temp, "durable"), u32::from(committed));
        }
    }

    #[test]
    fn sealed_events_are_still_removed_by_real_privacy_operations() {
        use crate::privacy_operation::{PrivacyRequest, PrivacyScope};
        for scope in [
            PrivacyScope::ForgetTerm {
                term: "private".into(),
            },
            PrivacyScope::ClearLearned {},
        ] {
            let temp = tempfile::tempdir().unwrap();
            let mut memory = domain(&temp);
            let mut r = WordSpanRegistry::default();
            let mut o = Observer::new();
            let id = begin(&mut r, &mut o);
            append(&mut r, &id, 1, "private ", &mut o);
            let proof = check(&mut r, &id, 1, true, &mut o);
            memory.apply_word_span(&proof).unwrap();
            assert_eq!(count(&temp, "private"), 1);
            let request = PrivacyRequest {
                operation_id: "forget".into(),
                expected_epoch: 1,
                scope,
            };
            memory
                .apply_privacy(&request, &request.digest(&[2; 32]).unwrap(), 2)
                .unwrap();
            assert_eq!(count(&temp, "private"), 0);
            assert!(memory.apply_word_span(&proof).is_err());
            let db = Connection::open(temp.path().join("memory.db")).unwrap();
            let events: u64 = db
                .query_row(
                    "SELECT COUNT(*) FROM inputia_events WHERE text IS NOT NULL",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(events, 0);
        }
    }
}
