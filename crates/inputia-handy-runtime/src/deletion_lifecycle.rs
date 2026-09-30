//! 跨库删除意图日志：只保存身份、修订与阶段，不保存正文或附件路径。
//! 源事务回执证明源库提交；墓碑及派生数据核验后才记录投影撤销。
//! projection_revoked 只覆盖源记录和统一投影，绝不表示附件或其他学习域已遗忘。

use crate::store::{StoreError, StoreResult};
use inputia_core::integration::events::Identifier;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const DELETE_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeleteRequest {
    pub schema_version: u32,
    pub operation_id: String,
    pub item_id: String,
    pub store_id: String,
    pub logical_name: String,
    pub record_id: String,
    pub expected_revision: u64,
}

impl DeleteRequest {
    pub fn validate(&self) -> StoreResult<()> {
        if self.schema_version != DELETE_SCHEMA_VERSION {
            return Err(StoreError::Invalid("unsupported deletion request version"));
        }
        for id in [&self.operation_id, &self.store_id, &self.record_id] {
            Identifier::parse(id).map_err(|_| StoreError::Invalid("deletion identifier"))?;
        }
        if !matches!(self.logical_name.as_str(), "history" | "clipboard")
            || self.item_id != crate::store::item_id(&self.store_id, &self.record_id)
            || self.expected_revision == 0
            || self.expected_revision >= i64::MAX as u64
        {
            return Err(StoreError::Invalid("deletion identity or revision"));
        }
        Ok(())
    }

