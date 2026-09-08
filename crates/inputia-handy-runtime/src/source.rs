//! 在现有源数据库内安装事务 outbox，不依赖临时 UI 事件或时间戳轮询。

use crate::store::{ItemSnapshot, SourceChange, SourceOperation};
use inputia_core::integration::events::Identifier;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fmt, time::Duration};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HistoryPatch {
    pub starred: Option<bool>,
    pub pinned: Option<bool>,
    pub title: Option<String>,
    pub clear_title: bool,
    pub text: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceTable {
    History,
    Clipboard,
}

impl SourceTable {
    pub fn logical_name(self) -> &'static str {
        match self {
            Self::History => "history",
            Self::Clipboard => "clipboard",
        }
    }

    fn table(self) -> &'static str {
        match self {
            Self::History => "transcription_history",
            Self::Clipboard => "clipboard_history",
        }
    }

    fn snapshot_sql(self, row: &str) -> String {
        // row 仅由模块内固定的 NEW 或 source_row 提供，不接受外部 SQL。
        match self {
            Self::History => format!(
                "json_object('source_kind','voice','content_type','text',
                 'text',COALESCE((SELECT text_override FROM unified_source_annotations WHERE record_id=CAST({row}.id AS TEXT)),NULLIF({row}.post_processed_text,''),{row}.transcription_text),
                 'title',{row}.title,'starred',json(CASE WHEN {row}.saved THEN 'true' ELSE 'false' END),
                 'pinned',json(CASE WHEN COALESCE((SELECT pinned FROM unified_source_annotations WHERE record_id=CAST({row}.id AS TEXT)),0) THEN 'true' ELSE 'false' END),'created_at_ms',{row}.timestamp * 1000,
                 'asset_ref',NULLIF({row}.file_name,''),'source_app',NULL,'source_trust','unknown')"
            ),
            Self::Clipboard => format!(
                "json_object('source_kind','clipboard',
                 'content_type',CASE {row}.content_type WHEN 'image' THEN 'image' WHEN 'file' THEN 'files'
                     WHEN 'richtext' THEN 'html' ELSE 'text' END,
                 'text',{row}.full_text,'title',{row}.title,
                 'starred',json(CASE WHEN {row}.is_favorite THEN 'true' ELSE 'false' END),
                 'pinned',json(CASE WHEN {row}.is_pinned THEN 'true' ELSE 'false' END),
                 'created_at_ms',{row}.created_at * 1000,'asset_ref',{row}.image_path,
                 'source_app',{row}.source_app,'source_trust','unknown')"
            ),
        }
    }
}

#[derive(Debug)]
pub enum SourceError {
    Sqlite(rusqlite::Error),
    Json(serde_json::Error),
    IncompatibleSchema,
    WrongSource,
    InvalidIdentifier,
    InvalidAcknowledgement,
    PolicyRegression,
    OperationConflict,
    SnapshotRequired { acknowledged_sequence: u64 },
    CursorAhead { maximum_sequence: u64 },
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(f, "source database: {error}"),
            Self::Json(_) => f.write_str("invalid source snapshot"),
            Self::IncompatibleSchema => f.write_str("unsupported source outbox schema"),
            Self::WrongSource => f.write_str("source database identity mismatch"),
            Self::InvalidIdentifier => f.write_str("invalid source operation identifier"),
            Self::InvalidAcknowledgement => {
                f.write_str("outbox acknowledgement exceeds produced sequence")
            }
            Self::PolicyRegression => f.write_str("source policy epoch cannot go backwards"),
            Self::OperationConflict => {
                f.write_str("operation identifier reused for a different mutation")
            }
            Self::SnapshotRequired {
                acknowledged_sequence,
            } => write!(
                f,
                "source snapshot required after acknowledged sequence {acknowledged_sequence}"
            ),
            Self::CursorAhead { maximum_sequence } => write!(
                f,
                "consumer cursor is ahead of source sequence {maximum_sequence}"
            ),
        }
    }
}

