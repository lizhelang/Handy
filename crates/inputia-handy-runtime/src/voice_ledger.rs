//! 会话请求的持久执行资格与事实投影。录音生命周期只由Coordinator决定。
//! 调用方必须在外部副作用前提交claim所在事务；本模块不执行录音或输出。

use crate::voice_protocol::{VoiceCommand, VoicePeer, VoicePhase, VoiceRequest, VoiceSessionView};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

#[derive(Debug)]
pub enum VoiceLedgerError {
    Sqlite(rusqlite::Error),
    Invalid,
    Conflict,
    Missing,
    Corrupt,
    Retired,
}
impl From<rusqlite::Error> for VoiceLedgerError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}
impl std::fmt::Display for VoiceLedgerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Sqlite(_) => "voice ledger database error",
            Self::Invalid => "invalid voice request",
            Self::Conflict => "voice request identity conflict",
            Self::Missing => "voice session missing",
            Self::Corrupt => "voice ledger corrupt",
            Self::Retired => "voice session belongs to a retired service",
        })
    }
}
impl std::error::Error for VoiceLedgerError {}
type Result<T> = std::result::Result<T, VoiceLedgerError>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionRecord {
    pub start: VoiceRequest,
    pub view: VoiceSessionView,
    pub retired: bool,
}

pub fn initialize(conn: &Connection) -> Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS unified_voice_peers(
      client_instance TEXT PRIMARY KEY, audit_identity BLOB NOT NULL CHECK(length(audit_identity)=32));")?;
    conn.execute_batch("CREATE TABLE IF NOT EXISTS unified_voice_sessions(
      session_id TEXT PRIMARY KEY, client_instance TEXT NOT NULL, server_instance TEXT NOT NULL,
      start_json TEXT NOT NULL, start_digest TEXT NOT NULL, start_claimed INTEGER NOT NULL DEFAULT 0,
      view_json TEXT NOT NULL, view_generation INTEGER NOT NULL DEFAULT 0,
      retired INTEGER NOT NULL DEFAULT 0 CHECK(retired IN(0,1)));
      CREATE TABLE IF NOT EXISTS unified_voice_requests(
      client_instance TEXT NOT NULL, request_id TEXT NOT NULL, session_id TEXT NOT NULL,
      request_digest TEXT NOT NULL, claimed INTEGER NOT NULL DEFAULT 0 CHECK(claimed IN(0,1)),
      PRIMARY KEY(client_instance,request_id), FOREIGN KEY(session_id) REFERENCES unified_voice_sessions(session_id));")?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS unified_voice_results(
      session_id TEXT PRIMARY KEY REFERENCES unified_voice_sessions(session_id),
      operation_id TEXT NOT NULL UNIQUE REFERENCES unified_output_operations(operation_id));",
    )?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS unified_voice_cancellations(
      session_id TEXT PRIMARY KEY REFERENCES unified_voice_sessions(session_id));",
    )?;
    Ok(())
}

/// audit由原生认证从当前socket内核凭据提供；不能从握手或请求正文取值。
/// 绑定跨服务重启保留，不驱逐旧身份，防止复用实例名接管持久会话。
pub fn bind_peer(conn: &Connection, client: &str, audit: &[u8; 32]) -> Result<()> {
    if client.is_empty()
        || client.len() > 256
        || client.chars().any(char::is_control)
        || audit.iter().all(|byte| *byte == 0)
    {
        return Err(VoiceLedgerError::Invalid);
    }
    let previous: Option<Vec<u8>> = conn
        .query_row(
            "SELECT audit_identity FROM unified_voice_peers WHERE client_instance=?1",
            [client],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(previous) = previous {
        return if previous.as_slice() == audit {
            Ok(())
        } else {
            Err(VoiceLedgerError::Conflict)
        };
    }
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM unified_voice_peers", [], |row| {
        row.get(0)
    })?;
    if count >= 16_384 {
        return Err(VoiceLedgerError::Invalid);
    }
    conn.execute(
        "INSERT INTO unified_voice_peers(client_instance,audit_identity) VALUES(?1,?2)",
        params![client, audit.as_slice()],
    )?;
    Ok(())
}

