//! 仅收录已经确认上屏的正文；采集世代隔离关闭、删除与迟到重放。
use inputia_core::{AppContext, AppPolicy};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, String>;
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
/// 收录权限及客户端队列必须携带的世代。
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct TypedPolicy {
    pub enabled: bool,
    pub epoch: u64,
}
fn readonly(root: &Path) -> Result<Option<Connection>> {
    let path = root.join("knowledge/typed.sqlite");
    if !path.exists() {
        return Ok(None);
    }
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(err)?;
    db.busy_timeout(std::time::Duration::from_secs(2))
        .map_err(err)?;
    Ok(Some(db))
}
fn writer(root: &Path) -> Result<Connection> {
    std::fs::create_dir_all(root.join("knowledge")).map_err(err)?;
    let db = Connection::open(root.join("knowledge/typed.sqlite")).map_err(err)?;
    db.busy_timeout(std::time::Duration::from_secs(2))
        .map_err(err)?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS typed_policy(singleton INTEGER PRIMARY KEY CHECK(singleton=1),enabled INTEGER NOT NULL,epoch INTEGER NOT NULL); INSERT OR IGNORE INTO typed_policy VALUES(1,0,1); CREATE TABLE IF NOT EXISTS typed_segments(id TEXT PRIMARY KEY,segment_id TEXT NOT NULL,source_app TEXT NOT NULL,text TEXT NOT NULL,revision INTEGER NOT NULL,created_at_ms INTEGER NOT NULL,updated_at_ms INTEGER NOT NULL); CREATE TABLE IF NOT EXISTS typed_events(event_id TEXT PRIMARY KEY,fingerprint TEXT NOT NULL,item_id TEXT NOT NULL,revision INTEGER NOT NULL); CREATE INDEX IF NOT EXISTS typed_recent ON typed_segments(updated_at_ms DESC);").map_err(err)?;
    Ok(db)
}
fn read_policy(db: &Connection) -> Result<TypedPolicy> {
    db.query_row(
        "SELECT enabled,epoch FROM typed_policy WHERE singleton=1",
        [],
        |r| {
            Ok(TypedPolicy {
                enabled: r.get(0)?,
                epoch: r.get(1)?,
            })
        },
    )
    .map_err(err)
}
/// 不创建数据库；首次使用默认关闭。
pub fn policy(root: &Path) -> Result<TypedPolicy> {
    match readonly(root)? {
        Some(db) => read_policy(&db),
        None => Ok(TypedPolicy {
            enabled: false,
            epoch: 1,
        }),
    }
}
/// 收录开关变化时更新世代，使排队的旧正文失效。
pub fn set_capture(root: &Path, enabled: bool) -> Result<TypedPolicy> {
    let mut db = writer(root)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(err)?;
    let before = read_policy(&tx)?;
    if before.enabled != enabled {
        bump(&tx)?;
        tx.execute(
            "UPDATE typed_policy SET enabled=?1 WHERE singleton=1",
            [enabled],
        )
        .map_err(err)?;
    }
    let result = read_policy(&tx)?;
    tx.commit().map_err(err)?;
    Ok(result)
}
fn bump(db: &Connection) -> Result<()> {
    let epoch = read_policy(db)?.epoch;
    if epoch >= i64::MAX as u64 {
        return Err("收录世代已耗尽".into());
    }
    db.execute(
        "UPDATE typed_policy SET epoch=epoch+1 WHERE singleton=1",
        [],
    )
    .map_err(err)?;
    Ok(())
}
fn digest(s: &str) -> String {
    format!("{:x}", Sha256::digest(s.as_bytes()))
}
fn safe_app(source: &str) -> bool {
    !source.trim().is_empty() && !AppPolicy::default().excludes(&AppContext::new(source))
}
/// 认证输入服务器提交的追加正文。相同事件幂等，跨应用和跨世代不会合并。
pub fn record(
    root: &Path,
    event_id: &str,
    segment_id: &str,
    text: &str,
    source_app: &str,
    expected_epoch: u64,
) -> Result<Value> {
    let before = policy(root)?;
    if !before.enabled || before.epoch != expected_epoch {
        return Err("键入收录已关闭或事件世代已过期".into());
    }
    if event_id.is_empty()
        || event_id.len() > 256
        || segment_id.is_empty()
        || segment_id.len() > 256
        || source_app.len() > 512
    {
        return Err("键入事件标识无效".into());
    }
    if text.is_empty() || text.chars().count() > 8192 || text.contains('\0') {
        return Err("键入片段为空或超过8192字符".into());
    }
    if !safe_app(source_app) {
        return Err("此应用不允许收录键入正文".into());
    }
    let mut db = writer(root)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(err)?;
    let current = read_policy(&tx)?;
    if !current.enabled || current.epoch != expected_epoch {
        return Err("键入收录已关闭或事件世代已过期".into());
    }
    let fingerprint = digest(&json!([segment_id, text, source_app, expected_epoch]).to_string());
    let previous: Option<(String, String, i64)> = tx
        .query_row(
            "SELECT fingerprint,item_id,revision FROM typed_events WHERE event_id=?1",
            [event_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(err)?;
    if let Some((old, id, revision)) = previous {
        if old != fingerprint {
            return Err("事件ID已用于不同正文".into());
        }
        return Ok(
            json!({"id":id,"revision":revision.to_string(),"duplicate":true,"epoch":current.epoch}),
        );
    }
    let id = format!(
        "history:typed:{}",
        digest(&json!([expected_epoch, source_app, segment_id]).to_string())
    );
    let old: Option<(String, i64)> = tx
        .query_row(
            "SELECT text,revision FROM typed_segments WHERE id=?1",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(err)?;
    let (mut full, revision) = old.unwrap_or((String::new(), 0));
    if full.chars().count() + text.chars().count() > 32768 {
        return Err("键入段落超过32768字符，请开始新段落".into());
    }
    full.push_str(text);
    let revision = revision.checked_add(1).ok_or("修订号溢出")?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(err)?
        .as_millis()
        .min(i64::MAX as u128) as i64;
    tx.execute("INSERT INTO typed_segments VALUES(?1,?2,?3,?4,?5,?6,?6) ON CONFLICT(id) DO UPDATE SET text=excluded.text,revision=excluded.revision,updated_at_ms=excluded.updated_at_ms",params![id,segment_id,source_app,full,revision,now]).map_err(err)?;
    tx.execute(
        "INSERT INTO typed_events VALUES(?1,?2,?3,?4)",
        params![event_id, fingerprint, id, revision],
    )
    .map_err(err)?;
    tx.commit().map_err(err)?;
    Ok(json!({"id":id,"revision":revision.to_string(),"duplicate":false,"epoch":current.epoch}))
}
fn item(r: &rusqlite::Row<'_>, offset: u64) -> rusqlite::Result<Value> {
    let id: String = r.get(0)?;
    let revision: i64 = r.get(1)?;
    let text: String = r.get(2)?;
    let app: String = r.get(3)?;
    let time: i64 = r.get(4)?;
    let total: u64 = r.get(5)?;
    let next = offset + text.chars().count() as u64;
    Ok(
        json!({"id":id,"revision":revision.to_string(),"source_id":"history:saved_snippet","title":format!("{app} · {:02}:{:02} UTC", (time / 3_600_000) % 24, (time / 60_000) % 60),"text":text,"locator":format!("inputia://typed/{id}"),"kind":"saved_snippet","source_app":app,"created_at_ms":time,"offset":offset,"total_chars":total,"truncated":next<total,"next_offset":if next<total{Some(next)}else{None}}),
    )
}
/// 查询真实键入库，返回有界结果与是否还有匹配记录。
pub fn search(root: &Path, query: &str, limit: usize) -> Result<(Vec<Value>, bool)> {
    if query.len() > 4096 || query.split_whitespace().count() > 32 {
        return Err("查询过长".into());
    }
    let Some(db) = readonly(root)? else {
        return Ok((vec![], false));
    };
    let mut sql="SELECT id,revision,substr(text,1,1200),source_app,created_at_ms,length(text) FROM typed_segments WHERE 1=1".to_string();
    let mut args: Vec<rusqlite::types::Value> = vec![];
    for token in query.split_whitespace() {
        sql.push_str(" AND instr(lower(source_app || ' ' || text),lower(?))>0");
        args.push(token.to_owned().into());
    }
    sql.push_str(" ORDER BY updated_at_ms DESC,id LIMIT 1001");
    let mut stmt = db.prepare(&sql).map_err(err)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(args), |r| item(r, 0))
        .map_err(err)?;
    let mut found = Vec::new();
    for (scanned, row) in rows.enumerate() {
        if scanned == 1000 {
            return Ok((found, true));
        }
        let row = row.map_err(err)?;
        if !safe_app(row["source_app"].as_str().unwrap_or("")) {
            continue;
        }
        if found.len() == limit.clamp(1, 100) {
            return Ok((found, true));
        }
        found.push(row);
    }
    Ok((found, false))
}
/// 按修订读取，源记录被删除或修改后旧引用立即失效。
pub fn read(root: &Path, id: &str, revision: &str, offset: u64, max_chars: u64) -> Result<Value> {
    if offset > i64::MAX as u64 - 1 {
        return Err("读取偏移越界".into());
    }
    let db = readonly(root)?.ok_or("键入引用不存在")?;
    let row=db.query_row("SELECT id,revision,substr(text,?1,?2),source_app,created_at_ms,length(text) FROM typed_segments WHERE id=?3",params![offset+1,max_chars.clamp(1,8000),id],|r|item(r,offset)).optional().map_err(err)?.ok_or("键入引用不存在")?;
    if row["revision"] != revision || !safe_app(row["source_app"].as_str().unwrap_or("")) {
        return Err("键入引用修订已过期或受隐私策略限制".into());
    }
    Ok(row)
}
/// 当前可读记录数，不读取正文。
pub fn count(root: &Path) -> Result<usize> {
    let Some(db) = readonly(root)? else {
        return Ok(0);
    };
    let mut s = db
        .prepare("SELECT source_app,count(*) FROM typed_segments GROUP BY source_app")
        .map_err(err)?;
    let rows = s
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, usize>(1)?)))
        .map_err(err)?;
    let mut count = 0;
    for row in rows {
        let (app, n) = row.map_err(err)?;
        if safe_app(&app) {
            count += n;
        }
    }
    Ok(count)
}
/// 删除记录并推进采集世代，阻止迟到事件复活已删段落。
pub fn delete(root: &Path, id: &str) -> Result<Value> {
    if !id.starts_with("history:typed:") {
        return Err("键入引用ID无效".into());
    }
    mutate_delete(root, Some(id))
}
/// 清空正文并推进采集世代，保留事件去重标识。
pub fn clear(root: &Path) -> Result<Value> {
    mutate_delete(root, None)
}
fn mutate_delete(root: &Path, id: Option<&str>) -> Result<Value> {
    let mut db = writer(root)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(err)?;
    let deleted = if let Some(id) = id {
        tx.execute("DELETE FROM typed_segments WHERE id=?1", [id])
    } else {
        tx.execute("DELETE FROM typed_segments", [])
    }
    .map_err(err)?;
    bump(&tx)?;
    let policy = read_policy(&tx)?;
    tx.commit().map_err(err)?;
    Ok(json!({"deleted":deleted,"capture_enabled":policy.enabled,"capture_epoch":policy.epoch}))
}