impl std::error::Error for SourceError {}
impl From<rusqlite::Error> for SourceError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sqlite(value)
    }
}
impl From<serde_json::Error> for SourceError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

pub type Result<T> = std::result::Result<T, SourceError>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotHeader {
    pub store_id: String,
    pub through_sequence: u64,
    pub policy_epoch: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotRecord {
    pub record_id: String,
    pub revision: u64,
    /// None 是源端持久保留的删除记录，不是未读到正文。
    pub payload: Option<ItemSnapshot>,
}

/// 在 with_snapshot 的一致性读事务内分批扫描，禁止跨回调保留连接。
pub struct SnapshotView<'a> {
    connection: &'a Connection,
    source: SourceTable,
    pub header: SnapshotHeader,
}

impl SnapshotView<'_> {
    pub fn read_page(
        &self,
        after_record_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SnapshotRecord>> {
        let table = self.source.table();
        let snapshot = self.source.snapshot_sql("source_row");
        let mut query = self.connection.prepare(&format!(
            "SELECT version.record_id,version.revision,
                CASE WHEN source_row.id IS NULL THEN NULL ELSE {snapshot} END
             FROM unified_source_versions version
             LEFT JOIN {table} source_row ON source_row.id=CAST(version.record_id AS INTEGER)
             WHERE version.record_id>?1 ORDER BY version.record_id LIMIT ?2"
        ))?;
        let rows = query.query_map(
            params![after_record_id.unwrap_or(""), limit.min(2_000) as i64],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, u64>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            },
        )?;
        let mut records = Vec::new();
        for row in rows {
            let (record_id, revision, payload) = row?;
            records.push(SnapshotRecord {
                record_id,
                revision,
                payload: payload.map(|p| serde_json::from_str(&p)).transpose()?,
            });
        }
        Ok(records)
    }
}

/// 每个源数据库仅有一个身份，重建库必须显式创建新身份并执行映射对账。
pub struct SourceOutbox {
    store_id: String,
}

