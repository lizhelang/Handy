//! 可撤销的术语贡献账本。调用方必须用 IntegrationStore 的唯一连接安装表，
//! 并在索引/游标/策略使用的同一个事务调用写 API；任何 Err 都必须回滚整个事务。
//! 此模块不打开数据库、不写日志、不接触历史全文，也不在输入法按键路径运行。

use std::fmt;

use inputia_core::integration::{
    events::{Identifier, SourceRecord},
    privacy::{PrivacyContext, PrivacyPolicy},
    terms::{
        build_hotwords, validate_term, HotwordBudget, HotwordError, TermEvidence, TermRejection,
    },
};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use sha2::{Digest, Sha256};

/// key 由服务端从私有配置提供随机密钥；备份恢复必须保留同一密钥和最新撤销账本。
/// 不实现 Debug，避免将密钥输出到诊断日志。
pub struct LearningLedger {
    key: Vec<u8>,
}

/// 一个已确认的最小词条证据；普通打字全文、原始 ASR、剪贴板全文不能填入此接口。
pub struct ContributionInput {
    pub contribution_id: Identifier,
    pub source: SourceRecord,
    pub source_revision: u64,
    pub policy_epoch: u64,
    pub term: String,
    pub evidence: TermEvidence,
    /// 只可由新的、明确的“加入词库”操作设置；重放或自动同步不可设置。
    pub explicit_relearn: bool,
}

/// 主控制中心逐词确认；重试必须保留完整请求，不包含可伪造的源信任或策略版本。
#[derive(Clone)]
pub struct HistoryTermConfirmation {
    pub operation_id: Identifier,
    pub item_id: String,
    pub expected_revision: u64,
    pub term: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplyContribution {
    Applied,
    Replay,
    /// 相同源版本、相同词只贡献一次，即使调用方换了事件 ID。
    AlreadyContributed,
}

#[derive(Debug)]
pub enum LearningError {
    Sqlite(rusqlite::Error),
    InvalidKey,
    KeyMismatch,
    InvalidVersion,
    StaleEpoch,
    PrivacyDenied,
    InvalidTerm(TermRejection),
    ReplayConflict,
    SourceDeleted,
    StaleRevision,
    Forgotten,
    InvalidRelearn,
    HotwordBudget(HotwordError),
}

impl fmt::Display for LearningError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 错误只携带类别，不回显词、来源正文或密钥。
        formatter.write_str(match self {
            Self::Sqlite(_) => "learning database operation failed",
            Self::InvalidKey => "learning key must contain at least 32 bytes",
            Self::KeyMismatch => "learning key does not match ledger",
            Self::InvalidVersion => "learning version is invalid",
            Self::StaleEpoch => "learning policy epoch is stale",
            Self::PrivacyDenied => "learning privacy policy denied access",
            Self::InvalidTerm(_) => "learning term is ineligible",
            Self::ReplayConflict => "learning contribution identity conflicts",
            Self::SourceDeleted => "learning source has been deleted",
            Self::StaleRevision => "learning source revision is stale",
            Self::Forgotten => "learning term has been forgotten",
            Self::InvalidRelearn => "relearn requires an explicit user term",
            Self::HotwordBudget(_) => "explicit terms exceed or violate model budget",
        })
    }
}

impl std::error::Error for LearningError {}
impl From<rusqlite::Error> for LearningError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}
pub type Result<T> = std::result::Result<T, LearningError>;

impl LearningLedger {
    pub fn new(key: &[u8]) -> Result<Self> {
        if key.len() < 32 {
            return Err(LearningError::InvalidKey);
        }
        Ok(Self { key: key.to_vec() })
    }

