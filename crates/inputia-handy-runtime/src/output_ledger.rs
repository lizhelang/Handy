//! 持久输出派发账本，不保存正文，也不执行平台副作用。
//!
//! 所有函数使用调用方的连接（`Transaction` 可解引用传入），不创建第二个 writer。
//! `claim_dispatch` 返回 true 后，调用方必须成功提交事务，再执行唯一一次外部派发。
//! 事务回滚后的 claim 不构成派发许可。数据库提交和目标应用插入无法组成原子事务，
//! 因而这里不承诺跨崩溃 exactly-once：崩溃恢复保守保留未知结果，绝不自动重派。

use std::fmt;

use inputia_core::integration::events::Identifier;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// 输出类型也是请求身份的一部分，不能将复制请求重放成插入请求。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputAction {
    InsertText,
    PasteAsset,
    Copy,
    CopyPlainText,
}

/// 一项操作只有一个执行所有者；重放不能换路线。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputOwner {
    Platform,
    Ime,
}

/// 只含有界的不透明标识，不可填入正文、路径或完整目标描述。
/// `target_id` 指向由原生适配器持有、派发前重新核验的完整目标 token。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputIntent {
    pub operation_id: String,
    pub item_id: String,
    pub revision: u64,
    /// 来源和 profile 是可选的以兼容旧账本；新入口应填写稳定不透明标识。
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub profile_id: Option<String>,
    #[serde(default)]
    pub deadline_at_ms: Option<u64>,
    pub target_id: Option<String>,
    pub owner: OutputOwner,
    pub policy_epoch: u64,
    pub action: OutputAction,
}

/// 派发成功仅意味着适配器完成派发，不能冒充目标应用确认。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputState {
    Prepared,
    Dispatched,
    Confirmed,
    DispatchedOnly,
    Uncertain,
    PendingTarget,
    Rejected,
}

/// PendingTarget/Rejected 仅用于尚未取得派发资格时的明确前置失败。
/// NotDispatched* 仅用于已取得 claim，但适配器明确证明尚未开始任何外部副作用。
/// 一旦开始副作用、等待回执超时或无法证明未派发，必须记录 Uncertain。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputOutcome {
    Confirmed,
    DispatchedOnly,
    Uncertain,
    PendingTarget,
    Rejected,
    NotDispatchedPendingTarget,
    NotDispatchedRejected,
}

impl From<OutputOutcome> for OutputState {
    fn from(value: OutputOutcome) -> Self {
        match value {
            OutputOutcome::Confirmed => Self::Confirmed,
            OutputOutcome::DispatchedOnly => Self::DispatchedOnly,
            OutputOutcome::Uncertain => Self::Uncertain,
            OutputOutcome::PendingTarget | OutputOutcome::NotDispatchedPendingTarget => {
                Self::PendingTarget
            }
            OutputOutcome::Rejected | OutputOutcome::NotDispatchedRejected => Self::Rejected,
        }
    }
}

impl OutputState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::Dispatched => "dispatched",
            Self::Confirmed => "confirmed",
            Self::DispatchedOnly => "dispatched_only",
            Self::Uncertain => "uncertain",
            Self::PendingTarget => "pending_target",
            Self::Rejected => "rejected",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "prepared" => Ok(Self::Prepared),
            "dispatched" => Ok(Self::Dispatched),
            "confirmed" => Ok(Self::Confirmed),
            "dispatched_only" => Ok(Self::DispatchedOnly),
            "uncertain" => Ok(Self::Uncertain),
            "pending_target" => Ok(Self::PendingTarget),
            "rejected" => Ok(Self::Rejected),
            _ => Err(OutputLedgerError::CorruptRecord),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputRecord {
    pub intent: OutputIntent,
    pub state: OutputState,
}

/// 用户待处理提示只包含稳定身份和结果，不暴露正文、原字段或来源路径。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputNotice {
    pub operation_id: String,
    pub item_id: String,
    pub state: OutputState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutputNoticePage {
    pub items: Vec<OutputNotice>,
    pub next_cursor: Option<String>,
}