fn encode<T: serde::Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(|_| VoiceLedgerError::Invalid)
}
fn digest(request: &VoiceRequest, semantic_start: bool) -> Result<String> {
    let mut request = request.clone();
    if semantic_start {
        request.request_id.clear();
    }
    let mut hash = Sha256::new();
    hash.update(b"handy-voice-request-v1\0");
    hash.update(encode(&request)?.as_bytes());
    Ok(format!("{:x}", hash.finalize()))
}

pub fn get(conn: &Connection, session_id: &str) -> Result<Option<SessionRecord>> {
    let stored: Option<(String,String,String,i64,i64)> = conn.query_row(
        "SELECT start_json,start_digest,view_json,view_generation,retired FROM unified_voice_sessions WHERE session_id=?1",
        [session_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?))).optional()?;
    let Some((raw, hash, view, generation, retired)) = stored else {
        return Ok(None);
    };
    let start: VoiceRequest = serde_json::from_str(&raw).map_err(|_| VoiceLedgerError::Corrupt)?;
    let view: VoiceSessionView =
        serde_json::from_str(&view).map_err(|_| VoiceLedgerError::Corrupt)?;
    if start.session_id != session_id
        || view.session_id != session_id
        || start.start_identity().is_none()
        || generation < 0
        || view.generation != generation as u64
        || digest(&start, true)? != hash
        || !(0..=1).contains(&retired)
    {
        return Err(VoiceLedgerError::Corrupt);
    }
    Ok(Some(SessionRecord {
        start,
        view,
        retired: retired == 1,
    }))
}

/// peer由认证连接提供，不能用请求自报字段建立peer。新的Start还需要当前策略屏障。
pub fn prepare(
    conn: &Connection,
    request: &VoiceRequest,
    peer: &VoicePeer<'_>,
) -> Result<SessionRecord> {
    request
        .validate_for(peer)
        .map_err(|_| VoiceLedgerError::Invalid)?;
    let hash = digest(request, false)?;
    let previous:Option<String>=conn.query_row("SELECT request_digest FROM unified_voice_requests WHERE client_instance=?1 AND request_id=?2",params![request.client_instance,request.request_id],|r|r.get(0)).optional()?;
    if previous.as_ref().is_some_and(|old| old != &hash) {
        return Err(VoiceLedgerError::Conflict);
    }
    let existing = get(conn, &request.session_id)?;
    match (&request.command, existing.as_ref()) {
        (_, None) if request.strict_start_identity().is_some() => {
            let (target, _) = request
                .strict_start_identity()
                .ok_or(VoiceLedgerError::Invalid)?;
            let view = VoiceSessionView {
                session_id: request.session_id.clone(),
                generation: 0,
                phase: VoicePhase::Preparing,
                target_id: Some(target.target_id.clone()),
                item_id: None,
                output_operation_id: None,
            };
            conn.execute("INSERT INTO unified_voice_sessions(session_id,client_instance,server_instance,start_json,start_digest,view_json) VALUES(?1,?2,?3,?4,?5,?6)",params![request.session_id,request.client_instance,request.server_instance,encode(request)?,digest(request,true)?,encode(&view)?])?;
        }
        (_, Some(record)) if request.strict_start_identity().is_some() => {
            if digest(&record.start, true)? != digest(request, true)? {
                return Err(VoiceLedgerError::Conflict);
            }
        }
        (_, Some(record)) => {
            if record.start.client_instance != request.client_instance {
                return Err(VoiceLedgerError::Conflict);
            }
            // 新服务允许本人查询/停止已中断旧session，但claim绝不会再执行旧动作。
            if !record.retired && record.start.server_instance != request.server_instance {
                return Err(VoiceLedgerError::Conflict);
            }
        }
        (_, None) => return Err(VoiceLedgerError::Missing),
    }
    conn.execute("INSERT OR IGNORE INTO unified_voice_requests(client_instance,request_id,session_id,request_digest) VALUES(?1,?2,?3,?4)",params![request.client_instance,request.request_id,request.session_id,hash])?;
    get(conn, &request.session_id)?.ok_or(VoiceLedgerError::Missing)
}

