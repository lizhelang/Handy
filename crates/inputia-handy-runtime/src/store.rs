//! Handy 统一索引的单写者持久存储；外部调用方在后台线程串行调用。
//!
//! 当前正文为跨源搜索物化一份；去重账本只有事件摘要，不保存正文。
//! 旧版本仅在更新时归档，初次导入不额外复制一份历史正文。

use std::{collections::HashSet, fmt, path::Path, time::Duration};

use crate::source::{SnapshotHeader, SnapshotRecord};

use crate::learning::{ApplyContribution, ContributionInput, LearningError, LearningLedger};
use inputia_core::integration::events::Identifier;
use inputia_core::integration::{
    privacy::{PrivacyContext, PrivacyPolicy},
    terms::HotwordBudget,
};
use rusqlite::{
    params, Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const SCHEMA_VERSION: i64 = 1;
const MAX_PAGE: u32 = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Voice,
    Clipboard,
    SavedSnippet,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentType {
    Text,
    Image,
    Files,
    Html,
    Rtf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceTrust {
    Verified,
    Observed,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceOperation {
    Upsert,
    Delete,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemSnapshot {
    pub source_kind: SourceKind,
    pub content_type: ContentType,
    pub text: Option<String>,
    pub title: Option<String>,
    pub starred: bool,
    pub pinned: bool,
    pub created_at_ms: i64,
    pub asset_ref: Option<String>,
    pub source_app: Option<String>,
    pub source_trust: SourceTrust,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceChange {
    pub store_id: String,
    pub seq: u64,
    pub event_id: String,
    pub record_id: String,
    pub revision: u64,
    pub operation: SourceOperation,
    pub policy_epoch: u64,
    pub payload: Option<ItemSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexedItem {
    pub item_id: String,
    pub store_id: String,
    pub record_id: String,
    pub revision: u64,
    pub snapshot: ItemSnapshot,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentRevision {
    pub item_id: String,
    pub revision: u64,
    pub snapshot: ItemSnapshot,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LearnedTermView {
    pub term: String,
    pub contributions: u64,
    pub explicitly_confirmed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermSnapshot {
    pub policy_epoch: u64,
    pub learning_generation: u64,
    pub terms: Vec<String>,
}

/// 仅决定已持久化历史能否进入本地投影，不授予采集、学习或远程使用权限。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryRetentionPolicy {
    pub epoch: u64,
    pub retain_unknown: bool,
    pub excluded_source_apps: Vec<String>,
}

impl HistoryRetentionPolicy {
    fn permits(&self, snapshot: &ItemSnapshot) -> bool {
        if !self.retain_unknown && snapshot.source_trust != SourceTrust::Verified {
            return false;
        }
        !snapshot.source_app.as_deref().is_some_and(|app| {
            self.excluded_source_apps
                .iter()
                .any(|excluded| excluded == app)
                || inputia_core::AppPolicy::default().excludes(&inputia_core::AppContext::new(app))
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryQuery {
    pub search: Option<String>,
    pub source_kind: Option<SourceKind>,
    pub content_type: Option<ContentType>,
    pub starred_only: bool,
    pub limit: u32,
    pub offset: u64,
}

impl Default for HistoryQuery {
    fn default() -> Self {
        Self {
            search: None,
            source_kind: None,
            content_type: None,
            starred_only: false,
            limit: 50,
            offset: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApplyOutcome {
    Applied,
    Replay,
    StaleSuppressed,
    DeletedSuppressed,
}

/// 计数描述本次对账动作；重复快照不新增修订、回执或删除记录。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SnapshotRestoreOutcome {
    pub applied: u64,
    pub stale_suppressed: u64,
    pub deleted_suppressed: u64,
    pub missing_deleted: u64,
    pub through_sequence: u64,
}

#[derive(Debug)]
pub enum StoreError {
    Output(crate::output_ledger::OutputLedgerError),
    Learning(LearningError),
    Io(std::io::Error),
    Sqlite(rusqlite::Error),
    Serialization(serde_json::Error),
    Invalid(&'static str),
    ProfileMismatch,
    UnsupportedSchema(i64),
    UnknownStore,
    SourceConflict,
    RetiredStore,
    ReconciliationRequired,
    SequenceGap { expected: u64, actual: u64 },
    ReplayUnknown,
    EventConflict,
    SnapshotRegression { current: u64, actual: u64 },
    EpochMismatch { expected: u64, actual: u64 },
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // 不将来自数据库的正文或外部输入拼入诊断。
        match self {
            Self::Output(error) => write!(f, "{error}"),
            Self::Learning(error) => write!(f, "{error}"),
            Self::Io(_) => write!(f, "integration file operation failed"),
            Self::Sqlite(_) => write!(f, "integration SQLite operation failed"),
            Self::Serialization(_) => write!(f, "integration serialization failed"),
            Self::Invalid(reason) => write!(f, "invalid integration input: {reason}"),
            Self::ProfileMismatch => write!(f, "integration profile mismatch"),
            Self::UnsupportedSchema(version) => write!(f, "unsupported schema version {version}"),
            Self::UnknownStore => write!(f, "source store is not registered"),
            Self::SourceConflict => write!(f, "source store identity conflict"),
            Self::RetiredStore => write!(f, "source store is retired"),
            Self::ReconciliationRequired => write!(f, "source replacement requires reconciliation"),
            Self::SequenceGap { expected, actual } => {
                write!(
                    f,
                    "source sequence gap: expected {expected}, received {actual}"
                )
            }
            Self::ReplayUnknown => write!(f, "old sequence has no matching event receipt"),
            Self::EventConflict => {
                write!(f, "event identity or payload differs from stored receipt")
            }
            Self::SnapshotRegression { current, actual } => {
                write!(
                    f,
                    "snapshot sequence regressed: current {current}, received {actual}"
                )
            }
            Self::EpochMismatch { expected, actual } => {
                write!(
                    f,
                    "policy epoch mismatch: expected {expected}, received {actual}"
                )
            }
        }
    }
}

impl std::error::Error for StoreError {}
impl From<crate::output_ledger::OutputLedgerError> for StoreError {
    fn from(error: crate::output_ledger::OutputLedgerError) -> Self {
        Self::Output(error)
    }
}
impl From<LearningError> for StoreError {
    fn from(error: LearningError) -> Self {
        Self::Learning(error)
    }
}
impl From<std::io::Error> for StoreError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
impl From<rusqlite::Error> for StoreError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sqlite(value)
    }
}
impl From<serde_json::Error> for StoreError {
    fn from(value: serde_json::Error) -> Self {
        Self::Serialization(value)
    }
}

pub type StoreResult<T> = Result<T, StoreError>;

/// 连接由统一服务拥有，不允许 Host 按键线程持有或等待该连接。
pub struct IntegrationStore {
    conn: Connection,
}

impl IntegrationStore {
    pub fn initialize_outputs(&mut self) -> StoreResult<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        crate::output_ledger::initialize(&tx)?;
        crate::output_ledger::recover_inflight(&tx)?;
        tx.commit()?;
        Ok(())
    }

    pub fn prepare_output(
        &mut self,
        intent: &crate::output_ledger::OutputIntent,
    ) -> StoreResult<crate::output_ledger::OutputRecord> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let record = crate::output_ledger::prepare(&tx, intent)?;
        if record.state == crate::output_ledger::OutputState::Prepared {
            validate_output_item(&tx, intent)?;
        }
        tx.commit()?;
        Ok(record)
    }

    pub fn claim_output(
        &mut self,
        intent: &crate::output_ledger::OutputIntent,
    ) -> StoreResult<bool> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        validate_output_item(&tx, intent)?;
        let claimed = crate::output_ledger::claim_dispatch(&tx, intent)?;
        tx.commit()?;
        Ok(claimed)
    }

    pub fn finish_output(
        &mut self,
        intent: &crate::output_ledger::OutputIntent,
        outcome: crate::output_ledger::OutputOutcome,
    ) -> StoreResult<crate::output_ledger::OutputRecord> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let result = crate::output_ledger::finish(&tx, intent, outcome)?;
        tx.commit()?;
        Ok(result)
    }

    pub fn output_record(
        &self,
        id: &str,
    ) -> StoreResult<Option<crate::output_ledger::OutputRecord>> {
        Ok(crate::output_ledger::get(&self.conn, id)?)
    }
    pub fn history_retention_policy(&self) -> StoreResult<HistoryRetentionPolicy> {
        let tx = self.conn.unchecked_transaction()?;
        let stored: Option<String> = tx
            .query_row(
                "SELECT value FROM integration_meta WHERE key='history_retention_policy'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let mut policy = match stored {
            Some(raw) => serde_json::from_str(&raw)?,
            None => HistoryRetentionPolicy {
                epoch: 0,
                retain_unknown: true,
                excluded_source_apps: Vec::new(),
            },
        };
        policy.epoch = epoch(&tx)?;
        Ok(policy)
    }

    /// 重新核验旧历史时保持原事件摘要；此方法绝不写学习贡献。
    /// 授权必须与规范库当前保留规则完全一致，不能由客户端捏造。
    pub fn apply_retained_history(
        &mut self,
        changes: &[SourceChange],
        policy: &HistoryRetentionPolicy,
    ) -> StoreResult<Vec<ApplyOutcome>> {
        if *policy != self.history_retention_policy()? {
            return Err(StoreError::Invalid(
                "retention authorization differs from stored policy",
            ));
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if epoch(&tx)? != policy.epoch {
            return Err(StoreError::EpochMismatch {
                expected: epoch(&tx)?,
                actual: policy.epoch,
            });
        }
        let outcomes = changes
            .iter()
            .map(|change| apply_authorized(&tx, change, Some(policy)))
            .collect::<StoreResult<Vec<_>>>()?;
        tx.commit()?;
        Ok(outcomes)
    }

    /// 控制中心的有界词库列表；调用者必须是已授权的本地设置入口。
    pub fn list_terms(&self, limit: u32, offset: u64) -> StoreResult<Vec<LearnedTermView>> {
        if limit == 0 || limit > 500 {
            return Err(StoreError::Invalid("term page size out of range"));
        }
        let mut query=self.conn.prepare("SELECT term,COUNT(*),MIN(evidence)<=1 FROM learning_contributions GROUP BY term ORDER BY MIN(evidence),term LIMIT ?1 OFFSET ?2")?;
        let rows = query.query_map(params![limit, checked_number(offset)?], |row| {
            Ok(LearnedTermView {
                term: row.get(0)?,
                contributions: row.get(1)?,
                explicitly_confirmed: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn learning_initialized(&self) -> StoreResult<bool> {
        Ok(self.conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='learning_meta')",[],|row|row.get(0))?)
    }

    /// 学习表及撤销触发器属于规范库的同一事务域；key 由受管配置提供。
    pub fn enable_learning(&mut self, key: &[u8]) -> StoreResult<()> {
        let ledger = LearningLedger::new(key)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        ledger.install(&tx)?;
        ledger.advance_epoch(&tx, epoch(&tx)?)?;
        tx.execute(
            "INSERT OR IGNORE INTO integration_meta(key,value) VALUES('learning_generation','0')",
            [],
        )?;
        tx.execute_batch(
            "DROP TRIGGER IF EXISTS integration_delete_learning;
             DROP TRIGGER IF EXISTS integration_revise_learning;
             CREATE TRIGGER integration_delete_learning AFTER DELETE ON integration_items
             BEGIN
               UPDATE integration_meta SET value=CAST(value AS INTEGER)+1 WHERE key='learning_generation';
               DELETE FROM learning_contributions WHERE store_id=OLD.store_id AND record_id=OLD.record_id;
               INSERT INTO learning_sources(store_id,record_id,revision,deleted) VALUES(OLD.store_id,OLD.record_id,OLD.revision,1)
               ON CONFLICT(store_id,record_id) DO UPDATE SET revision=max(revision,excluded.revision),deleted=1;
             END;
             CREATE TRIGGER integration_revise_learning AFTER UPDATE ON integration_items
             WHEN OLD.content_revision<>NEW.content_revision
             BEGIN
               UPDATE integration_meta SET value=CAST(value AS INTEGER)+1 WHERE key='learning_generation';
               DELETE FROM learning_contributions WHERE store_id=NEW.store_id AND record_id=NEW.record_id;
               INSERT INTO learning_sources(store_id,record_id,revision,deleted) VALUES(NEW.store_id,NEW.record_id,NEW.revision,0)
               ON CONFLICT(store_id,record_id) DO UPDATE SET revision=max(revision,excluded.revision);
             END;
             CREATE TRIGGER IF NOT EXISTS integration_advance_learning_epoch AFTER UPDATE ON integration_meta
             WHEN NEW.key='policy_epoch'
             BEGIN UPDATE learning_meta SET epoch=CAST(NEW.value AS INTEGER) WHERE singleton=1; END;"
        )?;
        // 首次启用学习也继承规范库已有的删除屏障，不能等到下一次删除再建立。
        tx.execute_batch(
            "INSERT INTO learning_sources(store_id,record_id,revision,deleted)
             SELECT instance.store_id,tombstone.record_id,tombstone.deleted_revision,1
             FROM integration_tombstones tombstone JOIN integration_instances instance USING(logical_name)
             WHERE 1 ON CONFLICT(store_id,record_id) DO UPDATE SET revision=max(revision,excluded.revision),deleted=1;"
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn contribute_term(
        &mut self,
        key: &[u8],
        input: &ContributionInput,
        policy: &PrivacyPolicy,
        context: PrivacyContext,
    ) -> StoreResult<ApplyContribution> {
        let ledger = LearningLedger::new(key)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = epoch(&tx)?;
        if current != policy.epoch {
            return Err(StoreError::EpochMismatch {
                expected: current,
                actual: policy.epoch,
            });
        }
        let live: Option<(i64,i64,String)> = tx
            .query_row(
                "SELECT revision,content_revision,snapshot FROM integration_items WHERE store_id=?1 AND record_id=?2",
                params![
                    input.source.store_id.as_str(),
                    input.source.record_id.as_str()
                ],
                |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)),
            )
            .optional()?;
        let Some((revision, content_revision, snapshot)) = live else {
            return Err(StoreError::Invalid("learning source is absent or changed"));
        };
        if revision != checked_number(input.source_revision)? {
            return Err(StoreError::Invalid("learning source is absent or changed"));
        }
        let snapshot: ItemSnapshot = serde_json::from_str(&snapshot)?;
        let trust = match snapshot.source_trust {
            SourceTrust::Verified => inputia_core::integration::privacy::SourceTrust::Verified,
            SourceTrust::Observed => inputia_core::integration::privacy::SourceTrust::Observed,
            SourceTrust::Unknown => inputia_core::integration::privacy::SourceTrust::Unknown,
        };
        let source_sensitive = context.source_sensitive
            || snapshot.source_app.as_deref().is_some_and(|app| {
                inputia_core::AppPolicy::default().excludes(&inputia_core::AppContext::new(app))
            });
        let context = inputia_core::integration::privacy::PrivacyContext {
            source_trust: if context.source_trust
                == inputia_core::integration::privacy::SourceTrust::Verified
            {
                trust
            } else {
                context.source_trust
            },
            source_sensitive,
            ..context
        };
        let contribution = ContributionInput {
            contribution_id: input.contribution_id.clone(),
            source: input.source.clone(),
            source_revision: content_revision as u64,
            policy_epoch: input.policy_epoch,
            term: input.term.clone(),
            evidence: input.evidence,
            explicit_relearn: input.explicit_relearn,
        };
        let result = ledger.apply_contribution(&tx, &contribution, policy, context)?;
        if result == ApplyContribution::Applied {
            tx.execute("UPDATE integration_meta SET value=CAST(value AS INTEGER)+1 WHERE key='learning_generation'",[])?;
        }
        tx.commit()?;
        Ok(result)
    }

    /// 词屏障、贡献移除和统一策略版本同一事务，失败时全量回滚。
    pub fn forget_term(&mut self, key: &[u8], term: &str, expected_epoch: u64) -> StoreResult<u64> {
        let ledger = LearningLedger::new(key)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = epoch(&tx)?;
        if current != expected_epoch {
            return Err(StoreError::EpochMismatch {
                expected: current,
                actual: expected_epoch,
            });
        }
        let next = current
            .checked_add(1)
            .ok_or(StoreError::Invalid("policy epoch overflow"))?;
        checked_number(next)?;
        ledger.forget_term(&tx, term, next)?;
        tx.execute("UPDATE integration_meta SET value=CAST(value AS INTEGER)+1 WHERE key='learning_generation'",[])?;
        tx.execute(
            "UPDATE integration_meta SET value=?1 WHERE key='policy_epoch'",
            [next.to_string()],
        )?;
        tx.commit()?;
        Ok(next)
    }

    pub fn hotwords(
        &self,
        key: &[u8],
        policy: &PrivacyPolicy,
        context: PrivacyContext,
        explicit: &[String],
        budget: HotwordBudget,
    ) -> StoreResult<Vec<String>> {
        let ledger = LearningLedger::new(key)?;
        Ok(ledger.hotwords(
            &self.conn,
            policy,
            context,
            epoch(&self.conn)?,
            explicit,
            budget,
        )?)
    }

    pub fn term_snapshot(
        &self,
        key: &[u8],
        policy: &PrivacyPolicy,
        context: PrivacyContext,
        explicit: &[String],
        budget: HotwordBudget,
    ) -> StoreResult<TermSnapshot> {
        let tx = self.conn.unchecked_transaction()?;
        let ledger = LearningLedger::new(key)?;
        let policy_epoch = epoch(&tx)?;
        let learning_generation: u64 = tx.query_row(
            "SELECT CAST(value AS INTEGER) FROM integration_meta WHERE key='learning_generation'",
            [],
            |row| row.get(0),
        )?;
        let terms = ledger.hotwords(&tx, policy, context, policy_epoch, explicit, budget)?;
        Ok(TermSnapshot {
            policy_epoch,
            learning_generation,
            terms,
        })
    }

    /// 在线消费者提交前复核两个版本；离线消费者仍须遵守 2 秒租约。
    pub fn term_snapshot_is_current(&self, snapshot: &TermSnapshot) -> StoreResult<bool> {
        let tx = self.conn.unchecked_transaction()?;
        let generation: u64 = tx.query_row(
            "SELECT CAST(value AS INTEGER) FROM integration_meta WHERE key='learning_generation'",
            [],
            |row| row.get(0),
        )?;
        Ok(snapshot.policy_epoch == epoch(&tx)? && snapshot.learning_generation == generation)
    }

    /// 打开版本化索引；已有库的 profile 不符时拒绝，不接管陌生数据库。
    pub fn open(path: impl AsRef<Path>, profile_id: &str) -> StoreResult<Self> {
        identifier(profile_id)?;
        let path = path.as_ref();
        // macOS 的 /var 与 /tmp 是系统目录别名；只解析父目录，保留最终文件的 NOFOLLOW。
        let file_name = path
            .file_name()
            .ok_or(StoreError::Invalid("database filename"))?;
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let path = parent.canonicalize()?.join(file_name);
        let mut options = std::fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(_) => (),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(error.into()),
        }
        let mut conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        conn.busy_timeout(Duration::from_millis(100))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let version: i64 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version != 0 && version != SCHEMA_VERSION {
            return Err(StoreError::UnsupportedSchema(version));
        }
        if version == 0 {
            let tables: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
                [],
                |row| row.get(0),
            )?;
            if tables != 0 {
                return Err(StoreError::UnsupportedSchema(0));
            }
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(SCHEMA)?;
            tx.execute(
                "INSERT INTO integration_meta(key,value) VALUES ('profile_id',?1),('policy_epoch','1')",
                [profile_id],
            )?;
            tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            tx.commit()?;
        } else {
            let stored: String = conn.query_row(
                "SELECT value FROM integration_meta WHERE key='profile_id'",
                [],
                |row| row.get(0),
            )?;
            if stored != profile_id {
                return Err(StoreError::ProfileMismatch);
            }
        }
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        Ok(Self { conn })
    }

    /// 逻辑来源与物理库 UUID 显式绑定；幂等注册不重置游标。
    pub fn register_source(&mut self, logical_name: &str, store_id: &str) -> StoreResult<()> {
        identifier(logical_name)?;
        identifier(store_id)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let active: Option<String> = tx
            .query_row(
                "SELECT active_store_id FROM integration_sources WHERE logical_name=?1",
                [logical_name],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(active) = active {
            if active != store_id {
                return Err(StoreError::SourceConflict);
            }
        } else {
            let known: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM integration_instances WHERE store_id=?1)",
                [store_id],
                |row| row.get(0),
            )?;
            if known {
                return Err(StoreError::SourceConflict);
            }
            tx.execute(
                "INSERT INTO integration_sources VALUES (?1,?2)",
                params![logical_name, store_id],
            )?;
            tx.execute(
                "INSERT INTO integration_instances VALUES (?1,?2,1,0)",
                params![store_id, logical_name],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// 仅无内容、无删除账本的空来源可直接换库；其余必须先提交迁移映射。
    pub fn replace_source(
        &mut self,
        logical_name: &str,
        expected_old_store_id: &str,
        new_store_id: &str,
    ) -> StoreResult<()> {
        identifier(logical_name)?;
        identifier(expected_old_store_id)?;
        identifier(new_store_id)?;
        if expected_old_store_id == new_store_id {
            return Err(StoreError::SourceConflict);
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let active: Option<String> = tx
            .query_row(
                "SELECT active_store_id FROM integration_sources WHERE logical_name=?1",
                [logical_name],
                |row| row.get(0),
            )
            .optional()?;
        if active.as_deref() != Some(expected_old_store_id) {
            return Err(StoreError::SourceConflict);
        }
        let needs_mapping: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM integration_items WHERE logical_name=?1) OR EXISTS(SELECT 1 FROM integration_tombstones WHERE logical_name=?1)",
            [logical_name], |row| row.get(0),
        )?;
        if needs_mapping {
            return Err(StoreError::ReconciliationRequired);
        }
        let known: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM integration_instances WHERE store_id=?1)",
            [new_store_id],
            |row| row.get(0),
        )?;
        if known {
            return Err(StoreError::SourceConflict);
        }
        tx.execute(
            "UPDATE integration_instances SET active=0 WHERE store_id=?1",
            [expected_old_store_id],
        )?;
        tx.execute(
            "INSERT INTO integration_instances VALUES (?1,?2,1,0)",
            params![new_store_id, logical_name],
        )?;
        tx.execute(
            "UPDATE integration_sources SET active_store_id=?1 WHERE logical_name=?2",
            params![new_store_id, logical_name],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn cursor(&self, store_id: &str) -> StoreResult<u64> {
        let value: Option<i64> = self
            .conn
            .query_row(
                "SELECT cursor FROM integration_instances WHERE store_id=?1",
                [store_id],
                |row| row.get(0),
            )
            .optional()?;
        value.map(|n| n as u64).ok_or(StoreError::UnknownStore)
    }

    pub fn policy_epoch(&self) -> StoreResult<u64> {
        epoch(&self.conn)
    }

    /// CAS 更新策略屏障；失败的旧消费者不能自行跳过服务端当前版本。
    pub fn advance_policy_epoch(&mut self, expected: u64, new_epoch: u64) -> StoreResult<()> {
        checked_number(new_epoch)?;
        if new_epoch <= expected {
            return Err(StoreError::Invalid("epoch must increase"));
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current = epoch(&tx)?;
        if current != expected {
            return Err(StoreError::EpochMismatch {
                expected: current,
                actual: expected,
            });
        }
        tx.execute(
            "UPDATE integration_meta SET value=?1 WHERE key='policy_epoch'",
            [new_epoch.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn apply_change(&mut self, change: &SourceChange) -> StoreResult<ApplyOutcome> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let outcome = apply(&tx, change)?;
        tx.commit()?;
        Ok(outcome)
    }

    /// 一个批次共用完整事务；任何缺口或冲突都回滚本批所有变更和游标。
    pub fn apply_changes(&mut self, changes: &[SourceChange]) -> StoreResult<Vec<ApplyOutcome>> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let outcomes = changes
            .iter()
            .map(|change| apply(&tx, change))
            .collect::<StoreResult<Vec<_>>>()?;
        tx.commit()?;
        Ok(outcomes)
    }

    /// 原子恢复源的完整一致性快照，并为缺失记录保留不可复活的删除屏障。
    ///
    /// 调用者必须传入同一次 SourceOutbox::with_snapshot 视图的全部页面，不能用
    /// 筛选结果或单页对账。源身份、策略及水位仅能沿当前已登记来源向前恢复。
    /// 快照无法恢复已被 ACK 回收的事件摘要，因此不会为扫描记录伪造去重回执。
    pub fn restore_source_snapshot(
        &mut self,
        header: &SnapshotHeader,
        records: &[SnapshotRecord],
    ) -> StoreResult<SnapshotRestoreOutcome> {
        self.restore_authorized_snapshot(header, records, None)
    }

    pub fn restore_retained_history(
        &mut self,
        header: &SnapshotHeader,
        records: &[SnapshotRecord],
        policy: &HistoryRetentionPolicy,
    ) -> StoreResult<SnapshotRestoreOutcome> {
        if *policy != self.history_retention_policy()? {
            return Err(StoreError::Invalid(
                "retention authorization differs from stored policy",
            ));
        }
        self.restore_authorized_snapshot(header, records, Some(policy))
    }

    fn restore_authorized_snapshot(
        &mut self,
        header: &SnapshotHeader,
        records: &[SnapshotRecord],
        retention: Option<&HistoryRetentionPolicy>,
    ) -> StoreResult<SnapshotRestoreOutcome> {
        identifier(&header.store_id)?;
        let through = checked_number(header.through_sequence)?;
        checked_number(header.policy_epoch)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let current_epoch = epoch(&tx)?;
        if (retention.is_none() && current_epoch != header.policy_epoch)
            || header.policy_epoch > current_epoch
            || retention.is_some_and(|policy| policy.epoch != current_epoch)
        {
            return Err(StoreError::EpochMismatch {
                expected: current_epoch,
                actual: header.policy_epoch,
            });
        }
        let instance: Option<(String, bool, i64)> = tx
            .query_row(
                "SELECT logical_name,active,cursor FROM integration_instances WHERE store_id=?1",
                [&header.store_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let (logical_name, active, cursor) = instance.ok_or(StoreError::UnknownStore)?;
        if !active {
            return Err(StoreError::RetiredStore);
        }
        let previous_floor = snapshot_floor(&tx, &header.store_id)?;
        let current = (cursor as u64).max(previous_floor);
        if header.through_sequence < current {
            return Err(StoreError::SnapshotRegression {
                current,
                actual: header.through_sequence,
            });
        }
        let mut seen = HashSet::with_capacity(records.len());
        let mut outcome = SnapshotRestoreOutcome {
            through_sequence: header.through_sequence,
            ..SnapshotRestoreOutcome::default()
        };
        for record in records {
            identifier(&record.record_id)?;
            let revision = checked_number(record.revision)?;
            if revision == 0 || !seen.insert(record.record_id.as_str()) {
                return Err(StoreError::Invalid(
                    "snapshot record revision or duplicate identity",
                ));
            }
            if let Some(policy) = retention {
                withdraw_disallowed_record(&tx, &header.store_id, &record.record_id, policy)?;
            }
            match apply_record(
                &tx,
                &logical_name,
                &header.store_id,
                &record.record_id,
                revision,
                record
                    .payload
                    .as_ref()
                    .filter(|snapshot| retention.is_none_or(|policy| policy.permits(snapshot))),
            )? {
                ApplyOutcome::Applied => outcome.applied += 1,
                ApplyOutcome::StaleSuppressed => outcome.stale_suppressed += 1,
                ApplyOutcome::DeletedSuppressed => outcome.deleted_suppressed += 1,
                ApplyOutcome::Replay => unreachable!("record application has no event receipt"),
            }
        }
        let missing = {
            let mut stmt = tx.prepare(
                "SELECT item_id,record_id,revision FROM integration_items WHERE store_id=?1",
            )?;
            let rows = stmt.query_map([&header.store_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        for (id, record_id, revision) in missing {
            if !seen.contains(record_id.as_str()) {
                tx.execute("INSERT INTO integration_tombstones(logical_name,record_id,deleted_revision) VALUES (?1,?2,?3) ON CONFLICT(logical_name,record_id) DO UPDATE SET deleted_revision=max(deleted_revision,excluded.deleted_revision)", params![logical_name, record_id, revision])?;
                tx.execute("DELETE FROM integration_items WHERE item_id=?1", [&id])?;
                outcome.missing_deleted += 1;
            }
        }
        tx.execute("INSERT INTO integration_meta(key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![snapshot_floor_key(&header.store_id), header.through_sequence.to_string()])?;
        tx.execute(
            "UPDATE integration_instances SET cursor=?1 WHERE store_id=?2",
            params![through, header.store_id],
        )?;
        tx.commit()?;
        Ok(outcome)
    }

    pub fn get(&self, id: &str) -> StoreResult<Option<IndexedItem>> {
        let raw = self.conn.query_row("SELECT item_id,store_id,record_id,revision,snapshot FROM integration_items WHERE item_id=?1", [id], read_item).optional()?;
        raw.map(decode_item).transpose()
    }

    /// 对当前物化版本查询；搜索按字面包含匹配，避免 SQL 通配符扩大范围。
    pub fn query(&self, query: &HistoryQuery) -> StoreResult<Vec<IndexedItem>> {
        if query.limit == 0 || query.limit > MAX_PAGE {
            return Err(StoreError::Invalid("page size out of range"));
        }
        let offset = checked_number(query.offset)?;
        let kind = query.source_kind.map(enum_value).transpose()?;
        let content_type = query.content_type.map(enum_value).transpose()?;
        let mut stmt = self.conn.prepare_cached(
            "SELECT item_id,store_id,record_id,revision,snapshot FROM integration_items
             WHERE (?1 IS NULL OR instr(lower(COALESCE(json_extract(snapshot,'$.text'),'')),lower(?1))>0 OR instr(lower(COALESCE(json_extract(snapshot,'$.title'),'')),lower(?1))>0)
             AND (?2 IS NULL OR source_kind=?2) AND (?3 IS NULL OR content_type=?3)
             AND (?4=0 OR starred=1)
             ORDER BY pinned DESC,created_at_ms DESC,item_id ASC LIMIT ?5 OFFSET ?6")?;
        let rows = stmt.query_map(
            params![
                query.search,
                kind,
                content_type,
                query.starred_only,
                query.limit,
                offset
            ],
            read_item,
        )?;
        rows.map(|row| decode_item(row?)).collect()
    }

    /// 返回旧修订及当前版本，按源 revision 排序；删除后不再返回旧正文。
    pub fn revisions(&self, id: &str) -> StoreResult<Vec<ContentRevision>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT revision,snapshot FROM integration_revisions WHERE item_id=?1
             UNION ALL SELECT content_revision,snapshot FROM integration_items WHERE item_id=?1 ORDER BY revision")?;
        let rows = stmt.query_map([id], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.map(|row| {
            let (revision, snapshot) = row?;
            Ok(ContentRevision {
                item_id: id.to_owned(),
                revision: revision as u64,
                snapshot: serde_json::from_str(&snapshot)?,
            })
        })
        .collect()
    }

    pub fn item_count(&self) -> StoreResult<u64> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM integration_items", [], |row| {
                row.get::<_, i64>(0)
            })? as u64)
    }
}

/// 长度前缀使 store/record 边界无歧义，不依赖正文、可重用整数或分隔符猜测。
pub fn item_id(store_id: &str, record_id: &str) -> String {
    format!(
        "{}:{}{}:{}",
        store_id.len(),
        store_id,
        record_id.len(),
        record_id
    )
}

fn identifier(value: &str) -> StoreResult<()> {
    Identifier::parse(value)
        .map(|_| ())
        .map_err(StoreError::Invalid)
}

fn validate_output_item(
    conn: &Connection,
    intent: &crate::output_ledger::OutputIntent,
) -> StoreResult<()> {
    let current = epoch(conn)?;
    if current != intent.policy_epoch {
        return Err(StoreError::EpochMismatch {
            expected: current,
            actual: intent.policy_epoch,
        });
    }
    let revision: Option<i64> = conn
        .query_row(
            "SELECT revision FROM integration_items WHERE item_id=?1",
            [&intent.item_id],
            |row| row.get(0),
        )
        .optional()?;
    if revision != Some(checked_number(intent.revision)?) {
        return Err(StoreError::Invalid("output item was deleted or changed"));
    }
    Ok(())
}

fn checked_number(value: u64) -> StoreResult<i64> {
    i64::try_from(value).map_err(|_| StoreError::Invalid("integer exceeds SQLite range"))
}

fn enum_value(value: impl Serialize) -> StoreResult<String> {
    let value = serde_json::to_value(value)?;
    value
        .as_str()
        .map(str::to_owned)
        .ok_or(StoreError::Invalid("enum must be a string"))
}

fn epoch(conn: &Connection) -> StoreResult<u64> {
    let value: String = conn.query_row(
        "SELECT value FROM integration_meta WHERE key='policy_epoch'",
        [],
        |row| row.get(0),
    )?;
    value
        .parse()
        .map_err(|_| StoreError::Invalid("stored policy epoch"))
}

fn snapshot_floor_key(store_id: &str) -> String {
    format!("snapshot_floor:{store_id}")
}

fn snapshot_floor(conn: &Connection, store_id: &str) -> StoreResult<u64> {
    let value: Option<String> = conn
        .query_row(
            "SELECT value FROM integration_meta WHERE key=?1",
            [snapshot_floor_key(store_id)],
            |row| row.get(0),
        )
        .optional()?;
    value
        .map(|value| {
            value
                .parse()
                .map_err(|_| StoreError::Invalid("stored snapshot floor"))
        })
        .transpose()
        .map(|value| value.unwrap_or(0))
}

type RawItem = (String, String, String, i64, String);
fn read_item(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawItem> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
    ))
}
fn decode_item(
    (item_id, store_id, record_id, revision, snapshot): RawItem,
) -> StoreResult<IndexedItem> {
    Ok(IndexedItem {
        item_id,
        store_id,
        record_id,
        revision: revision as u64,
        snapshot: serde_json::from_str(&snapshot)?,
    })
}

fn apply(tx: &Transaction<'_>, change: &SourceChange) -> StoreResult<ApplyOutcome> {
    apply_authorized(tx, change, None)
}

fn apply_authorized(
    tx: &Transaction<'_>,
    change: &SourceChange,
    retention: Option<&HistoryRetentionPolicy>,
) -> StoreResult<ApplyOutcome> {
    identifier(&change.store_id)?;
    identifier(&change.event_id)?;
    identifier(&change.record_id)?;
    let seq = checked_number(change.seq)?;
    let revision = checked_number(change.revision)?;
    checked_number(change.policy_epoch)?;
    if seq == 0 || revision == 0 {
        return Err(StoreError::Invalid(
            "sequence and revision must be positive",
        ));
    }
    if (change.operation == SourceOperation::Upsert) != change.payload.is_some() {
        return Err(StoreError::Invalid("payload does not match operation"));
    }
    let current_epoch = epoch(tx)?;
    if (retention.is_none() && current_epoch != change.policy_epoch)
        || change.policy_epoch > current_epoch
        || retention.is_some_and(|policy| policy.epoch != current_epoch)
    {
        return Err(StoreError::EpochMismatch {
            expected: current_epoch,
            actual: change.policy_epoch,
        });
    }
    let instance: Option<(String, bool, i64)> = tx
        .query_row(
            "SELECT logical_name,active,cursor FROM integration_instances WHERE store_id=?1",
            [&change.store_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let (logical_name, active, cursor) = instance.ok_or(StoreError::UnknownStore)?;
    if !active {
        return Err(StoreError::RetiredStore);
    }
    let digest = Sha256::digest(serde_json::to_vec(change)?).to_vec();
    if let Some(policy) = retention {
        withdraw_disallowed_record(tx, &change.store_id, &change.record_id, policy)?;
    }
    let receipt: Option<Vec<u8>> = tx
        .query_row(
            "SELECT digest FROM integration_events WHERE event_id=?1",
            [&change.event_id],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(receipt) = receipt {
        if receipt != digest {
            return Err(StoreError::EventConflict);
        }
        return Ok(ApplyOutcome::Replay);
    }
    if change.seq <= snapshot_floor(tx, &change.store_id)? {
        let known_sequence: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM integration_events WHERE store_id=?1 AND seq=?2)",
            params![change.store_id, seq],
            |row| row.get(0),
        )?;
        if known_sequence {
            return Err(StoreError::EventConflict);
        }
        // 没有原事件摘要只能证明其已被快照覆盖，不能声称确认了同一事件。
        return Ok(ApplyOutcome::StaleSuppressed);
    }
    if seq <= cursor {
        return Err(StoreError::ReplayUnknown);
    }
    if cursor.checked_add(1) != Some(seq) {
        return Err(StoreError::SequenceGap {
            expected: cursor as u64 + 1,
            actual: change.seq,
        });
    }
    let outcome = apply_record(
        tx,
        &logical_name,
        &change.store_id,
        &change.record_id,
        revision,
        change
            .payload
            .as_ref()
            .filter(|snapshot| retention.is_none_or(|policy| policy.permits(snapshot))),
    )?;
    tx.execute(
        "INSERT INTO integration_events(event_id,store_id,seq,digest) VALUES (?1,?2,?3,?4)",
        params![change.event_id, change.store_id, seq, digest],
    )?;
    tx.execute(
        "UPDATE integration_instances SET cursor=?1 WHERE store_id=?2",
        params![seq, change.store_id],
    )?;
    Ok(outcome)
}

/// 现行策略撤销独立于源 revision；同版本快照与已确认事件也须重核验。
/// 检查规范库当前正文，不能让一个旧拒绝事件误删较新的合法来源版本。
fn withdraw_disallowed_record(
    tx: &Transaction<'_>,
    store_id: &str,
    record_id: &str,
    policy: &HistoryRetentionPolicy,
) -> StoreResult<()> {
    let id = item_id(store_id, record_id);
    let existing: Option<(String, i64, String)> = tx
        .query_row(
            "SELECT logical_name,revision,snapshot FROM integration_items WHERE item_id=?1",
            [&id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    if let Some((logical_name, revision, raw)) = existing {
        let snapshot: ItemSnapshot = serde_json::from_str(&raw)?;
        if !policy.permits(&snapshot) {
            tx.execute("INSERT INTO integration_tombstones(logical_name,record_id,deleted_revision) VALUES(?1,?2,?3) ON CONFLICT(logical_name,record_id) DO UPDATE SET deleted_revision=max(deleted_revision,excluded.deleted_revision)",params![logical_name,record_id,revision])?;
            tx.execute("DELETE FROM integration_items WHERE item_id=?1", [&id])?;
            tx.execute("INSERT INTO integration_meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",params![format!("retention-exclusion:{id}"),policy.epoch.to_string()])?;
        }
    }
    Ok(())
}

fn apply_record(
    tx: &Transaction<'_>,
    logical_name: &str,
    store_id: &str,
    record_id: &str,
    revision: i64,
    payload: Option<&ItemSnapshot>,
) -> StoreResult<ApplyOutcome> {
    let id = item_id(store_id, record_id);
    let existing: Option<(i64, i64, String)> = tx
        .query_row(
            "SELECT revision,content_revision,snapshot FROM integration_items WHERE item_id=?1",
            [&id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    let deleted: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM integration_tombstones WHERE logical_name=?1 AND record_id=?2)", params![logical_name,record_id], |row| row.get(0))?;
    let outcome = if deleted {
        ApplyOutcome::DeletedSuppressed
    } else if existing
        .as_ref()
        .is_some_and(|(old_revision, _, _)| *old_revision >= revision)
    {
        ApplyOutcome::StaleSuppressed
    } else {
        match payload {
            None => {
                tx.execute("INSERT INTO integration_tombstones(logical_name,record_id,deleted_revision) VALUES (?1,?2,?3)", params![logical_name,record_id,revision])?;
                tx.execute("DELETE FROM integration_items WHERE item_id=?1", [&id])?;
            }
            Some(snapshot) => {
                let content_revision = if let Some((_, old_content_revision, old_snapshot)) =
                    existing
                {
                    let previous: ItemSnapshot = serde_json::from_str(&old_snapshot)?;
                    if previous.text != snapshot.text
                        || previous.asset_ref != snapshot.asset_ref
                        || previous.content_type != snapshot.content_type
                    {
                        tx.execute("INSERT INTO integration_revisions(item_id,revision,snapshot) VALUES (?1,?2,?3)",params![id,old_content_revision,old_snapshot])?;
                        revision
                    } else {
                        old_content_revision
                    }
                } else {
                    revision
                };
                tx.execute(
                    "INSERT INTO integration_items(item_id,logical_name,store_id,record_id,revision,content_revision,source_kind,content_type,starred,pinned,created_at_ms,snapshot)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
                     ON CONFLICT(item_id) DO UPDATE SET revision=excluded.revision,content_revision=excluded.content_revision,source_kind=excluded.source_kind,content_type=excluded.content_type,starred=excluded.starred,pinned=excluded.pinned,created_at_ms=excluded.created_at_ms,snapshot=excluded.snapshot",
                    params![id,logical_name,store_id,record_id,revision,content_revision,enum_value(snapshot.source_kind)?,enum_value(snapshot.content_type)?,snapshot.starred,snapshot.pinned,snapshot.created_at_ms,serde_json::to_string(snapshot)?])?;
            }
        }
        ApplyOutcome::Applied
    };
    Ok(outcome)
}

const SCHEMA: &str = "
CREATE TABLE integration_meta(key TEXT PRIMARY KEY,value TEXT NOT NULL);
CREATE TABLE integration_sources(logical_name TEXT PRIMARY KEY,active_store_id TEXT NOT NULL UNIQUE);
CREATE TABLE integration_instances(store_id TEXT PRIMARY KEY,logical_name TEXT NOT NULL REFERENCES integration_sources(logical_name),active INTEGER NOT NULL CHECK(active IN (0,1)),cursor INTEGER NOT NULL CHECK(cursor>=0));
CREATE TABLE integration_items(
 item_id TEXT PRIMARY KEY,logical_name TEXT NOT NULL REFERENCES integration_sources(logical_name),
 store_id TEXT NOT NULL REFERENCES integration_instances(store_id),record_id TEXT NOT NULL,revision INTEGER NOT NULL CHECK(revision>0),content_revision INTEGER NOT NULL CHECK(content_revision>0),
 source_kind TEXT NOT NULL,content_type TEXT NOT NULL,starred INTEGER NOT NULL,pinned INTEGER NOT NULL,created_at_ms INTEGER NOT NULL,snapshot TEXT NOT NULL CHECK(json_valid(snapshot)),UNIQUE(store_id,record_id));
CREATE INDEX integration_items_order ON integration_items(pinned DESC,created_at_ms DESC,item_id);
CREATE INDEX integration_items_source ON integration_items(source_kind,content_type);
CREATE TABLE integration_revisions(item_id TEXT NOT NULL REFERENCES integration_items(item_id) ON DELETE CASCADE,revision INTEGER NOT NULL,snapshot TEXT NOT NULL CHECK(json_valid(snapshot)),PRIMARY KEY(item_id,revision));
CREATE TABLE integration_tombstones(logical_name TEXT NOT NULL REFERENCES integration_sources(logical_name),record_id TEXT NOT NULL,deleted_revision INTEGER NOT NULL,PRIMARY KEY(logical_name,record_id));
CREATE TABLE integration_events(event_id TEXT PRIMARY KEY,store_id TEXT NOT NULL REFERENCES integration_instances(store_id),seq INTEGER NOT NULL,digest BLOB NOT NULL CHECK(length(digest)=32),UNIQUE(store_id,seq));
";