pub const MAX_NOTICE_PAGE_SIZE: u32 = 100;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoverySummary {
    pub uncertain: usize,
    pub rejected: usize,
}

#[derive(Debug)]
pub enum OutputLedgerError {
    Sqlite(rusqlite::Error),
    InvalidIntent,
    ReplayConflict,
    NotFound,
    InvalidTransition,
    CorruptRecord,
}

impl fmt::Display for OutputLedgerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Sqlite(_) => "output ledger database operation failed",
            Self::InvalidIntent => "output intent is invalid",
            Self::ReplayConflict => "output operation identity conflicts",
            Self::NotFound => "output operation does not exist",
            Self::InvalidTransition => "output operation transition is not allowed",
            Self::CorruptRecord => "output ledger record is invalid",
        })
    }
}

impl std::error::Error for OutputLedgerError {}
impl From<rusqlite::Error> for OutputLedgerError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sqlite(value)
    }
}
pub type Result<T> = std::result::Result<T, OutputLedgerError>;

/// 在服务 schema 初始化时安装表；应纳入调用方已有的 schema 事务。
pub fn initialize(connection: &Connection) -> Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS unified_output_operations (
            operation_id TEXT PRIMARY KEY NOT NULL,
            intent_json TEXT NOT NULL,
            intent_digest BLOB NOT NULL CHECK(length(intent_digest) = 32),
            state TEXT NOT NULL CHECK(state IN (
                'prepared', 'dispatched', 'confirmed', 'dispatched_only',
                'uncertain', 'pending_target', 'rejected'
            ))
        );
        CREATE TABLE IF NOT EXISTS unified_output_notice_reads (
            operation_id TEXT NOT NULL REFERENCES unified_output_operations(operation_id),
            state TEXT NOT NULL CHECK(state IN ('uncertain', 'pending_target', 'rejected')),
            intent_digest BLOB NOT NULL CHECK(length(intent_digest) = 32),
            PRIMARY KEY(operation_id, state)
        );
        CREATE INDEX IF NOT EXISTS unified_output_notice_scan
            ON unified_output_operations(operation_id)
            WHERE state IN ('pending_target', 'uncertain', 'rejected');",
    )?;
    Ok(())
}

/// 准备一次显式操作；相同 ID 只允许完整请求相同的重放，不重置其状态。
pub fn prepare(connection: &Connection, intent: &OutputIntent) -> Result<OutputRecord> {
    validate(intent)?;
    let encoded = serde_json::to_string(intent).map_err(|_| OutputLedgerError::InvalidIntent)?;
    connection.execute(
        "INSERT INTO unified_output_operations(operation_id, intent_json, intent_digest, state)
         VALUES (?1, ?2, ?3, 'prepared') ON CONFLICT(operation_id) DO NOTHING",
        params![intent.operation_id, encoded, digest(intent).as_slice()],
    )?;
    matching_record(connection, intent)
}

