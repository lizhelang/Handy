//! 历史知识只读视图；引用必须在源库仍存在且修订一致，共享设置独立保存。
use inputia_core::{AppContext, AppPolicy};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde_json::{json, Value};
use std::path::Path;

type Result<T> = std::result::Result<T, String>;
const KINDS: [&str; 3] = ["voice", "clipboard", "saved_snippet"];
const SCAN_LIMIT: usize = 1000;
const READ_LIMIT: u64 = 8000;
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn readonly(path: &Path) -> Result<Option<Connection>> {
    if !path.is_file() {
        return Ok(None);
    }
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(err)?;
    db.busy_timeout(std::time::Duration::from_millis(250))
        .map_err(err)?;
    Ok(Some(db))
}
fn access(root: &Path, kind: &str) -> Result<(bool, bool)> {
    let Some(db) = readonly(&root.join("knowledge/history-access.sqlite"))? else {
        return Ok((true, false));
    };
    db.query_row(
        "SELECT enabled,external_access FROM history_access WHERE kind=?1",
        [kind],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()
    .map(|v| v.unwrap_or((true, false)))
    .map_err(err)
}
fn allowed(root: &Path, kind: &str, external: bool) -> Result<bool> {
    let (enabled, shared) = access(root, kind)?;
    Ok(enabled && (!external || shared))
}
fn kind_from_source(id: &str) -> Option<&str> {
    id.strip_prefix("history:").filter(|k| KINDS.contains(k))
}
fn warning(warnings: &mut Vec<Value>, message: &str) {
    let v = json!(message);
    if !warnings.contains(&v) {
        warnings.push(v);
    }
}

struct Row {
    id: String,
    store: String,
    record: String,
    revision: i64,
    kind: String,
    text: String,
    title: String,
    app: String,
    text_len: u64,
}
fn read_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Row> {
    Ok(Row {
        id: r.get(0)?,
        store: r.get(1)?,
        record: r.get(2)?,
        revision: r.get(3)?,
        kind: r.get(4)?,
        text: r.get(5)?,
        title: r.get(6)?,
        app: r.get(7)?,
        text_len: r.get(8)?,
    })
}
// 所有正文通过 SQL 有界读取；计数扫描仅请求零字符。
const SELECT: &str = "SELECT item_id,store_id,record_id,revision,source_kind,substr(COALESCE(json_extract(snapshot,'$.text'),''),?1,?2),substr(COALESCE(json_extract(snapshot,'$.title'),''),1,300),COALESCE(json_extract(snapshot,'$.source_app'),''),length(COALESCE(json_extract(snapshot,'$.text'),'')) FROM integration_items";
fn current(root: &Path, row: &Row) -> Result<bool> {
    let (file, table, logical) = match row.kind.as_str() {
        "voice" => ("history.db", "transcription_history", "history"),
        "clipboard" => ("clipboard.db", "clipboard_history", "clipboard"),
        _ => return Err("保存片段尚无可验证源库，已跳过".into()),
    };
    if AppPolicy::default().excludes(&AppContext::new(&row.app)) {
        return Ok(false);
    }
    let Some(db) = readonly(&root.join(file))? else {
        return Err("历史源库不可用，已跳过相应引用".into());
    };
    // 在同一个读事务内核对源身份、版本及原记录。
    db.execute_batch("BEGIN").map_err(err)?;
    let meta: Option<(i64,String,String)> = db.query_row("SELECT schema_version,store_id,logical_name FROM unified_source_meta WHERE singleton=1", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|_| "历史源库版本未知，已跳过相应引用".to_string())?;
    let Some((schema, store, name)) = meta else {
        return Err("历史源库缺少身份信息，已跳过相应引用".into());
    };
    if schema != crate::source::SOURCE_SCHEMA_VERSION || name != logical {
        return Err("历史源库版本未知，已跳过相应引用".into());
    }
    if store != row.store {
        return Ok(false);
    }
    let revision: Option<i64> = db.query_row(&format!("SELECT v.revision FROM unified_source_versions v JOIN {table} s ON CAST(s.id AS TEXT)=v.record_id WHERE v.record_id=?1"), [&row.record], |r| r.get(0)).optional().map_err(|_| "历史源库结构未知，已跳过相应引用".to_string())?;
    Ok(revision == Some(row.revision))
}
fn item(row: &Row, offset: u64) -> Value {
    let returned = row.text.chars().count() as u64;
    json!({"id":format!("history:{}",row.id),"revision":row.revision.to_string(),"source_id":format!("history:{}",row.kind),"title":row.title,"text":row.text,"locator":format!("inputia://history/{}",row.id),"kind":row.kind,"offset":offset,"total_chars":row.text_len,"truncated":offset+returned<row.text_len,"next_offset":if offset+returned<row.text_len {Some(offset+returned)} else {None}})
}
fn scan(
    root: &Path,
    kind: &str,
    query: &str,
    limit: usize,
    count_only: bool,
    warnings: &mut Vec<Value>,
) -> Result<(Vec<Value>, usize, bool)> {
    if kind == "saved_snippet" {
        if count_only {
            return Ok((vec![], crate::typed_history::count(root)?, false));
        }
        let (items, more) = crate::typed_history::search(root, query, limit)?;
        let count = items.len();
        return Ok((items, count, more));
    }
    let Some(db) = readonly(&root.join("integration.db"))? else {
        warning(warnings, "统一历史库尚未建立");
        return Ok((vec![], 0, false));
    };
    if query.len() > 4096 || query.split_whitespace().count() > 32 {
        return Err("查询过长".into());
    }
    let mut sql = format!("{SELECT} WHERE source_kind=?3 AND content_type='text'");
    let mut args: Vec<rusqlite::types::Value> = vec![
        1i64.into(),
        (if count_only { 0i64 } else { 1200i64 }).into(),
        kind.to_owned().into(),
    ];
    for token in query.split_whitespace() {
        sql.push_str(" AND instr(lower(COALESCE(json_extract(snapshot,'$.title'),'') || ' ' || COALESCE(json_extract(snapshot,'$.text'),'')),lower(?))>0");
        args.push(token.to_owned().into());
    }
    sql.push_str(" ORDER BY created_at_ms DESC,item_id LIMIT ?");
    args.push(((SCAN_LIMIT + 1) as i64).into());
    let mut stmt = match db.prepare(&sql) {
        Ok(s) => s,
        Err(_) => {
            warning(warnings, "统一历史库结构未知，已跳过");
            return Ok((vec![], 0, false));
        }
    };
    let rows = stmt
        .query_map(rusqlite::params_from_iter(args), read_row)
        .map_err(err)?;
    let mut items = Vec::new();
    let mut count = 0;
    for (scanned, row) in rows.enumerate() {
        if scanned == SCAN_LIMIT {
            warning(
                warnings,
                "历史候选超过单次扫描上限，结果不完整，请缩小查询范围",
            );
            return Ok((items, count, true));
        }
        let row = row.map_err(err)?;
        match current(root, &row) {
            Ok(true) => {
                count += 1;
                if !count_only {
                    if items.len() == limit {
                        return Ok((items, count, true));
                    }
                    items.push(item(&row, 0));
                }
            }
            Ok(false) => warning(warnings, "部分历史引用已变化、删除或受隐私策略限制，已跳过"),
            Err(e) => warning(warnings, &e),
        }
    }
    Ok((items, count, false))
}
/// 优先处理历史来源配置和历史引用读取；不属于历史的操作返回 None。
pub fn handle(root: &Path, action: &str, payload: &Value, external: bool) -> Result<Option<Value>> {
    let id = payload.get("id").and_then(Value::as_str).unwrap_or("");
    if matches!(action, "typed_capture" | "delete_typed" | "clear_typed") {
        if external {
            return Err("外部接口不能修改键入收录或删除记录".into());
        }
        return match action {
            "typed_capture" => Ok(Some(
                serde_json::to_value(crate::typed_history::set_capture(
                    root,
                    payload
                        .get("enabled")
                        .and_then(Value::as_bool)
                        .ok_or("enabled必须是布尔值")?,
                )?)
                .map_err(err)?,
            )),
            "delete_typed" => Ok(Some(crate::typed_history::delete(root, id)?)),
            _ => Ok(Some(crate::typed_history::clear(root)?)),
        };
    }
    if action == "read" && id.starts_with("history:typed:") {
        if !allowed(root, "saved_snippet", external)? {
            return Err("引用不存在或未授权读取".into());
        }
        return Ok(Some(crate::typed_history::read(
            root,
            id,
            payload
                .get("revision")
                .and_then(Value::as_str)
                .ok_or("读取需要revision")?,
            payload.get("offset").and_then(Value::as_u64).unwrap_or(0),
            payload
                .get("max_chars")
                .and_then(Value::as_u64)
                .unwrap_or(READ_LIMIT),
        )?));
    }
    if matches!(action, "update_source" | "remove_source") {
        let Some(kind) = kind_from_source(id) else {
            return Ok(None);
        };
        if external {
            return Err("外部知识接口不能修改历史共享设置".into());
        }
        let (mut enabled, mut shared) = access(root, kind)?;
        for (key, target) in [("enabled", &mut enabled), ("external_access", &mut shared)] {
            if let Some(v) = payload.get(key) {
                *target = v.as_bool().ok_or("来源开关必须是布尔值")?;
            }
        }
        if action == "remove_source" {
            enabled = false;
            shared = false;
        }
        std::fs::create_dir_all(root.join("knowledge")).map_err(err)?;
        let db = Connection::open(root.join("knowledge/history-access.sqlite")).map_err(err)?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS history_access(kind TEXT PRIMARY KEY,enabled INTEGER NOT NULL,external_access INTEGER NOT NULL)").map_err(err)?;
        db.execute("INSERT INTO history_access VALUES(?1,?2,?3) ON CONFLICT(kind) DO UPDATE SET enabled=excluded.enabled,external_access=excluded.external_access",params![kind,enabled,shared]).map_err(err)?;
        return Ok(Some(
            json!({"id":id,"enabled":enabled,"external_access":shared,"removed":if action=="remove_source" {Some(id)} else {None}}),
        ));
    }
    if action != "read" || !id.starts_with("history:") {
        return Ok(None);
    }
    let expected = payload
        .get("revision")
        .and_then(Value::as_str)
        .ok_or("读取历史引用需要 revision")?;
    let offset = payload.get("offset").and_then(Value::as_u64).unwrap_or(0);
    if offset > i64::MAX as u64 - 1 {
        return Err("读取偏移越界".into());
    }
    let max = payload
        .get("max_chars")
        .and_then(Value::as_u64)
        .unwrap_or(READ_LIMIT)
        .clamp(1, READ_LIMIT);
    let db = readonly(&root.join("integration.db"))?.ok_or("引用不存在或未授权读取")?;
    let sql = format!("{SELECT} WHERE item_id=?3 AND content_type='text'");
    let row = db
        .query_row(&sql, params![offset + 1, max, &id[8..]], read_row)
        .optional()
        .map_err(err)?
        .ok_or("引用不存在或未授权读取")?;
    if !KINDS.contains(&row.kind.as_str()) || !allowed(root, &row.kind, external)? {
        return Err("引用不存在或未授权读取".into());
    }
    if expected != row.revision.to_string() || !current(root, &row)? {
        return Err("引用已过期、删除或受隐私策略限制，请重新搜索".into());
    }
    Ok(Some(item(&row, offset)))
}
/// 在文件知识结果上合并已授权的历史来源，保持统一响应结构。
pub fn augment(
    root: &Path,
    action: &str,
    payload: &Value,
    external: bool,
    mut data: Value,
) -> Result<Value> {
    if !matches!(action, "status" | "sources" | "search") {
        return Ok(data);
    }
    let mut warnings = data
        .get("warnings")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if matches!(action, "status" | "sources") {
        let sources = data
            .as_object_mut()
            .ok_or("知识响应不是对象")?
            .entry("sources")
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .ok_or("知识来源不是数组")?;
        for kind in KINDS {
            let (enabled, shared) = access(root, kind)?;
            if external && (!enabled || !shared) {
                continue;
            }
            let (_, count, incomplete) = if enabled {
                scan(root, kind, "", SCAN_LIMIT, true, &mut warnings)?
            } else {
                (vec![], 0, false)
            };
            sources.push(json!({"id":format!("history:{kind}"),"name":match kind {"voice"=>"语音历史","clipboard"=>"剪贴板历史",_=>"保存片段"},"kind":kind,"path":"","enabled":enabled,"external_access":shared,"status":if !enabled {"disabled"} else if incomplete||!warnings.is_empty() {"warning"} else {"ready"},"item_count":if incomplete {None} else {Some(count)}}));
            if kind == "saved_snippet" {
                let policy = crate::typed_history::policy(root)?;
                if let Some(source) = sources.last_mut() {
                    source["capture_enabled"] = json!(policy.enabled);
                    source["capture_epoch"] = json!(policy.epoch);
                    source["name"] = json!("键入正文");
                    source["status"] = json!(if enabled { "ready" } else { "disabled" });
                }
            }
        }
    } else {
        let query = payload
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        let limit = payload
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(20)
            .clamp(1, 100) as usize;
        let source = payload.get("source_id").and_then(Value::as_str);
        let file_items = data
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut has_more = data
            .get("has_more")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let mut groups = vec![file_items];
        for kind in KINDS {
            if source.is_some_and(|s| s != format!("history:{kind}"))
                || !allowed(root, kind, external)?
            {
                continue;
            }
            let (rows, _, more) = scan(root, kind, query, limit, false, &mut warnings)?;
            groups.push(rows);
            has_more |= more;
        }
        let available: usize = groups.iter().map(Vec::len).sum();
        let mut items = Vec::new();
        let mut iterators: Vec<_> = groups.into_iter().map(Vec::into_iter).collect();
        while items.len() < limit {
            let before = items.len();
            for group in &mut iterators {
                if let Some(item) = group.next() {
                    items.push(item);
                }
                if items.len() == limit {
                    break;
                }
            }
            if before == items.len() {
                break;
            }
        }
        has_more |= available > items.len();
        data["items"] = json!(items);
        data["has_more"] = json!(has_more);
    }
    data["warnings"] = json!(warnings);
    Ok(data)
}