/// false只允许查询已有事实，不能再次调用Start。Status本身永不取得副作用资格。
pub fn claim(conn: &Connection, request: &VoiceRequest) -> Result<bool> {
    let record = get(conn, &request.session_id)?.ok_or(VoiceLedgerError::Missing)?;
    let hash:Option<String>=conn.query_row("SELECT request_digest FROM unified_voice_requests WHERE client_instance=?1 AND request_id=?2",params![request.client_instance,request.request_id],|r|r.get(0)).optional()?;
    if hash.as_deref() != Some(&digest(request, false)?) {
        return Err(VoiceLedgerError::Conflict);
    }
    if record.retired || matches!(request.command, VoiceCommand::Status) {
        return Ok(false);
    }
    if record.start.client_instance != request.client_instance
        || record.start.server_instance != request.server_instance
    {
        return Err(VoiceLedgerError::Conflict);
    }
    if request.strict_start_identity().is_some() {
        let cancelled: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM unified_voice_cancellations WHERE session_id=?1)",
            [&request.session_id],
            |row| row.get(0),
        )?;
        if cancelled {
            return Ok(false);
        }
        if digest(&record.start, true)? != digest(request, true)? {
            return Err(VoiceLedgerError::Conflict);
        }
        let won=conn.execute("UPDATE unified_voice_sessions SET start_claimed=1 WHERE session_id=?1 AND start_claimed=0 AND retired=0",[&request.session_id])?==1;
        conn.execute("UPDATE unified_voice_requests SET claimed=1 WHERE client_instance=?1 AND request_id=?2",params![request.client_instance,request.request_id])?;
        return Ok(won);
    }
    Ok(conn.execute("UPDATE unified_voice_requests SET claimed=1 WHERE client_instance=?1 AND request_id=?2 AND claimed=0",params![request.client_instance,request.request_id])?==1)
}

/// 事实只从该session原Coordinator实例写入； generation不可回退，同代数内容不能变化。
pub fn project(
    conn: &Connection,
    client: &str,
    server: &str,
    view: &VoiceSessionView,
) -> Result<bool> {
    let record = get(conn, &view.session_id)?.ok_or(VoiceLedgerError::Missing)?;
    if record.retired {
        return Err(VoiceLedgerError::Retired);
    }
    if record.start.client_instance != client || record.start.server_instance != server {
        return Err(VoiceLedgerError::Conflict);
    }
    let claimed: bool = conn.query_row(
        "SELECT start_claimed FROM unified_voice_sessions WHERE session_id=?1",
        [&view.session_id],
        |row| row.get(0),
    )?;
    if !claimed || view.target_id != record.view.target_id {
        return Err(VoiceLedgerError::Invalid);
    }
    if view.generation > i64::MAX as u64 {
        return Err(VoiceLedgerError::Invalid);
    }
    if view.generation < record.view.generation {
        return Ok(false);
    }
    if view.generation == record.view.generation {
        return if view == &record.view {
            Ok(false)
        } else {
            Err(VoiceLedgerError::Conflict)
        };
    }
    conn.execute(
        "UPDATE unified_voice_sessions SET view_json=?1,view_generation=?2 WHERE session_id=?3",
        params![encode(view)?, view.generation as i64, view.session_id],
    )?;
    Ok(true)
}

/// 仅在唯一writer lease取得后、Coordinator启动前调用。不能恢复录音或自动重播Start。
pub fn recover(conn: &Connection) -> Result<usize> {
    let ids = conn
        .prepare("SELECT session_id FROM unified_voice_sessions WHERE retired=0")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for id in &ids {
        let mut record = get(conn, id)?.ok_or(VoiceLedgerError::Missing)?;
        if matches!(
            record.view.phase,
            VoicePhase::Preparing | VoicePhase::Recording | VoicePhase::Processing
        ) {
            record.view.phase = VoicePhase::Interrupted;
            record.view.generation = record
                .view
                .generation
                .checked_add(1)
                .filter(|v| *v <= i64::MAX as u64)
                .ok_or(VoiceLedgerError::Corrupt)?;
        }
        conn.execute("UPDATE unified_voice_sessions SET retired=1,view_json=?1,view_generation=?2 WHERE session_id=?3",params![encode(&record.view)?,record.view.generation as i64,id])?;
    }
    Ok(ids.len())
}