    fn digest(&self) -> StoreResult<Vec<u8>> {
        self.validate()?;
        let mut hash = Sha256::new();
        hash.update(b"inputia-delete-request-v1\0");
        hash.update(serde_json::to_vec(self)?);
        Ok(hash.finalize().to_vec())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeleteState {
    Requested,
    SourceApplied,
    ProjectionRevoked,
    Rejected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentCleanup {
    NotStarted,
    Scheduled,
    Completed,
    Blocked,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeleteFailure {
    SourceIdentity,
    SourceRevision,
    OperationConflict,
    SourceUnavailable,
    SourceChangedAfterCommit,
    ProjectionUnavailable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DeleteRecord {
    pub request: DeleteRequest,
    pub state: DeleteState,
    pub attachment_cleanup: AttachmentCleanup,
    pub last_failure: Option<DeleteFailure>,
}

pub(crate) fn initialize(conn: &Connection) -> StoreResult<()> {
    let schema:Option<String>=conn.query_row("SELECT sql FROM sqlite_master WHERE type='table' AND name='integration_deletion_operations'",[],|r|r.get(0)).optional()?;
    let upgrade = schema.is_some_and(|sql| !sql.contains("'scheduled'"));
    if upgrade {
        conn.execute_batch("ALTER TABLE integration_deletion_operations RENAME TO integration_deletion_operations_v1;")?;
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS integration_deletion_operations(
            operation_id TEXT PRIMARY KEY,
            request_digest BLOB NOT NULL CHECK(length(request_digest)=32),
            request_json TEXT NOT NULL,
            state TEXT NOT NULL CHECK(state IN('requested','source_applied','projection_revoked','rejected')),
            attachment_cleanup TEXT NOT NULL CHECK(attachment_cleanup IN('not_started','scheduled','completed','blocked')),
            last_failure TEXT
        );",
    )?;
    if upgrade {
        conn.execute_batch("INSERT INTO integration_deletion_operations SELECT * FROM integration_deletion_operations_v1;DROP TABLE integration_deletion_operations_v1;")?;
    }
    conn.execute_batch("CREATE INDEX IF NOT EXISTS integration_deletion_pending ON integration_deletion_operations(state,operation_id);")?;
    Ok(())
}

pub(crate) fn get(conn: &Connection, operation_id: &str) -> StoreResult<Option<DeleteRecord>> {
    let row: Option<(Vec<u8>, String, String, String, Option<String>)> = conn
        .query_row(
            "SELECT request_digest,request_json,state,attachment_cleanup,last_failure
         FROM integration_deletion_operations WHERE operation_id=?1",
            [operation_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()?;
    row.map(
        |(digest, request_json, state, attachment_cleanup, failure)| {
            let request: DeleteRequest = serde_json::from_str(&request_json)?;
            if request.operation_id != operation_id || request.digest()? != digest {
                return Err(StoreError::Invalid("deletion request digest conflict"));
            }
            Ok(DeleteRecord {
                request,
                state: serde_json::from_value(serde_json::Value::String(state))?,
                attachment_cleanup: serde_json::from_value(serde_json::Value::String(
                    attachment_cleanup,
                ))?,
                last_failure: failure
                    .map(|s| serde_json::from_value(serde_json::Value::String(s)))
                    .transpose()?,
            })
        },
    )
    .transpose()
}

/// 调用方在同一事务内提交日志和学习世代撤销，再执行源副作用。
pub(crate) fn prepare(conn: &Connection, request: &DeleteRequest) -> StoreResult<DeleteRecord> {
    let digest = request.digest()?;
    if let Some(prior) = get(conn, &request.operation_id)? {
        if prior.request != *request {
            return Err(StoreError::Invalid(
                "deletion operation identifier conflict",
            ));
        }
        return Ok(prior);
    }
    conn.execute("INSERT INTO integration_deletion_operations(operation_id,request_digest,request_json,state,attachment_cleanup)
        VALUES(?1,?2,?3,'requested','not_started')",
        params![request.operation_id,digest,serde_json::to_string(request)?])?;
    conn.execute("UPDATE integration_meta SET value=CAST(value AS INTEGER)+1 WHERE key='learning_generation'", [])?;
    Ok(DeleteRecord {
        request: request.clone(),
        state: DeleteState::Requested,
        attachment_cleanup: AttachmentCleanup::NotStarted,
        last_failure: None,
    })
}

pub(crate) fn pending(
    conn: &Connection,
    after: Option<&str>,
    limit: u32,
) -> StoreResult<Vec<DeleteRecord>> {
    let mut query = conn.prepare(
        "SELECT operation_id FROM integration_deletion_operations
        WHERE state IN('requested','source_applied') AND (?2 IS NULL OR operation_id>?2)
        ORDER BY operation_id LIMIT ?1",
    )?;
    let ids = query
        .query_map(params![limit.clamp(1, 64), after], |row| {
            row.get::<_, String>(0)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    ids.iter()
        .map(|id| get(conn, id)?.ok_or(StoreError::Invalid("deletion request disappeared")))
        .collect()
}

pub(crate) fn has_pending(conn: &Connection) -> StoreResult<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM integration_deletion_operations
        WHERE state IN('requested','source_applied'))",
        [],
        |row| row.get(0),
    )?)
}

/// 启动审计包含终态，不能让一个裸 phase 字符串绕过外部提交证据。
pub(crate) fn audit_page(conn: &Connection, after: Option<&str>) -> StoreResult<Vec<DeleteRecord>> {
    let mut query = conn.prepare(
        "SELECT operation_id FROM integration_deletion_operations
        WHERE (?1 IS NULL OR operation_id>?1) ORDER BY operation_id LIMIT 32",
    )?;
    let ids = query
        .query_map([after], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    ids.iter()
        .map(|id| get(conn, id)?.ok_or(StoreError::Invalid("deletion request disappeared")))
        .collect()
}

pub(crate) fn transition(
    conn: &Connection,
    request: &DeleteRequest,
    next: DeleteState,
    failure: Option<DeleteFailure>,
) -> StoreResult<DeleteRecord> {
    let current =
        get(conn, &request.operation_id)?.ok_or(StoreError::Invalid("deletion request missing"))?;
    if current.request != *request {
        return Err(StoreError::Invalid(
            "deletion operation identifier conflict",
        ));
    }
    if current.state != next
        && !matches!(
            (current.state, next),
            (DeleteState::Requested, DeleteState::SourceApplied)
                | (DeleteState::SourceApplied, DeleteState::ProjectionRevoked)
                | (DeleteState::Requested, DeleteState::Rejected)
        )
    {
        return Err(StoreError::Invalid("deletion transition"));
    }
    if next == DeleteState::Rejected
        && !matches!(
            failure,
            Some(DeleteFailure::SourceRevision | DeleteFailure::OperationConflict)
        )
    {
        return Err(StoreError::Invalid(
            "deletion rejection needs a definite pre-commit conflict",
        ));
    }
    let result = DeleteRecord {
        state: next,
        last_failure: failure,
        ..current.clone()
    };
    if result != current {
        let state = serde_json::to_value(next)?;
        let failure = failure.map(serde_json::to_value).transpose()?;
        conn.execute("UPDATE integration_deletion_operations SET state=?2,last_failure=?3 WHERE operation_id=?1",
            params![request.operation_id,state.as_str(),failure.as_ref().and_then(serde_json::Value::as_str)])?;
    }
    Ok(result)
}