impl SourceOutbox {
    /// 调用者须先完成原业务表迁移和备份；不改变其 user_version。
    pub fn install(conn: &mut Connection, source: SourceTable) -> Result<Self> {
        conn.busy_timeout(Duration::from_millis(250))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS unified_source_meta (
                singleton INTEGER PRIMARY KEY CHECK(singleton=1), schema_version INTEGER NOT NULL,
                store_id TEXT NOT NULL, logical_name TEXT NOT NULL, policy_epoch INTEGER NOT NULL,
                acknowledged_sequence INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE IF NOT EXISTS unified_source_versions (
                record_id TEXT PRIMARY KEY, revision INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS unified_source_annotations (
                record_id TEXT PRIMARY KEY,pinned INTEGER NOT NULL DEFAULT 0,text_override TEXT);
             CREATE TABLE IF NOT EXISTS unified_source_outbox (
                seq INTEGER PRIMARY KEY AUTOINCREMENT, event_id TEXT NOT NULL UNIQUE,
                record_id TEXT NOT NULL, revision INTEGER NOT NULL, operation TEXT NOT NULL,
                policy_epoch INTEGER NOT NULL, payload TEXT);
             CREATE TABLE IF NOT EXISTS unified_source_operations (
                operation_id TEXT PRIMARY KEY, request_digest TEXT NOT NULL, response TEXT NOT NULL);"
        )?;
        let existing = tx.query_row(
            "SELECT schema_version,store_id,logical_name FROM unified_source_meta WHERE singleton=1",
            [], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)),
        ).optional()?;
        let fresh = existing.is_none();
        if let Some((schema, _, name)) = &existing {
            if ![1, 2, 3].contains(schema) {
                return Err(SourceError::IncompatibleSchema);
            }
            if name != source.logical_name() {
                return Err(SourceError::WrongSource);
            }
        } else {
            tx.execute(
                "INSERT INTO unified_source_meta(singleton,schema_version,store_id,logical_name,policy_epoch)
                 VALUES(1,3,lower(hex(randomblob(16))),?1,1)", [source.logical_name()],
            )?;
        }
        let has_override:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('unified_source_annotations') WHERE name='text_override')",[],|row|row.get(0))?;
        if !has_override {
            tx.execute(
                "ALTER TABLE unified_source_annotations ADD COLUMN text_override TEXT",
                [],
            )?;
        }
        let table = source.table();
        for (suffix, event, row, operation) in [
            ("insert", "INSERT", "NEW", "upsert"),
            ("update", "UPDATE", "NEW", "upsert"),
            ("delete", "DELETE", "OLD", "delete"),
        ] {
            let snapshot = if operation == "delete" {
                "NULL".into()
            } else {
                source.snapshot_sql(row)
            };
            let clear_old_override = if operation == "delete" {
                "DELETE FROM unified_source_annotations WHERE record_id=CAST(OLD.id AS TEXT);"
            } else if source == SourceTable::History {
                "UPDATE unified_source_annotations SET text_override=NULL WHERE record_id=CAST(NEW.id AS TEXT) AND text_override IS NOT NEW.post_processed_text;"
            } else {
                ""
            };
            tx.execute_batch(&format!(
                "DROP TRIGGER IF EXISTS unified_{table}_{suffix};
                 CREATE TRIGGER unified_{table}_{suffix} AFTER {event} ON {table}
                 BEGIN
                   {clear_old_override}
                   INSERT INTO unified_source_versions(record_id,revision) VALUES(CAST({row}.id AS TEXT),1)
                   ON CONFLICT(record_id) DO UPDATE SET revision=revision+1;
                   INSERT INTO unified_source_outbox(event_id,record_id,revision,operation,policy_epoch,payload)
                   SELECT store_id || ':' || lower(hex(randomblob(16))),CAST({row}.id AS TEXT),
                     (SELECT revision FROM unified_source_versions WHERE record_id=CAST({row}.id AS TEXT)),
                     '{operation}',policy_epoch,{snapshot} FROM unified_source_meta WHERE singleton=1;
                 END;"
            ))?;
        }
        if fresh {
            // 同一事务捕获现有快照并安装触发器，扫描期间不会遗漏源表变更。
            let snapshot = source.snapshot_sql("source_row");
            let timestamp = match source {
                SourceTable::History => "timestamp",
                SourceTable::Clipboard => "created_at",
            };
            tx.execute_batch(&format!(
                "INSERT INTO unified_source_versions(record_id,revision) SELECT CAST(id AS TEXT),1 FROM {table};
                 INSERT INTO unified_source_outbox(event_id,record_id,revision,operation,policy_epoch,payload)
                 SELECT meta.store_id || ':' || lower(hex(randomblob(16))),CAST(source_row.id AS TEXT),1,
                     'upsert',meta.policy_epoch,{snapshot}
                 FROM {table} source_row CROSS JOIN unified_source_meta meta
                 ORDER BY source_row.{timestamp} DESC, source_row.id DESC;"
            ))?;
        }
        let store_id = tx.query_row(
            "SELECT store_id FROM unified_source_meta WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        tx.execute(
            "UPDATE unified_source_meta SET schema_version=3 WHERE singleton=1",
            [],
        )?;
        tx.commit()?;
        Ok(Self { store_id })
    }

    pub fn store_id(&self) -> &str {
        &self.store_id
    }

    /// 每批有界读取，继续使用返回的 seq 直到耗尽，不永久截断历史。
    pub fn read_batch(
        &self,
        conn: &Connection,
        after_seq: u64,
        limit: usize,
    ) -> Result<Vec<SourceChange>> {
        let tx = conn.unchecked_transaction()?;
        self.verify_identity(&tx)?;
        let (acknowledged, maximum): (u64,u64) = tx.query_row(
            "SELECT acknowledged_sequence,max(acknowledged_sequence,COALESCE((SELECT MAX(seq) FROM unified_source_outbox),0)) FROM unified_source_meta WHERE singleton=1",
            [],
            |r| Ok((r.get(0)?,r.get(1)?)),
        )?;
        if after_seq > maximum {
            return Err(SourceError::CursorAhead {
                maximum_sequence: maximum,
            });
        }
        if after_seq < acknowledged {
            return Err(SourceError::SnapshotRequired {
                acknowledged_sequence: acknowledged,
            });
        }
        let mut query = tx.prepare(
            "SELECT seq,event_id,record_id,revision,operation,policy_epoch,payload
             FROM unified_source_outbox WHERE seq>?1 ORDER BY seq LIMIT ?2",
        )?;
        let rows = query.query_map(params![after_seq, limit.min(2_000) as i64], |r| {
            Ok((
                r.get::<_, u64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, u64>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, u64>(5)?,
                r.get::<_, Option<String>>(6)?,
            ))
        })?;
        let mut events = Vec::new();
        for row in rows {
            let (seq, event_id, record_id, revision, operation, policy_epoch, payload) = row?;
            let operation = match operation.as_str() {
                "upsert" => SourceOperation::Upsert,
                "delete" => SourceOperation::Delete,
                _ => return Err(SourceError::IncompatibleSchema),
            };
            events.push(SourceChange {
                store_id: self.store_id.clone(),
                seq,
                event_id,
                record_id,
                revision,
                operation,
                policy_epoch,
                payload: payload
                    .map(|p| serde_json::from_str::<ItemSnapshot>(&p))
                    .transpose()?,
            });
        }
        Ok(events)
    }

    /// 索引/游标丢失时获取完整当前状态和删除记录，消费者提交快照后才推进水位。
    pub fn with_snapshot<T>(
        &self,
        conn: &mut Connection,
        source: SourceTable,
        read: impl FnOnce(&SnapshotView<'_>) -> Result<T>,
    ) -> Result<T> {
        let tx = conn.transaction()?;
        self.verify_identity(&tx)?;
        let (name,through_sequence,policy_epoch): (String,u64,u64) = tx.query_row(
            "SELECT logical_name,max(acknowledged_sequence,COALESCE((SELECT MAX(seq) FROM unified_source_outbox),0)),policy_epoch
             FROM unified_source_meta WHERE singleton=1", [], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))
        )?;
        if name != source.logical_name() {
            return Err(SourceError::WrongSource);
        }
        let view = SnapshotView {
            connection: &tx,
            source,
            header: SnapshotHeader {
                store_id: self.store_id.clone(),
                through_sequence,
                policy_epoch,
            },
        };
        read(&view)
    }

    fn verify_identity(&self, conn: &Connection) -> Result<()> {
        let id: String = conn.query_row(
            "SELECT store_id FROM unified_source_meta WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        if id != self.store_id {
            return Err(SourceError::WrongSource);
        }
        Ok(())
    }

    /// 只能在规范索引事务完成后调用；这里仅回收已确认事件正文。
    pub fn acknowledge(&self, conn: &mut Connection, sequence: u64) -> Result<()> {
        self.verify_identity(conn)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let maximum: u64 = tx.query_row(
            "SELECT max(acknowledged_sequence,COALESCE((SELECT MAX(seq) FROM unified_source_outbox),0))
             FROM unified_source_meta WHERE singleton=1", [], |r| r.get(0),
        )?;
        if sequence > maximum {
            return Err(SourceError::InvalidAcknowledgement);
        }
        tx.execute("UPDATE unified_source_meta SET acknowledged_sequence=max(acknowledged_sequence,?1) WHERE singleton=1", [sequence])?;
        tx.execute(
            "DELETE FROM unified_source_outbox WHERE seq<=?1",
            [sequence],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn advance_policy(&self, conn: &Connection, epoch: u64) -> Result<()> {
        self.verify_identity(conn)?;
        let changed = conn.execute(
            "UPDATE unified_source_meta SET policy_epoch=?1 WHERE singleton=1 AND policy_epoch<=?1",
            [epoch],
        )?;
        if changed == 0 {
            return Err(SourceError::PolicyRegression);
        }
        Ok(())
    }

    /// 修改源表并由事务触发器生成事件；同操作重试返回原修订号，不重复改文。
    pub fn update_record(
        &self,
        conn: &mut Connection,
        source: SourceTable,
        record_id: &str,
        expected_revision: u64,
        operation_id: &str,
        patch: &HistoryPatch,
    ) -> Result<MutationResult> {
        Identifier::parse(record_id).map_err(|_| SourceError::InvalidIdentifier)?;
        if patch
            .title
            .as_ref()
            .is_some_and(|s| s.chars().count() > 256)
            || patch
                .text
                .as_ref()
                .is_some_and(|s| s.len() > 2 * 1024 * 1024 || s.contains('\0'))
            || (patch.clear_title && patch.title.is_some())
        {
            return Err(SourceError::InvalidIdentifier);
        }
        let digest = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&(
                source.logical_name(),
                record_id,
                expected_revision,
                patch
            ))?)
        );
        self.mutate_once(conn,operation_id,&digest,|tx| {
            let logical:String=tx.query_row("SELECT logical_name FROM unified_source_meta",[],|row|row.get(0))?;
            if logical!=source.logical_name(){return Err(rusqlite::Error::InvalidQuery);}
            let revision:u64=tx.query_row("SELECT revision FROM unified_source_versions WHERE record_id=?1",[record_id],|row|row.get(0))?;
            if revision!=expected_revision{return Err(rusqlite::Error::QueryReturnedNoRows);}
            let changed=match source {
                SourceTable::History=> {
                    if let Some(text)=&patch.text {
                        tx.execute("INSERT INTO unified_source_annotations(record_id,text_override) VALUES(?1,?2) ON CONFLICT(record_id) DO UPDATE SET text_override=excluded.text_override",params![record_id,text])?;
                    }
                    if let Some(pinned)=patch.pinned {
                        tx.execute("INSERT INTO unified_source_annotations(record_id,pinned) VALUES(?1,?2) ON CONFLICT(record_id) DO UPDATE SET pinned=excluded.pinned",params![record_id,pinned])?;
                    }
                    tx.execute("UPDATE transcription_history SET saved=COALESCE(?1,saved),title=CASE WHEN ?2 THEN '' ELSE COALESCE(?3,title) END,post_processed_text=COALESCE(?4,post_processed_text) WHERE id=CAST(?5 AS INTEGER)",params![patch.starred,patch.clear_title,patch.title,patch.text,record_id])?
                },
                SourceTable::Clipboard=> {
                    if let Some(text)=&patch.text {
                        let kind:String=tx.query_row("SELECT content_type FROM clipboard_history WHERE id=CAST(?1 AS INTEGER)",[record_id],|row|row.get(0))?;
                        if kind!="text" && kind!="richtext" {return Err(rusqlite::Error::InvalidQuery);}
                        let hash=format!("{:x}",Sha256::digest(text.as_bytes()));
                        let preview=text.chars().take(200).collect::<String>();
                        tx.execute("UPDATE clipboard_history SET full_text=?1,content_preview=?2,content_hash=?3,size_bytes=?4,is_favorite=COALESCE(?5,is_favorite),is_pinned=COALESCE(?6,is_pinned),title=CASE WHEN ?7 THEN NULL ELSE COALESCE(?8,title) END WHERE id=CAST(?9 AS INTEGER)",params![text,preview,hash,text.len() as i64,patch.starred,patch.pinned,patch.clear_title,patch.title,record_id])?
                    } else {
                        tx.execute("UPDATE clipboard_history SET is_favorite=COALESCE(?1,is_favorite),is_pinned=COALESCE(?2,is_pinned),title=CASE WHEN ?3 THEN NULL ELSE COALESCE(?4,title) END WHERE id=CAST(?5 AS INTEGER)",params![patch.starred,patch.pinned,patch.clear_title,patch.title,record_id])?
                    }
                }
            };
            if changed!=1{return Err(rusqlite::Error::QueryReturnedNoRows);}
            let revision:u64=tx.query_row("SELECT revision FROM unified_source_versions WHERE record_id=?1",[record_id],|row|row.get(0))?;
            Ok(revision.to_string())
        })
    }

    /// 删除记录与附件生命周期独立；本事务绝不删除 WAV、图片或用户文件。
    pub fn delete_record(
        &self,
        conn: &mut Connection,
        source: SourceTable,
        record_id: &str,
        expected_revision: u64,
        operation_id: &str,
    ) -> Result<MutationResult> {
        Identifier::parse(record_id).map_err(|_| SourceError::InvalidIdentifier)?;
        let item_id = crate::store::item_id(&self.store_id, record_id);
        let logical: String = conn.query_row(
            "SELECT logical_name FROM unified_source_meta WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        if logical != source.logical_name() {
            return Err(SourceError::WrongSource);
        }
        let digest = delete_digest(&item_id, expected_revision);
        self.mutate_once(conn, operation_id, &digest, |tx| {
            let logical: String = tx.query_row(
                "SELECT logical_name FROM unified_source_meta WHERE singleton=1",
                [],
                |r| r.get(0),
            )?;
            if logical != source.logical_name() {
                return Err(rusqlite::Error::InvalidQuery);
            }
            let revision: u64 = tx.query_row(
                "SELECT revision FROM unified_source_versions WHERE record_id=?1",
                [record_id],
                |r| r.get(0),
            )?;
            if revision != expected_revision {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            let changed = tx.execute(
                &format!("DELETE FROM {} WHERE CAST(id AS TEXT)=?1", source.table()),
                [record_id],
            )?;
            if changed != 1 {
                return Err(rusqlite::Error::QueryReturnedNoRows);
            }
            Ok("true".into())
        })
    }

    /// 删除后的投影已不存在，仍可按原始请求身份查源事务回执。
    pub fn delete_receipt(
        &self,
        conn: &Connection,
        item_id: &str,
        revision: u64,
        operation_id: &str,
    ) -> Result<Option<bool>> {
        self.verify_identity(conn)?;
        Identifier::parse(operation_id).map_err(|_| SourceError::InvalidIdentifier)?;
        let prior: Option<(String,String)> = conn.query_row("SELECT request_digest,response FROM unified_source_operations WHERE operation_id=?1", [operation_id], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
        match prior {
            None => Ok(None),
            Some((digest, response))
                if digest == delete_digest(item_id, revision) && response == "true" =>
            {
                Ok(Some(true))
            }
            Some(_) => Err(SourceError::OperationConflict),
        }
    }

    /// 业务修改与去重回执在同一源事务；重复请求不再执行 toggle 等非幂等动作。
    pub fn mutate_once<F>(
        &self,
        conn: &mut Connection,
        operation_id: &str,
        request_digest: &str,
        change: F,
    ) -> Result<MutationResult>
    where
        F: FnOnce(&Transaction<'_>) -> rusqlite::Result<String>,
    {
        Identifier::parse(operation_id).map_err(|_| SourceError::InvalidIdentifier)?;
        self.verify_identity(conn)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let prior = tx.query_row("SELECT request_digest,response FROM unified_source_operations WHERE operation_id=?1",
            [operation_id], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()?;
        if let Some((digest, response)) = prior {
            if digest != request_digest {
                return Err(SourceError::OperationConflict);
            }
            return Ok(MutationResult {
                replayed: true,
                response,
            });
        }
        let response = change(&tx)?;
        tx.execute("INSERT INTO unified_source_operations(operation_id,request_digest,response) VALUES(?1,?2,?3)", params![operation_id,request_digest,response])?;
        tx.commit()?;
        Ok(MutationResult {
            replayed: false,
            response,
        })
    }
}

fn delete_digest(item_id: &str, revision: u64) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!("delete-record:{item_id}:{revision}"))
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MutationResult {
    pub replayed: bool,
    pub response: String,
}