    /// 初始化表，并校验密钥身份。应在 IntegrationStore schema 初始化事务内调用。
    /// 首次 epoch 为 0；调用 advance_epoch 与统一服务当前策略对齐。
    pub fn install(&self, connection: &Connection) -> Result<()> {
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS learning_meta (
                singleton INTEGER PRIMARY KEY CHECK(singleton=1),
                epoch INTEGER NOT NULL CHECK(epoch>=0), key_check BLOB NOT NULL
             );
             CREATE TABLE IF NOT EXISTS learning_sources (
                store_id TEXT NOT NULL, record_id TEXT NOT NULL,
                revision INTEGER NOT NULL CHECK(revision>0),
                deleted INTEGER NOT NULL CHECK(deleted IN (0,1)),
                PRIMARY KEY(store_id,record_id)
             );
             CREATE TABLE IF NOT EXISTS learning_receipts (
                contribution_id TEXT PRIMARY KEY, digest BLOB NOT NULL
             );
             CREATE TABLE IF NOT EXISTS learning_forget_receipts (
                operation_id TEXT PRIMARY KEY, digest BLOB NOT NULL,
                result_epoch INTEGER NOT NULL CHECK(result_epoch>0)
             );
             CREATE TABLE IF NOT EXISTS learning_confirmation_receipts (
                operation_id TEXT PRIMARY KEY, digest BLOB NOT NULL
             );
             CREATE TABLE IF NOT EXISTS learning_contributions (
                contribution_id TEXT PRIMARY KEY,
                store_id TEXT NOT NULL, record_id TEXT NOT NULL,
                revision INTEGER NOT NULL, term_id BLOB NOT NULL, term TEXT NOT NULL,
                evidence INTEGER NOT NULL CHECK(evidence BETWEEN 0 AND 2),
                uses INTEGER NOT NULL, sessions INTEGER NOT NULL,
                UNIQUE(store_id,record_id,revision,term_id)
             );
             CREATE INDEX IF NOT EXISTS learning_term_lookup
                ON learning_contributions(term_id);
             CREATE TABLE IF NOT EXISTS learning_forgotten (
                term_id BLOB PRIMARY KEY, epoch INTEGER NOT NULL
             );",
        )?;
        let check = self.keyed_identity(b"key-check-v1", b"");
        connection.execute(
            "INSERT OR IGNORE INTO learning_meta(singleton,epoch,key_check) VALUES(1,0,?1)",
            params![check],
        )?;
        self.check_key(connection)
    }

    /// 当前撤销屏障；消费者快照应携带此版本并遵守 2 秒租约。
    pub fn epoch(&self, connection: &Connection) -> Result<u64> {
        self.check_key(connection)?;
        Ok(connection.query_row(
            "SELECT epoch FROM learning_meta WHERE singleton=1",
            [],
            |row| row.get::<_, i64>(0),
        )? as u64)
    }

    /// 仅由统一策略所有者调用；不允许倒退。暂停学习也应更新同一策略事务。
    pub fn advance_epoch(&self, tx: &Transaction<'_>, new_epoch: u64) -> Result<()> {
        let epoch = version(new_epoch, true)?;
        if new_epoch < self.epoch(tx)? {
            return Err(LearningError::StaleEpoch);
        }
        tx.execute(
            "UPDATE learning_meta SET epoch=?1 WHERE singleton=1",
            [epoch],
        )?;
        Ok(())
    }

    /// 写入一条最小贡献；摘要覆盖身份、版本、规范词、证据及重新学习意图。
    pub fn apply_contribution(
        &self,
        tx: &Transaction<'_>,
        input: &ContributionInput,
        policy: &PrivacyPolicy,
        context: PrivacyContext,
    ) -> Result<ApplyContribution> {
        self.require_epoch(tx, policy.epoch, input.policy_epoch)?;
        if !policy.decide(input.policy_epoch, context).learn {
            return Err(LearningError::PrivacyDenied);
        }
        let revision = version(input.source_revision, false)?;
        if input.explicit_relearn && input.evidence != TermEvidence::ExplicitUserTerm {
            return Err(LearningError::InvalidRelearn);
        }
        let term =
            validate_term(&input.term, input.evidence).map_err(LearningError::InvalidTerm)?;
        let term_id = self.keyed_identity(b"term-v1", term.as_bytes());
        let (evidence, uses, sessions) = evidence_parts(input.evidence);
        let digest = self.contribution_digest(input, &term, evidence, uses, sessions);
        let previous: Option<Vec<u8>> = tx
            .query_row(
                "SELECT digest FROM learning_receipts WHERE contribution_id=?1",
                [input.contribution_id.as_str()],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(previous) = previous {
            return if previous == digest {
                Ok(ApplyContribution::Replay)
            } else {
                Err(LearningError::ReplayConflict)
            };
        }
        let source = self.source_state(tx, &input.source)?;
        if let Some((last_revision, deleted)) = source {
            if deleted {
                return Err(LearningError::SourceDeleted);
            }
            if revision < last_revision {
                return Err(LearningError::StaleRevision);
            }
        }
        let forgotten: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM learning_forgotten WHERE term_id=?1)",
            params![term_id],
            |row| row.get(0),
        )?;
        if forgotten && !input.explicit_relearn {
            return Err(LearningError::Forgotten);
        }
        self.revise_source(tx, &input.source, input.source_revision)?;
        if forgotten {
            tx.execute(
                "DELETE FROM learning_forgotten WHERE term_id=?1",
                params![term_id],
            )?;
        }
        tx.execute(
            "INSERT INTO learning_receipts(contribution_id,digest) VALUES(?1,?2)",
            params![input.contribution_id.as_str(), digest],
        )?;
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO learning_contributions
                (contribution_id,store_id,record_id,revision,term_id,term,evidence,uses,sessions)
                VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                input.contribution_id.as_str(),
                input.source.store_id.as_str(),
                input.source.record_id.as_str(),
                revision,
                term_id,
                term,
                evidence,
                uses,
                sessions
            ],
        )?;
        Ok(if inserted == 1 {
            ApplyContribution::Applied
        } else {
            ApplyContribution::AlreadyContributed
        })
    }

    /// 每次源正文修订均须调用，即使修订后没有可学习词；撤销旧版本贡献。
    /// 收藏等不改变正文的版本升级，应由调用方明确重新关联合法词条。
    pub fn revise_source(
        &self,
        tx: &Transaction<'_>,
        source: &SourceRecord,
        revision: u64,
    ) -> Result<()> {
        self.check_key(tx)?;
        let revision = version(revision, false)?;
        if let Some((previous, deleted)) = self.source_state(tx, source)? {
            if deleted {
                return Err(LearningError::SourceDeleted);
            }
            if revision < previous {
                return Err(LearningError::StaleRevision);
            }
        }
        tx.execute(
            "DELETE FROM learning_contributions WHERE store_id=?1 AND record_id=?2 AND revision<?3",
            params![
                source.store_id.as_str(),
                source.record_id.as_str(),
                revision
            ],
        )?;
        tx.execute(
            "INSERT INTO learning_sources(store_id,record_id,revision,deleted) VALUES(?1,?2,?3,0)
             ON CONFLICT(store_id,record_id) DO UPDATE SET revision=excluded.revision",
            params![
                source.store_id.as_str(),
                source.record_id.as_str(),
                revision
            ],
        )?;
        Ok(())
    }

    /// 删除只撤销该来源，永久保留不含正文的来源身份屏障；来源 ID 不得复用。
    pub fn delete_source(
        &self,
        tx: &Transaction<'_>,
        source: &SourceRecord,
        revision: u64,
    ) -> Result<usize> {
        self.check_key(tx)?;
        let revision = version(revision, false)?;
        if let Some((previous, _)) = self.source_state(tx, source)? {
            if revision < previous {
                return Err(LearningError::StaleRevision);
            }
        }
        let count = tx.execute(
            "DELETE FROM learning_contributions WHERE store_id=?1 AND record_id=?2",
            params![source.store_id.as_str(), source.record_id.as_str()],
        )?;
        tx.execute(
            "INSERT INTO learning_sources(store_id,record_id,revision,deleted) VALUES(?1,?2,?3,1)
             ON CONFLICT(store_id,record_id) DO UPDATE SET revision=excluded.revision,deleted=1",
            params![
                source.store_id.as_str(),
                source.record_id.as_str(),
                revision
            ],
        )?;
        Ok(count)
    }

    /// 忘记后仅新的显式加入可解除词屏障；同时前移 epoch，旧事件即使稍后解禁也无效。
    pub fn forget_term(&self, tx: &Transaction<'_>, term: &str, new_epoch: u64) -> Result<usize> {
        let term = validate_term(term, TermEvidence::ExplicitUserTerm)
            .map_err(LearningError::InvalidTerm)?;
        self.require_new_epoch(tx, new_epoch)?;
        let term_id = self.keyed_identity(b"term-v1", term.as_bytes());
        let count = tx.execute(
            "DELETE FROM learning_contributions WHERE term_id=?1",
            params![term_id],
        )?;
        tx.execute(
            "INSERT INTO learning_forgotten(term_id,epoch) VALUES(?1,?2)
             ON CONFLICT(term_id) DO UPDATE SET epoch=excluded.epoch",
            params![term_id, new_epoch as i64],
        )?;
        self.advance_epoch(tx, new_epoch)?;
        Ok(count)
    }

    /// 清空共享学习并前移屏障；保留去重摘要、来源删除和遗忘标记，避免旧导入复活。
    pub fn clear(&self, tx: &Transaction<'_>, new_epoch: u64) -> Result<usize> {
        self.require_new_epoch(tx, new_epoch)?;
        let count = tx.execute("DELETE FROM learning_contributions", [])?;
        self.advance_epoch(tx, new_epoch)?;
        Ok(count)
    }

    /// 本地模型提示专用。显式词采用独立预算；暂停学习时已存在的贡献仍可读。
    /// 不可将结果自动用于远程上下文或模糊强制替换。
    pub fn hotwords(
        &self,
        connection: &Connection,
        policy: &PrivacyPolicy,
        context: PrivacyContext,
        event_epoch: u64,
        explicit: &[String],
        budget: HotwordBudget,
    ) -> Result<Vec<String>> {
        self.require_epoch(connection, policy.epoch, event_epoch)?;
        if !policy.decide(event_epoch, context).personalized_read {
            return Err(LearningError::PrivacyDenied);
        }
        let mut statement = connection.prepare(
            "SELECT term,evidence,uses,sessions FROM learning_contributions
             ORDER BY evidence,term,store_id,record_id,contribution_id",
        )?;
        let mut rows = statement.query([])?;
        let mut explicit = explicit.to_vec();
        let mut candidates = Vec::new();
        while let Some(row) = rows.next()? {
            let term: String = row.get(0)?;
            let evidence: i64 = row.get(1)?;
            if evidence == 0 {
                explicit.push(term);
            } else {
                let evidence = if evidence == 1 {
                    TermEvidence::ConfirmedCorrection
                } else {
                    TermEvidence::SelectedTypedTerm {
                        uses: row.get(2)?,
                        sessions: row.get(3)?,
                    }
                };
                candidates.push((term, evidence));
            }
        }
        // 外部旧配置也不能让已忘记词绕过撤销账本；新显式加入必须经过写入 API。
        let mut filtered = Vec::new();
        for term in explicit {
            let normalized = term.split_whitespace().collect::<Vec<_>>().join(" ");
            let id = self.keyed_identity(b"term-v1", normalized.as_bytes());
            let forgotten: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM learning_forgotten WHERE term_id=?1)",
                params![id],
                |row| row.get(0),
            )?;
            if !forgotten {
                filtered.push(term);
            }
        }
        build_hotwords(&filtered, &candidates, budget).map_err(LearningError::HotwordBudget)
    }

    fn require_epoch(
        &self,
        connection: &Connection,
        policy_epoch: u64,
        event_epoch: u64,
    ) -> Result<()> {
        if policy_epoch != event_epoch || self.epoch(connection)? != policy_epoch {
            return Err(LearningError::StaleEpoch);
        }
        Ok(())
    }

    fn require_new_epoch(&self, tx: &Transaction<'_>, new_epoch: u64) -> Result<()> {
        version(new_epoch, true)?;
        if new_epoch <= self.epoch(tx)? {
            return Err(LearningError::StaleEpoch);
        }
        Ok(())
    }

    pub(crate) fn check_key(&self, connection: &Connection) -> Result<()> {
        let stored: Vec<u8> = connection.query_row(
            "SELECT key_check FROM learning_meta WHERE singleton=1",
            [],
            |row| row.get(0),
        )?;
        if stored != self.keyed_identity(b"key-check-v1", b"") {
            return Err(LearningError::KeyMismatch);
        }
        Ok(())
    }

    fn source_state(
        &self,
        connection: &Connection,
        source: &SourceRecord,
    ) -> Result<Option<(i64, bool)>> {
        Ok(connection
            .query_row(
                "SELECT revision,deleted FROM learning_sources WHERE store_id=?1 AND record_id=?2",
                params![source.store_id.as_str(), source.record_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }

    // 本地稳定词身份而非 HMAC、认证凭据或对外可验证 MAC。
    // 固定域和长度编码避免串接歧义；32+ 字节随机私钥防止对删除词的字典枚举。
    pub(crate) fn forget_digest(&self, term: &str, expected_epoch: u64) -> Vec<u8> {
        let mut payload = expected_epoch.to_be_bytes().to_vec();
        payload.extend_from_slice(term.as_bytes());
        self.keyed_identity(b"forget-receipt-v1", &payload)
    }

    pub(crate) fn confirmation_digest(
        &self,
        request: &HistoryTermConfirmation,
        term: &str,
    ) -> Vec<u8> {
        let mut payload = Sha256::new();
        for field in [request.operation_id.as_str(), &request.item_id, term] {
            hash_field(&mut payload, field.as_bytes());
        }
        payload.update(request.expected_revision.to_be_bytes());
        self.keyed_identity(b"history-confirmation-receipt-v1", &payload.finalize())
    }

    fn keyed_identity(&self, domain: &[u8], text: &[u8]) -> Vec<u8> {
        let mut hash = Sha256::new();
        hash.update(b"inputia-learning-private-identity-v1\0");
        hash_field(&mut hash, domain);
        hash_field(&mut hash, &self.key);
        hash_field(&mut hash, text);
        hash.finalize().to_vec()
    }

    fn contribution_digest(
        &self,
        input: &ContributionInput,
        term: &str,
        evidence: i64,
        uses: u32,
        sessions: u32,
    ) -> Vec<u8> {
        let mut payload = Sha256::new();
        payload.update(b"inputia-learning-contribution-v1\0");
        for field in [
            input.contribution_id.as_str(),
            input.source.store_id.as_str(),
            input.source.record_id.as_str(),
            term,
        ] {
            hash_field(&mut payload, field.as_bytes());
        }
        payload.update(input.source_revision.to_be_bytes());
        payload.update(input.policy_epoch.to_be_bytes());
        payload.update(evidence.to_be_bytes());
        payload.update(uses.to_be_bytes());
        payload.update(sessions.to_be_bytes());
        payload.update([u8::from(input.explicit_relearn)]);
        // receipt 不保存公开的词散列，不能用短词字典枚举已撤销贡献。
        self.keyed_identity(b"receipt-v1", &payload.finalize())
    }
}

fn hash_field(hash: &mut Sha256, field: &[u8]) {
    hash.update((field.len() as u64).to_be_bytes());
    hash.update(field);
}

fn version(value: u64, allow_zero: bool) -> Result<i64> {
    if (!allow_zero && value == 0) || value > i64::MAX as u64 {
        return Err(LearningError::InvalidVersion);
    }
    Ok(value as i64)
}

fn evidence_parts(evidence: TermEvidence) -> (i64, u32, u32) {
    match evidence {
        TermEvidence::ExplicitUserTerm => (0, 0, 0),
        TermEvidence::ConfirmedCorrection => (1, 0, 0),
        TermEvidence::SelectedTypedTerm { uses, sessions } => (2, uses, sessions),
        TermEvidence::UnconfirmedVoice | TermEvidence::UnconfirmedClipboard => (3, 0, 0),
    }
}