/// 返回记录，不回显正文；损坏或身份不一致时失败关闭。
pub fn get(connection: &Connection, operation_id: &str) -> Result<Option<OutputRecord>> {
    Identifier::parse(operation_id).map_err(|_| OutputLedgerError::InvalidIntent)?;
    let row: Option<(String, Vec<u8>, String)> = connection
        .query_row(
            "SELECT intent_json, intent_digest, state FROM unified_output_operations
             WHERE operation_id = ?1",
            [operation_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    row.map(|(encoded, stored_digest, state)| {
        let intent: OutputIntent =
            serde_json::from_str(&encoded).map_err(|_| OutputLedgerError::CorruptRecord)?;
        validate(&intent).map_err(|_| OutputLedgerError::CorruptRecord)?;
        if intent.operation_id != operation_id || digest(&intent).as_slice() != stored_digest {
            return Err(OutputLedgerError::CorruptRecord);
        }
        Ok(OutputRecord {
            intent,
            state: OutputState::parse(&state)?,
        })
    })
    .transpose()
}

/// 有界只读查询；游标是上一页末尾的操作 ID，与正文、时间和当前焦点无关。
/// 未知结果只能在此发现，查询绝不准备或重新 claim 输出。
pub fn list_unresolved_notices(
    connection: &Connection,
    cursor: Option<&str>,
    limit: u32,
) -> Result<OutputNoticePage> {
    if limit == 0 || limit > MAX_NOTICE_PAGE_SIZE {
        return Err(OutputLedgerError::InvalidIntent);
    }
    if let Some(cursor) = cursor {
        Identifier::parse(cursor).map_err(|_| OutputLedgerError::InvalidIntent)?;
    }
    let mut statement = connection.prepare(
        "SELECT operation.operation_id FROM unified_output_operations AS operation
         WHERE operation.state IN ('pending_target', 'uncertain', 'rejected')
         AND (?1 IS NULL OR operation.operation_id > ?1)
         AND NOT EXISTS (
             SELECT 1 FROM unified_output_notice_reads AS notice
             WHERE notice.operation_id=operation.operation_id
               AND notice.state=operation.state
               AND notice.intent_digest=operation.intent_digest
         )
         ORDER BY operation.operation_id ASC LIMIT ?2",
    )?;
    let ids = statement
        .query_map(params![cursor, i64::from(limit) + 1], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let has_more = ids.len() > limit as usize;
    let mut items = Vec::with_capacity(ids.len().min(limit as usize));
    for id in ids.into_iter().take(limit as usize) {
        // 沿用账本完整性校验；不得把损坏 intent_json 的任意字段带入 UI。
        let record = get(connection, &id)?.ok_or(OutputLedgerError::CorruptRecord)?;
        if !is_notice_state(record.state) {
            return Err(OutputLedgerError::CorruptRecord);
        }
        items.push(OutputNotice {
            operation_id: record.intent.operation_id,
            item_id: record.intent.item_id,
            state: record.state,
        });
    }
    let next_cursor = if has_more {
        items.last().map(|item| item.operation_id.clone())
    } else {
        None
    };
    Ok(OutputNoticePage { items, next_cursor })
}

/// 只确认用户已阅读特定结果。原输出账本、未知结果和重放禁令均不改变。
/// 调用方使用同一写事务包住状态检查与已读写入，避免确认一个随后变化的状态。
pub fn acknowledge_notice(
    connection: &Connection,
    operation_id: &str,
    expected_state: OutputState,
) -> Result<()> {
    if !is_notice_state(expected_state) {
        return Err(OutputLedgerError::InvalidTransition);
    }
    let record = get(connection, operation_id)?.ok_or(OutputLedgerError::NotFound)?;
    if record.state != expected_state {
        return Err(OutputLedgerError::InvalidTransition);
    }
    connection.execute(
        "INSERT INTO unified_output_notice_reads(operation_id, state, intent_digest)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(operation_id, state) DO UPDATE SET intent_digest=excluded.intent_digest",
        params![
            operation_id,
            expected_state.as_str(),
            digest(&record.intent).as_slice()
        ],
    )?;
    Ok(())
}

fn is_notice_state(state: OutputState) -> bool {
    matches!(
        state,
        OutputState::PendingTarget | OutputState::Uncertain | OutputState::Rejected
    )
}

/// 原子 Prepared → Dispatched；true 仅授予本事务唯一 claim。
/// 必须提交成功后才能派发，数据库忙、回滚或提交失败均不能执行外部副作用。
/// 目标/隐私前置检查必须在调用此函数之前完成；账本本身不验证原生目标。
pub fn claim_dispatch(connection: &Connection, intent: &OutputIntent) -> Result<bool> {
    matching_record(connection, intent)?;
    Ok(connection.execute(
        "UPDATE unified_output_operations SET state = 'dispatched'
         WHERE operation_id = ?1 AND intent_digest = ?2 AND state = 'prepared'",
        params![intent.operation_id, digest(intent).as_slice()],
    )? == 1)
}

/// 写入明确结果或前置失败；同终态重复回执幂等，不同回执冲突不能覆盖。
/// Prepared → PendingTarget/Rejected 表示没有派发；重新插入需要新的用户操作 ID。
/// Dispatched → Confirmed/DispatchedOnly/Uncertain 不允许再 claim 或改所有者。
/// 已 claim 后的最后目标检查失败，可用 NotDispatched* 证明无副作用并结束为
/// PendingTarget/Rejected；不能用于副作用错误、丢失回执、超时或崩溃恢复。
pub fn finish(
    connection: &Connection,
    intent: &OutputIntent,
    outcome: OutputOutcome,
) -> Result<OutputRecord> {
    matching_record(connection, intent)?;
    let state = OutputState::from(outcome);
    let expected = match outcome {
        OutputOutcome::PendingTarget | OutputOutcome::Rejected => OutputState::Prepared,
        _ => OutputState::Dispatched,
    };
    let changed = connection.execute(
        "UPDATE unified_output_operations SET state = ?3
         WHERE operation_id = ?1 AND intent_digest = ?2 AND state = ?4",
        params![
            intent.operation_id,
            digest(intent).as_slice(),
            state.as_str(),
            expected.as_str()
        ],
    )?;
    let record = matching_record(connection, intent)?;
    if changed == 0 && record.state != state {
        return Err(OutputLedgerError::InvalidTransition);
    }
    Ok(record)
}

/// 仅在取得服务独占所有权、尚无活跃派发的启动恢复阶段调用。
/// 已派发无回执 → Uncertain；尚未派发 → Rejected，需用户重新确认新操作。
/// 单条 UPDATE 保证即使不在外层事务中也不会只恢复一半状态。
pub fn recover_inflight(connection: &Connection) -> Result<RecoverySummary> {
    let mut statement = connection.prepare(
        "UPDATE unified_output_operations
         SET state = CASE state WHEN 'dispatched' THEN 'uncertain' ELSE 'rejected' END
         WHERE state IN ('prepared', 'dispatched') RETURNING state",
    )?;
    let states = statement.query_map([], |row| row.get::<_, String>(0))?;
    let mut summary = RecoverySummary::default();
    for state in states {
        match state?.as_str() {
            "uncertain" => summary.uncertain += 1,
            "rejected" => summary.rejected += 1,
            _ => return Err(OutputLedgerError::CorruptRecord),
        }
    }
    Ok(summary)
}

fn matching_record(connection: &Connection, intent: &OutputIntent) -> Result<OutputRecord> {
    validate(intent)?;
    let record = get(connection, &intent.operation_id)?.ok_or(OutputLedgerError::NotFound)?;
    if record.intent != *intent {
        return Err(OutputLedgerError::ReplayConflict);
    }
    Ok(record)
}

fn validate(intent: &OutputIntent) -> Result<()> {
    for value in [Some(&intent.operation_id), intent.target_id.as_ref()]
        .into_iter()
        .flatten()
    {
        Identifier::parse(value.as_str()).map_err(|_| OutputLedgerError::InvalidIntent)?;
    }
    // item_id 是 store 生成的长度前缀复合身份，其组成由 store 核验。
    if intent.item_id.is_empty()
        || intent.item_id.len() > 1024
        || intent.item_id.chars().any(char::is_control)
        || intent.revision == 0
        || (intent.owner == OutputOwner::Ime && intent.action != OutputAction::InsertText)
        || intent.source.as_ref().is_some_and(|value| {
            value.is_empty() || value.len() > 128 || value.chars().any(char::is_control)
        })
        || intent.profile_id.as_ref().is_some_and(|value| {
            value.is_empty() || value.len() > 128 || value.chars().any(char::is_control)
        })
        || intent.deadline_at_ms == Some(0)
    {
        return Err(OutputLedgerError::InvalidIntent);
    }
    Ok(())
}

fn digest(intent: &OutputIntent) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(b"inputia-output-intent-v1\0");
    for value in [&intent.operation_id, &intent.item_id] {
        hash.update((value.len() as u64).to_be_bytes());
        hash.update(value.as_bytes());
    }
    hash.update(intent.revision.to_be_bytes());
    match &intent.target_id {
        Some(value) => {
            hash.update([1]);
            hash.update((value.len() as u64).to_be_bytes());
            hash.update(value.as_bytes());
        }
        None => hash.update([0]),
    }
    hash.update([match intent.owner {
        OutputOwner::Platform => 0,
        OutputOwner::Ime => 1,
    }]);
    hash.update(intent.policy_epoch.to_be_bytes());
    hash.update([match intent.action {
        OutputAction::InsertText => 0,
        OutputAction::PasteAsset => 1,
        OutputAction::Copy => 2,
        OutputAction::CopyPlainText => 3,
    }]);
    // 旧账本的摘要只覆盖 v1 字段。仅当新字段实际存在时追加 v2 扩展，
    // 这样缺失新字段的旧记录仍可按原摘要读取和完成迁移。
    if intent.source.is_some() || intent.profile_id.is_some() || intent.deadline_at_ms.is_some() {
        hash.update(b"\0inputia-output-contract-v2\0");
        for value in [&intent.source, &intent.profile_id] {
            match value {
                Some(value) => {
                    hash.update([1]);
                    hash.update((value.len() as u64).to_be_bytes());
                    hash.update(value.as_bytes());
                }
                None => hash.update([0]),
            }
        }
        match intent.deadline_at_ms {
            Some(value) => {
                hash.update([1]);
                hash.update(value.to_be_bytes());
            }
            None => hash.update([0]),
        }
    }
    hash.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_frames_all_fields_including_operation_identity() {
        let request = OutputIntent {
            operation_id: "ab".into(),
            item_id: "c".into(),
            revision: 1,
            source: None,
            profile_id: None,
            deadline_at_ms: None,
            target_id: None,
            owner: OutputOwner::Platform,
            policy_epoch: 0,
            action: OutputAction::InsertText,
        };
        let mut changed = request.clone();
        changed.operation_id = "a".into();
        changed.item_id = "bc".into();
        assert_ne!(digest(&request), digest(&changed));
        let mut changed = request.clone();
        changed.operation_id.push('c');
        assert_ne!(digest(&request), digest(&changed));
        let mut changed = request.clone();
        changed.target_id = Some(String::new());
        assert_ne!(digest(&request), digest(&changed));
        let mut changed = request.clone();
        changed.action = OutputAction::Copy;
        assert_ne!(digest(&request), digest(&changed));
        let mut changed = request.clone();
        changed.source = Some("voice".into());
        assert_ne!(digest(&request), digest(&changed));
        let mut changed = request.clone();
        changed.profile_id = Some("default".into());
        assert_ne!(digest(&request), digest(&changed));
        let mut changed = request.clone();
        changed.deadline_at_ms = Some(123);
        assert_ne!(digest(&request), digest(&changed));
        let encoded = serde_json::to_string(&request).unwrap();
        let decoded: OutputIntent = serde_json::from_str(&encoded).unwrap();
        assert_eq!(digest(&request), digest(&decoded));

        let legacy = r#"{"operation_id":"ab","item_id":"c","revision":1,"target_id":null,"owner":"platform","policy_epoch":0,"action":"insert_text"}"#;
        let decoded: OutputIntent = serde_json::from_str(legacy).unwrap();
        assert_eq!(decoded.source, None);
        assert_eq!(decoded.profile_id, None);
        assert_eq!(decoded.deadline_at_ms, None);
        assert_eq!(digest(&request), digest(&decoded));
    }
}
