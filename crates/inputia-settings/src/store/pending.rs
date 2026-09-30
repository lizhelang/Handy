//! 配置请求的耐久单槽。正文可能含凭据，类型不实现 Debug，公开状态只包含身份。
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Binding {
    version: u32,
    ledger_id: String,
}
pub(super) fn valid_protocol<S: DocumentSchema>(version: u32, binding: Option<&Binding>) -> bool {
    match (version, binding) {
        (1, None) => true,
        (2, Some(value)) => {
            S::PENDING_NAME.is_some() && value.version == 1 && valid_uuid(&value.ledger_id)
        }
        _ => false,
    }
}

/// 三份固定文件在激活前的归属。observer 必须先耐久保存这些摘要。
#[derive(Clone, Debug, Serialize)]
pub struct LedgerActivationIntent {
    pub domain: String,
    pub store_id: String,
    pub ledger_id: String,
    pub activation_id: String,
    pub files: Vec<ActivationFile>,
}
#[derive(Clone, Debug, Serialize)]
pub struct ActivationFile {
    pub name: String,
    pub original_sha256: Option<String>,
    pub original_size: Option<u64>,
    pub target_sha256: String,
    pub target_size: u64,
}
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PendingStatus {
    Disabled,
    RequiresActivation,
    Idle,
    Active {
        operation_id: String,
    },
    Resolved {
        operation_id: String,
        outcome: String,
        commit_revision: Option<String>,
    },
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Floor {
    revision: String,
    values_digest: String,
}
impl Floor {
    fn of(snapshot: &Snapshot) -> Self {
        Self {
            revision: snapshot.revision.clone(),
            values_digest: snapshot.values_digest.clone(),
        }
    }
    fn check(&self, document: &Document) -> Result<()> {
        let prior = revision(&self.revision).map_err(|_| Error::RepairRequired)?;
        let current = revision(&document.header.revision)?;
        if !is_digest(&self.values_digest)
            || current < prior
            || (current == prior && self.values_digest != document.header.values_digest)
        {
            return Err(Error::RepairRequired);
        }
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "request",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum Operation {
    Patch(PatchRequest),
    ExternalImport(ImportRequest),
}
impl Operation {
    fn id(&self) -> &str {
        match self {
            Self::Patch(r) => &r.operation_id,
            Self::ExternalImport(r) => &r.operation_id,
        }
    }
    fn store_id(&self) -> &str {
        match self {
            Self::Patch(r) => &r.expected_store_id,
            Self::ExternalImport(r) => &r.expected_store_id,
        }
    }
    fn expected_revision(&self) -> &str {
        match self {
            Self::Patch(r) => &r.expected_revision,
            Self::ExternalImport(r) => &r.expected_revision,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "phase", rename_all = "snake_case", deny_unknown_fields)]
enum State {
    Bootstrap {
        floor: Floor,
    },
    Active {
        request: Operation,
        request_digest: String,
        floor: Floor,
    },
    Resolved {
        operation_id: String,
        request_digest: String,
        outcome: Outcome,
        floor: Floor,
    },
}
impl State {
    fn floor(&self) -> &Floor {
        match self {
            Self::Bootstrap { floor }
            | Self::Active { floor, .. }
            | Self::Resolved { floor, .. } => floor,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum Outcome {
    Saved {
        commit_revision: String,
    },
    Conflict,
    #[serde(rename = "outcome_expired")]
    Expired,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ledger {
    schema_version: u32,
    domain: String,
    store_id: String,
    ledger_id: String,
    state: State,
}
impl Ledger {
    fn bytes(&self) -> Result<Vec<u8>> {
        let bytes = canonical(&serde_json::to_value(self).map_err(|_| Error::InvalidDocument)?)?;
        if bytes.len() > LIMIT {
            return Err(Error::InvalidRequest);
        }
        Ok(bytes)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stage {
    Prepare,
    Document,
    Resolve,
    ActivationLedger,
    ActivationDocument,
    ActivationMarker,
}
pub(super) struct Decision {
    pub(super) bytes: Option<Vec<u8>>,
    pub(super) result: ApplyResult,
}
impl Decision {
    pub(super) fn observed(result: ApplyResult) -> Self {
        Self {
            bytes: None,
            result,
        }
    }
    fn current(&self) -> &Snapshot {
        match &self.result {
            ApplyResult::Saved { current, .. }
            | ApplyResult::Conflict { current }
            | ApplyResult::OutcomeExpired { current } => current,
        }
    }
}
impl<S: DocumentSchema> DocumentStore<S> {
    fn read_ledger(&self, document: &Document) -> Result<Option<Ledger>> {
        let Some(name) = S::PENDING_NAME else {
            return Ok(None);
        };
        let raw = self.files.read(name, LIMIT, true)?;
        let Some(binding) = &document.header.pending_protocol else {
            return if raw.is_none() {
                Ok(None)
            } else {
                Err(Error::RepairRequired)
            };
        };
        let ledger: Ledger =
            serde_json::from_value(strict_json(&raw.ok_or(Error::RepairRequired)?)?)
                .map_err(|_| Error::RepairRequired)?;
        if ledger.schema_version != 1
            || ledger.domain != S::DOMAIN
            || ledger.store_id != document.header.store_id
            || ledger.ledger_id != binding.ledger_id
        {
            return Err(Error::RepairRequired);
        }
        ledger.state.floor().check(document)?;
        match &ledger.state {
            State::Bootstrap { .. } => {}
            State::Active {
                request,
                request_digest,
                floor,
            } => {
                let actual = self
                    .validate_operation(request)
                    .map_err(|_| Error::RepairRequired)?;
                if &actual != request_digest
                    || request.store_id() != ledger.store_id
                    || revision(request.expected_revision())? > revision(&floor.revision)?
                {
                    return Err(Error::RepairRequired);
                }
            }
            State::Resolved {
                operation_id,
                request_digest,
                outcome,
                floor,
            } => {
                let base = operation_base(operation_id, &ledger.store_id)
                    .map_err(|_| Error::RepairRequired)?;
                if !is_digest(request_digest) || base > revision(&floor.revision)? {
                    return Err(Error::RepairRequired);
                }
                if let Outcome::Saved { commit_revision } = outcome {
                    if base.checked_add(1) != Some(revision(commit_revision)?)
                        || revision(commit_revision)? > revision(&floor.revision)?
                    {
                        return Err(Error::RepairRequired);
                    }
                    // 末态不独立充当提交证据；保存回执已经过期时也不能伪报 Saved。
                    let receipt = document
                        .header
                        .receipts
                        .iter()
                        .find(|r| &r.operation_id == operation_id)
                        .ok_or(Error::RepairRequired)?;
                    if &receipt.request_digest != request_digest
                        || &receipt.revision != commit_revision
                    {
                        return Err(Error::RepairRequired);
                    }
                }
            }
        }
        Ok(Some(ledger))
    }
    pub(super) fn check_pending_document(&self, document: &Document) -> Result<()> {
        self.read_ledger(document).map(|_| ())
    }
    fn validate_operation(&self, operation: &Operation) -> Result<String> {
        match operation {
            Operation::Patch(request) => self.validate_patch(request),
            Operation::ExternalImport(request) => self.validate_import(request),
        }
    }
    fn admission(
        &self,
        document: &Document,
        operation: &Operation,
        digest: &str,
    ) -> Result<Option<Ledger>> {
        if S::PENDING_NAME.is_none() {
            return Ok(None);
        }
        let ledger = self
            .read_ledger(document)?
            .ok_or(Error::PendingProtocolRequired)?;
        if let State::Active {
            request,
            request_digest,
            ..
        } = &ledger.state
        {
            if request.id() != operation.id() {
                return Err(Error::PendingOperation);
            }
            if request_digest != digest {
                return Err(Error::OperationMismatch);
            }
        }
        if let State::Resolved {
            operation_id,
            request_digest,
            ..
        } = &ledger.state
        {
            if operation_id == operation.id() && request_digest != digest {
                return Err(Error::OperationMismatch);
            }
        }
        Ok(Some(ledger))
    }
    pub(super) fn apply_pending(
        &self,
        request: &PatchRequest,
        floor: Option<&Snapshot>,
        hook: &mut impl FnMut(Stage, Boundary) -> Result<()>,
    ) -> Result<ApplyResult> {
        let digest = self.validate_patch(request)?;
        let document = self.load_guarded(false, floor, S::PENDING_NAME.is_none(), None)?;
        let operation = Operation::Patch(request.clone());
        let ledger = self.admission(&document, &operation, &digest)?;
        let decision = self.decide_patch(request, &digest, &document)?;
        self.execute_pending(&document, operation, digest, ledger, decision, hook)
    }
    pub(super) fn import_pending(
        &self,
        request: &ImportRequest,
        hook: &mut impl FnMut(Stage, Boundary) -> Result<()>,
    ) -> Result<ApplyResult> {
        let digest = self.validate_import(request)?;
        let document = self.load_internal(true)?;
        let operation = Operation::ExternalImport(request.clone());
        let ledger = self.admission(&document, &operation, &digest)?;
        let decision = self.decide_import(request, &digest, &document)?;
        self.execute_pending(&document, operation, digest, ledger, decision, hook)
    }
    fn execute_pending(
        &self,
        document: &Document,
        operation: Operation,
        request_digest: String,
        mut ledger: Option<Ledger>,
        decision: Decision,
        hook: &mut impl FnMut(Stage, Boundary) -> Result<()>,
    ) -> Result<ApplyResult> {
        // 错 store 的请求只返回冲突，不能把另一领域的操作写成当前领域日志。
        if operation.store_id() != document.header.store_id
            || revision(operation.expected_revision())? > revision(&document.header.revision)?
        {
            return Ok(decision.result);
        }
        if let Some(ledger) = &mut ledger {
            // 重试 Active 也补同步，不能把上轮 rename 后的不确定日志当已耐久。
            if !matches!(&ledger.state, State::Active { .. }) {
                ledger.state = State::Active {
                    request: operation.clone(),
                    request_digest: request_digest.clone(),
                    floor: Floor::of(&document.snapshot()),
                };
            }
            let bytes = ledger.bytes()?;
            maintenance::ensure_normal_start(&self.home, self.uid)
                .map_err(|_| Error::Maintenance)?;
            self.files.replace(
                S::PENDING_NAME.ok_or(Error::RepairRequired)?,
                &bytes,
                &mut |boundary| hook(Stage::Prepare, boundary),
            )?;
        }
        maintenance::ensure_normal_start(&self.home, self.uid).map_err(|_| Error::Maintenance)?;
        if let Some(bytes) = &decision.bytes {
            self.files.replace(S::FILE_NAME, bytes, &mut |boundary| {
                hook(Stage::Document, boundary)
            })?;
        } else if matches!(&decision.result, ApplyResult::Saved { .. }) {
            self.files
                .confirm_durable(S::FILE_NAME, S::MARKER_NAME, &mut |boundary| {
                    hook(Stage::Document, boundary)
                })?;
        }
        // 过期回执不能确认是否曾执行，保留原请求并阻止新操作。
        if matches!(&decision.result, ApplyResult::OutcomeExpired { .. }) {
            return Ok(decision.result);
        }
        if let Some(ledger) = &mut ledger {
            ledger.state = State::Resolved {
                operation_id: operation.id().into(),
                request_digest,
                outcome: match &decision.result {
                    ApplyResult::Saved {
                        commit_revision, ..
                    } => Outcome::Saved {
                        commit_revision: commit_revision.clone(),
                    },
                    ApplyResult::Conflict { .. } => Outcome::Conflict,
                    ApplyResult::OutcomeExpired { .. } => Outcome::Expired,
                },
                floor: Floor::of(decision.current()),
            };
            // 配置可能已提交，末态写入失败一律仍为结果不确定。
            self.files
                .replace(
                    S::PENDING_NAME.ok_or(Error::RepairRequired)?,
                    &ledger.bytes()?,
                    &mut |boundary| hook(Stage::Resolve, boundary),
                )
                .map_err(|_| Error::CommitUncertain)?;
        }
        Ok(decision.result)
    }
    /// 只报告安全元数据；不返回 patch、外部文件正文或读取凭据。
    pub fn pending_status(&self) -> Result<PendingStatus> {
        if S::PENDING_NAME.is_none() {
            return Ok(PendingStatus::Disabled);
        }
        let document = self.load_guarded(true, None, false, None)?;
        let Some(ledger) = self.read_ledger(&document)? else {
            return Ok(PendingStatus::RequiresActivation);
        };
        if !matches!(&ledger.state, State::Active { .. })
            && digest(&Value::Object(document.values.clone()))? != document.header.values_digest
        {
            return Err(Error::ExternalEdit);
        }
        Ok(match ledger.state {
            State::Bootstrap { .. } => PendingStatus::Idle,
            State::Active { request, .. } => PendingStatus::Active {
                operation_id: request.id().into(),
            },
            State::Resolved {
                operation_id,
                outcome,
                ..
            } => PendingStatus::Resolved {
                operation_id,
                outcome: match &outcome {
                    Outcome::Saved { .. } => "saved",
                    Outcome::Conflict => "conflict",
                    Outcome::Expired => "outcome_expired",
                }
                .into(),
                commit_revision: match outcome {
                    Outcome::Saved { commit_revision } => Some(commit_revision),
                    _ => None,
                },
            },
        })
    }
    /// 仅重放原配置请求；调用方必须在业务启动前持有已登记的启动恢复事务。
    /// 这不执行或证明任何设备、文件删除、文本输出等外部副作用。
    pub fn reconcile_pending(&self) -> Result<Option<ApplyResult>> {
        let document = self.load_guarded(true, None, false, None)?;
        let ledger = self
            .read_ledger(&document)?
            .ok_or(Error::PendingProtocolRequired)?;
        match ledger.state {
            State::Active {
                request: Operation::Patch(request),
                ..
            } => self.apply(&request).map(Some),
            State::Active {
                request: Operation::ExternalImport(request),
                ..
            } => self.import_external(&request).map(Some),
            State::Resolved { outcome, .. } => {
                if digest(&Value::Object(document.values.clone()))? != document.header.values_digest
                {
                    return Err(Error::ExternalEdit);
                }
                // 末态 rename 可能成功但目录同步未知；确认原正文与日志后才报告恢复结果。
                self.files
                    .confirm_durable(S::FILE_NAME, S::MARKER_NAME, &mut |_| Ok(()))?;
                self.files.confirm_durable(
                    S::PENDING_NAME.ok_or(Error::RepairRequired)?,
                    S::MARKER_NAME,
                    &mut |_| Ok(()),
                )?;
                let current = document.snapshot();
                Ok(Some(match outcome {
                    Outcome::Saved { commit_revision } => ApplyResult::Saved {
                        commit_revision,
                        replayed: true,
                        current,
                    },
                    Outcome::Conflict => ApplyResult::Conflict { current },
                    Outcome::Expired => ApplyResult::OutcomeExpired { current },
                }))
            }
            State::Bootstrap { .. } => Ok(None),
        }
    }
    /// 仅显式激活完整旧协议；不修补部分三文件状态，不改变值、版本或原回执。
    pub fn activate_pending_protocol(
        &self,
        observer: &mut dyn FnMut(&LedgerActivationIntent) -> Result<()>,
    ) -> Result<Snapshot> {
        self.activate_with_hook(observer, &mut |_, _| Ok(()))
    }
    fn activate_with_hook(
        &self,
        observer: &mut dyn FnMut(&LedgerActivationIntent) -> Result<()>,
        hook: &mut impl FnMut(Stage, Boundary) -> Result<()>,
    ) -> Result<Snapshot> {
        let name = S::PENDING_NAME.ok_or(Error::InvalidRequest)?;
        // 不调用隐式初始化：完整旧协议及三份原字节必须先确定。
        let names = [name, S::FILE_NAME, S::MARKER_NAME];
        let originals = names
            .iter()
            .map(|n| self.files.read(n, LIMIT, *n != S::FILE_NAME))
            .collect::<Result<Vec<_>>>()?;
        if originals[1].is_none() || originals[2].is_none() {
            return Err(Error::RepairRequired);
        }
        let mut document = self.load_guarded(false, None, false, None)?;
        if document.header.pending_protocol.is_some() {
            self.files
                .confirm_durable(S::FILE_NAME, S::MARKER_NAME, &mut |b| {
                    hook(Stage::ActivationDocument, b)
                })?;
            self.files.confirm_durable(name, S::MARKER_NAME, &mut |b| {
                hook(Stage::ActivationLedger, b)
            })?;
            return Ok(document.snapshot());
        }
        let binding = Binding {
            version: 1,
            ledger_id: uuid::Uuid::new_v4().to_string(),
        };
        document.header.schema_version = 2;
        document.header.pending_protocol = Some(binding.clone());
        let ledger = Ledger {
            schema_version: 1,
            domain: S::DOMAIN.into(),
            store_id: document.header.store_id.clone(),
            ledger_id: binding.ledger_id.clone(),
            state: State::Bootstrap {
                floor: Floor::of(&document.snapshot()),
            },
        };
        let marker = Marker {
            schema_version: 2,
            domain: S::DOMAIN.into(),
            store_id: document.header.store_id.clone(),
            pending_protocol: Some(binding.clone()),
        };
        let targets = [
            ledger.bytes()?,
            document.bytes()?,
            serde_json::to_vec(&marker).map_err(|_| Error::InvalidDocument)?,
        ];
        let intent = LedgerActivationIntent {
            domain: S::DOMAIN.into(),
            store_id: document.header.store_id.clone(),
            ledger_id: binding.ledger_id,
            activation_id: uuid::Uuid::new_v4().to_string(),
            files: names
                .iter()
                .zip(&originals)
                .zip(&targets)
                .map(|((name, original), target)| ActivationFile {
                    name: (*name).into(),
                    original_sha256: original.as_ref().map(|bytes| raw_digest(bytes)),
                    original_size: original.as_ref().map(|bytes| bytes.len() as u64),
                    target_sha256: raw_digest(target),
                    target_size: target.len() as u64,
                })
                .collect(),
        };
        observer(&intent)?;
        for (name, original) in names.iter().zip(&originals) {
            if self.files.read(name, LIMIT, *name != S::FILE_NAME)? != *original {
                return Err(Error::ExternalChanged);
            }
        }
        maintenance::ensure_normal_start(&self.home, self.uid).map_err(|_| Error::Maintenance)?;
        for ((name, bytes), stage) in names.iter().zip(&targets).zip([
            Stage::ActivationLedger,
            Stage::ActivationDocument,
            Stage::ActivationMarker,
        ]) {
            self.files
                .replace(name, bytes, &mut |boundary| hook(stage, boundary))?;
        }
        Ok(document.snapshot())
    }
}

#[cfg(test)]
mod tests;
