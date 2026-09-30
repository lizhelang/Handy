//! 启动恢复事务的逐文件写前归属；仅传递摘要，不传递配置值。
use super::*;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct FileDigest {
    pub sha256: String,
    pub size: u64,
}
impl FileDigest {
    pub(super) fn of(bytes: &[u8]) -> Self {
        Self {
            sha256: raw_digest(bytes),
            size: bytes.len() as u64,
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransitionPhase {
    PrepareRequest,
    CommitDocument,
    ResolveRequest,
}
/// observer 所在的启动事务分配耐久序号；本意图不能被反序列化成写入许可。
#[derive(Clone, Debug, Serialize)]
pub struct TransitionIntent {
    pub domain: String,
    pub store_id: String,
    pub ledger_id: String,
    pub operation_id: String,
    pub request_digest: String,
    pub phase: TransitionPhase,
    pub file_name: String,
    pub before: FileDigest,
    pub after: FileDigest,
}
/// 同一 Files 锁内同步回调；失败不得替换源文件，不得重入设置或反向取得迁移锁。
pub type TransitionObserver<'a> = dyn FnMut(&TransitionIntent) -> Result<()> + 'a;

pub(super) struct TransitionOwner<'a> {
    pub store_id: &'a str,
    pub ledger_id: &'a str,
    pub operation_id: &'a str,
    pub request_digest: &'a str,
}
/// 从刚校验过的三文件取得，只有本次成功替换才能推进对应成员。
pub(super) struct TransitionBaseline {
    names: [&'static str; 3],
    hashes: [String; 3],
}
impl TransitionBaseline {
    pub(super) fn advance(&mut self, name: &str, bytes: &[u8]) -> Result<()> {
        let index = self
            .names
            .iter()
            .position(|n| *n == name)
            .ok_or(Error::RepairRequired)?;
        self.hashes[index] = raw_digest(bytes);
        Ok(())
    }
}
impl<S: DocumentSchema> DocumentStore<S> {
    pub(super) fn transition_baseline(
        &self,
        document: &Document,
        ledger_digest: &str,
    ) -> Result<TransitionBaseline> {
        let baseline = TransitionBaseline {
            names: [
                S::FILE_NAME,
                S::MARKER_NAME,
                S::PENDING_NAME.ok_or(Error::PendingProtocolRequired)?,
            ],
            hashes: [
                document
                    .source_digest
                    .clone()
                    .ok_or(Error::RepairRequired)?,
                document
                    .marker_digest
                    .clone()
                    .ok_or(Error::RepairRequired)?,
                ledger_digest.into(),
            ],
        };
        self.check_transition_baseline(&baseline)?;
        Ok(baseline)
    }
    pub(super) fn check_transition_baseline(
        &self,
        baseline: &TransitionBaseline,
    ) -> Result<Vec<Vec<u8>>> {
        let mut raw = Vec::with_capacity(3);
        for (name, expected) in baseline.names.iter().zip(&baseline.hashes) {
            let bytes = self
                .files
                .read(name, LIMIT, *name != S::FILE_NAME)?
                .ok_or(Error::RepairRequired)?;
            if raw_digest(&bytes) != *expected {
                return Err(Error::ExternalChanged);
            }
            raw.push(bytes);
        }
        Ok(raw)
    }
    pub(super) fn observe_transition(
        &self,
        owner: &TransitionOwner<'_>,
        phase: TransitionPhase,
        baseline: &TransitionBaseline,
        bytes: &[u8],
        observer: &mut TransitionObserver<'_>,
    ) -> Result<()> {
        let index = if phase == TransitionPhase::CommitDocument {
            0
        } else {
            2
        };
        // 三份字节都必须沿用已校验基线；不能在阶段之间重新采信外部变化。
        let before = self.check_transition_baseline(baseline)?;
        maintenance::ensure_normal_start(&self.home, self.uid).map_err(|_| Error::Maintenance)?;
        observer(&TransitionIntent {
            domain: S::DOMAIN.into(),
            store_id: owner.store_id.into(),
            ledger_id: owner.ledger_id.into(),
            operation_id: owner.operation_id.into(),
            request_digest: owner.request_digest.into(),
            phase,
            file_name: baseline.names[index].into(),
            before: FileDigest::of(&before[index]),
            after: FileDigest::of(bytes),
        })?;
        // 回调等待外层日志时，任一成员变化均撤销本次写许可。
        self.check_transition_baseline(baseline)?;
        maintenance::ensure_normal_start(&self.home, self.uid).map_err(|_| Error::Maintenance)
    }
}
