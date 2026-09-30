//! 本地候选学习。只重排调用方给出的合法候选，学习证据可撤销且与正文收录独立。
use rusqlite::{params, params_from_iter, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
type RootGate = std::sync::Arc<std::sync::Mutex<()>>;
fn root_gate(root: &Path) -> Result<RootGate> {
    use std::sync::{Arc, Mutex, OnceLock, Weak};
    static GATES: OnceLock<Mutex<HashMap<std::path::PathBuf, Weak<Mutex<()>>>>> = OnceLock::new();
    let path = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_owned());
    let mut gates = GATES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(err)?;
    if let Some(gate) = gates.get(&path).and_then(Weak::upgrade) {
        return Ok(gate);
    }
    gates.retain(|_, gate| gate.strong_count() > 0);
    let gate = Arc::new(Mutex::new(()));
    gates.insert(path, Arc::downgrade(&gate));
    Ok(gate)
}

type Result<T> = std::result::Result<T, String>;
const EVIDENCE_QUERY_LIMIT: usize = 4000;
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
fn norm(s: &str) -> String {
    s.trim().to_lowercase()
}
fn digest(s: &str) -> String {
    format!("{:x}", Sha256::digest(s.as_bytes()))
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct LearningPolicy {
    pub enabled: bool,
    pub epoch: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Feedback {
    pub event_id: String,
    pub context_id: String,
    pub input_code: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub schema_id: String,
    pub text: String,
    #[serde(default)]
    pub previous: String,
    #[serde(default)]
    pub explicit_selection: bool,
    #[serde(default)]
    pub original_rank: usize,
    pub source_app: String,
    pub learning_epoch: u64,
    pub operation: String,
}
fn exact() -> String {
    "exact".into()
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub id: String,
    pub text: String,
    pub base_rank: usize,
    pub consumed_len: usize,
    #[serde(default = "exact")]
    pub match_type: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Query {
    #[serde(default)]
    pub input_code: String,
    #[serde(default)]
    pub schema_id: String,
    #[serde(default)]
    pub context: String,
    pub context_id: String,
    pub source_app: String,
    pub learning_epoch: u64,
    #[serde(default)]
    pub candidates: Vec<Candidate>,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

/// 当前候选模型仅供显式实验使用；未验证的模型不能拖慢默认打字路径。
pub fn experimental_candidate_model_enabled(value: Option<&str>) -> bool {
    value == Some("1")
}

/// 验证未来本地模型给出的候选二次排序。
/// 只允许调用方已提供的真实 ID，且不跨 consumed_len/match_type 资格组。
pub fn apply_rerank(candidates: &[Candidate], ordered_ids: &[String]) -> Result<Vec<Candidate>> {
    let by_id: HashMap<&str, &Candidate> = candidates
        .iter()
        .map(|candidate| (candidate.id.as_str(), candidate))
        .collect();
    if ordered_ids.len() > candidates.len()
        || ordered_ids.iter().collect::<HashSet<_>>().len() != ordered_ids.len()
        || ordered_ids
            .iter()
            .any(|id| !by_id.contains_key(id.as_str()))
    {
        return Err("candidate_rerank_identity".into());
    }
    let mut output = candidates.to_vec();
    let mut positions: HashMap<(usize, String), Vec<usize>> = HashMap::new();
    for (index, candidate) in candidates.iter().enumerate() {
        positions
            .entry((candidate.consumed_len, candidate.match_type.clone()))
            .or_default()
            .push(index);
    }
    let mut requested: HashMap<(usize, String), Vec<&Candidate>> = HashMap::new();
    for id in ordered_ids {
        let candidate = by_id[id.as_str()];
        requested
            .entry((candidate.consumed_len, candidate.match_type.clone()))
            .or_default()
            .push(candidate);
    }
    for (group, group_candidates) in requested {
        let slots = positions
            .get(&group)
            .ok_or_else(|| "candidate_rerank_group".to_string())?;
        if group_candidates.len() > slots.len() {
            return Err("candidate_rerank_group_size".into());
        }
        for (slot, candidate) in slots.iter().zip(group_candidates) {
            output[*slot] = candidate.clone();
        }
    }
    Ok(output)
}

/// 显式热词只提升当前原生候选，保持消费长度/匹配类型组及原生身份。
pub fn prioritize_explicit_candidates(
    candidates: &[Candidate],
    ordered_ids: &[String],
    explicit: &[String],
) -> Result<Vec<String>> {
    use inputia_core::integration::terms::{build_hotwords, HotwordBudget};
    let words = build_hotwords(explicit, &[], HotwordBudget::default())
        .map_err(|_| "invalid_explicit_hotwords".to_string())?;
    let priorities: HashMap<_, _> = words
        .iter()
        .enumerate()
        .map(|(index, word)| (norm(word), index))
        .collect();
    let current = apply_rerank(candidates, ordered_ids)?;
    let mut requested = current.clone();
    requested.sort_by_key(|candidate| {
        priorities
            .get(&norm(&candidate.text))
            .copied()
            .unwrap_or(usize::MAX)
    });
    let ids: Vec<_> = requested
        .iter()
        .map(|candidate| candidate.id.clone())
        .collect();
    Ok(apply_rerank(&current, &ids)?
        .into_iter()
        .map(|candidate| candidate.id)
        .collect())
}
fn default_limit() -> usize {
    10
}
fn db(root: &Path) -> Result<Connection> {
    std::fs::create_dir_all(root.join("knowledge")).map_err(err)?;
    let db = Connection::open(root.join("knowledge/personalization.sqlite")).map_err(err)?;
    db.busy_timeout(std::time::Duration::from_secs(2))
        .map_err(err)?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS learning_policy(id INTEGER PRIMARY KEY,enabled INTEGER NOT NULL,epoch INTEGER NOT NULL); INSERT OR IGNORE INTO learning_policy VALUES(1,1,1); CREATE TABLE IF NOT EXISTS evidence(event_id TEXT PRIMARY KEY,fingerprint TEXT NOT NULL,text TEXT NOT NULL,normalized TEXT NOT NULL,previous TEXT NOT NULL,code TEXT NOT NULL,weight REAL NOT NULL,created REAL NOT NULL,undone INTEGER NOT NULL DEFAULT 0,origin TEXT NOT NULL DEFAULT '',origin_revision TEXT NOT NULL DEFAULT ''); CREATE INDEX IF NOT EXISTS evidence_text ON evidence(normalized); CREATE INDEX IF NOT EXISTS evidence_previous ON evidence(previous); CREATE INDEX IF NOT EXISTS evidence_origin ON evidence(origin); CREATE TABLE IF NOT EXISTS forgotten(text TEXT PRIMARY KEY); CREATE TABLE IF NOT EXISTS imports(origin TEXT PRIMARY KEY,item_id TEXT NOT NULL,revision TEXT NOT NULL,kind TEXT NOT NULL); CREATE TABLE IF NOT EXISTS retired_events(event_id TEXT PRIMARY KEY); CREATE TABLE IF NOT EXISTS feedback_bindings(event_id TEXT PRIMARY KEY,binding TEXT NOT NULL); CREATE TABLE IF NOT EXISTS base_phrases(normalized TEXT PRIMARY KEY,text TEXT NOT NULL,frequency REAL NOT NULL); CREATE TABLE IF NOT EXISTS base_meta(id INTEGER PRIMARY KEY,sha256 TEXT NOT NULL); CREATE TABLE IF NOT EXISTS reconcile_cursor(id INTEGER PRIMARY KEY,cursor TEXT NOT NULL);").map_err(err)?;
    // 遗忘屏障只保留本域密钥摘要；升级时同事务迁移旧明文，不再新增词正文副本。
    db.pragma_update(None, "secure_delete", true).map_err(err)?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS privacy_metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL)",
    )
    .map_err(err)?;
    let migrated: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM privacy_metadata WHERE key='forgotten_hash_v1')",
            [],
            |r| r.get(0),
        )
        .map_err(err)?;
    let privacy_key = crate::private_key::load_or_create(
        &root.join("knowledge/personalization-privacy.key"),
        !migrated,
    )
    .map_err(|_| "personalization_privacy_key_unavailable")?;
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, &privacy_key);
    db.create_scalar_function(
        "privacy_term_digest",
        1,
        rusqlite::functions::FunctionFlags::SQLITE_UTF8
            | rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
        move |context| {
            let value: String = context.get(0)?;
            Ok(ring::hmac::sign(&key, norm(&value).as_bytes())
                .as_ref()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>())
        },
    )
    .map_err(err)?;
    if !migrated {
        db.execute_batch("BEGIN IMMEDIATE; UPDATE forgotten SET text=privacy_term_digest(text); INSERT INTO privacy_metadata VALUES('forgotten_hash_v1','1'); COMMIT;").map_err(err)?;
    }
    let has_revision = {
        let mut stmt = db.prepare("PRAGMA table_info(evidence)").map_err(err)?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(1)).map_err(err)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(err)?
            .iter()
            .any(|name| name == "origin_revision")
    };
    if !has_revision {
        db.execute_batch("ALTER TABLE evidence ADD COLUMN origin_revision TEXT NOT NULL DEFAULT ''; UPDATE evidence SET origin_revision=COALESCE((SELECT revision FROM imports WHERE imports.origin=evidence.origin),'') WHERE origin<>'';").map_err(err)?;
    }
    for column in ["source_app", "schema_id"] {
        let present: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('evidence') WHERE name=?1)",
                [column],
                |row| row.get(0),
            )
            .map_err(err)?;
        if !present {
            db.execute_batch(&format!(
                "ALTER TABLE evidence ADD COLUMN {column} TEXT NOT NULL DEFAULT '';"
            ))
            .map_err(err)?;
        }
    }
    db.execute_batch("CREATE INDEX IF NOT EXISTS evidence_schema_code ON evidence(schema_id,code,created DESC); CREATE TABLE IF NOT EXISTS evidence_anchors(event_id TEXT NOT NULL,anchor TEXT NOT NULL,PRIMARY KEY(event_id,anchor)); CREATE INDEX IF NOT EXISTS evidence_anchor_lookup ON evidence_anchors(anchor,event_id); CREATE INDEX IF NOT EXISTS evidence_active_recency ON evidence((origin='') DESC,created DESC) WHERE undone=0;").map_err(err)?;
    Ok(db)
}
fn state(db: &Connection) -> Result<LearningPolicy> {
    db.query_row(
        "SELECT enabled,epoch FROM learning_policy WHERE id=1",
        [],
        |r| {
            Ok(LearningPolicy {
                enabled: r.get(0)?,
                epoch: r.get(1)?,
            })
        },
    )
    .map_err(err)
}
/// 个人学习默认开启；不开启正文收录或外部共享。
pub fn policy(root: &Path) -> Result<LearningPolicy> {
    crate::privacy_operation::ensure_readable(root)?;
    let gate = root_gate(root)?;
    let _guard = gate.lock().map_err(err)?;
    if !root.join("knowledge/personalization.sqlite").exists() {
        return Ok(LearningPolicy {
            enabled: true,
            epoch: 1,
        });
    }
    state(&db(root)?)
}
fn bump(db: &Connection) -> Result<()> {
    if state(db)?.epoch >= i64::MAX as u64 {
        return Err("学习世代已耗尽".into());
    }
    db.execute("UPDATE learning_policy SET epoch=epoch+1 WHERE id=1", [])
        .map_err(err)?;
    Ok(())
}
/// 关闭与重新开启均推进世代，拒绝旧队列。
pub fn set_enabled(root: &Path, enabled: bool) -> Result<LearningPolicy> {
    let gate = root_gate(root)?;
    let _guard = gate.lock().map_err(err)?;
    set_enabled_inner(root, enabled)
}
fn set_enabled_inner(root: &Path, enabled: bool) -> Result<LearningPolicy> {
    let mut db = db(root)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(err)?;
    if state(&tx)?.enabled != enabled {
        bump(&tx)?;
        tx.execute(
            "UPDATE learning_policy SET enabled=?1 WHERE id=1",
            [enabled],
        )
        .map_err(err)?;
    }
    let p = state(&tx)?;
    tx.commit().map_err(err)?;
    Ok(p)
}
fn safe_app(app: &str) -> bool {
    !app.trim().is_empty()
        && app.len() <= 512
        && !inputia_core::AppPolicy::default().excludes(&inputia_core::AppContext::new(app))
}
fn valid_text(s: &str) -> bool {
    !s.trim().is_empty() && s.chars().count() <= 128 && !s.contains('\0')
}
fn valid_schema(schema: &str) -> bool {
    schema.len() <= 128 && !schema.chars().any(char::is_control)
}
fn recall_code(code: &str) -> bool {
    !code.is_empty() && code.is_ascii() && code.bytes().all(|b| b.is_ascii_graphic())
}

/// 拒绝只影响原有码形与语境；跨句式的弱关联不能扩大负反馈范围。
fn rejection_context_matches(context: &str, previous: &str) -> bool {
    if previous.is_empty() {
        context.is_empty()
    } else {
        suffix_context(context, previous)
    }
}

fn anchor_window(context: &str) -> &str {
    let sentence = context
        .rsplit(['。', '！', '？', '.', '!', '?', '\n', ';', '；'])
        .next()
        .unwrap_or("");
    let start = sentence.char_indices().rev().nth(47).map_or(0, |(i, _)| i);
    &sentence[start..]
}

/// 覆盖最后分句的完整 48 字窗口；英文保留词边界，中文使用全部 2–4 字片段。
fn context_anchors(context: &str) -> Vec<(String, f64)> {
    let chars: Vec<_> = anchor_window(context).chars().collect();
    let mut anchors = Vec::new();
    let mut seen = HashSet::new();
    for end in (1..=chars.len()).rev() {
        let distance = chars.len() - end;
        if chars[end - 1].is_ascii_alphanumeric() {
            if end < chars.len() && chars[end].is_ascii_alphanumeric() {
                continue;
            }
            let mut start = end - 1;
            while start > 0 && (chars[start - 1].is_ascii_alphanumeric() || chars[start - 1] == '_')
            {
                start -= 1;
            }
            let word: String = chars[start..end].iter().collect();
            if word.len() >= 2 && seen.insert(word.clone()) {
                anchors.push((word, 1.0 / (1.0 + distance as f64 / 12.0)));
            }
        } else {
            for width in (2..=4).rev() {
                if end < width {
                    continue;
                }
                let span = &chars[end - width..end];
                if !span
                    .iter()
                    .all(|c| matches!(*c as u32, 0x3400..=0x9fff | 0xf900..=0xfaff))
                {
                    continue;
                }
                let word: String = span.iter().collect();
                if seen.insert(word.clone()) {
                    anchors.push((word, (width as f64 / 4.0) / (1.0 + distance as f64 / 12.0)));
                }
            }
        }
    }
    // 48 个连续汉字最多 47+46+45=138 个片段，余量覆盖混合文本的词边界。
    anchors.truncate(144);
    anchors
}

/// 完整语境优先，短后缀和近期词锚点仅提供较弱的可迁移证据。
fn cached_context_affinity(
    context: &str,
    previous: &str,
    current: &HashMap<String, f64>,
    cache: &mut HashMap<String, f64>,
) -> f64 {
    // 完整上下文的精确匹配始终单独判断，不能与只有近句相同的弱关联混用。
    if suffix_context(context, previous) {
        return 1.0;
    }
    if context.is_empty() || previous.is_empty() {
        return 0.0;
    }
    let window = anchor_window(previous);
    *cache.entry(window.to_owned()).or_insert_with(|| {
        context_anchors(window)
            .into_iter()
            .filter_map(|(anchor, weight)| {
                current
                    .get(&anchor)
                    .map(|other| 0.6 * (weight * other).sqrt())
            })
            .fold(0.0, f64::max)
    })
}

/// 每个请求锚点只做一次存在性索引查找，不展开常见词的全部历史 posting。
fn present_anchors(db: &Connection, anchors: &[(String, f64)]) -> Result<Vec<String>> {
    if anchors.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "WITH requested(anchor) AS (VALUES {}) SELECT anchor FROM requested WHERE EXISTS (SELECT 1 FROM evidence_anchors AS existing WHERE existing.anchor=requested.anchor)",
        vec!["(?)"; anchors.len()].join(",")
    );
    let mut stmt = db.prepare(&sql).map_err(err)?;
    let rows = stmt
        .query_map(
            params_from_iter(anchors.iter().map(|(anchor, _)| anchor)),
            |r| r.get(0),
        )
        .map_err(err)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(err)
}

/// 少于预算加一时返回完整有效ID集；达到上限仅说明应采用密集查询计划。
fn bounded_anchor_events(db: &Connection, anchors: &[String]) -> Result<Vec<String>> {
    if anchors.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT DISTINCT indexed.event_id FROM evidence_anchors AS indexed JOIN evidence AS learned ON learned.event_id=indexed.event_id WHERE indexed.anchor IN ({}) AND learned.undone=0 AND privacy_term_digest(learned.normalized) NOT IN(SELECT text FROM forgotten) LIMIT {}",
        vec!["?"; anchors.len()].join(","), EVIDENCE_QUERY_LIMIT + 1
    );
    let mut stmt = db.prepare(&sql).map_err(err)?;
    let rows = stmt
        .query_map(params_from_iter(anchors), |row| row.get(0))
        .map_err(err)?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(err)
}

fn scoped_rejected(db: &Connection, q: &Query, key: &str) -> Result<bool> {
    let mut stmt = db.prepare("SELECT previous,weight FROM evidence WHERE normalized=?1 AND schema_id=?2 AND code=?3 AND (source_app=?4 OR source_app='') AND origin='' AND undone=0 AND created>?5 ORDER BY created DESC LIMIT 100").map_err(err)?;
    let rows = stmt
        .query_map(
            params![
                key,
                q.schema_id,
                q.input_code.clone(),
                q.source_app,
                now() - 14.0 * 86400.0
            ],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)),
        )
        .map_err(err)?;
    for row in rows {
        let (previous, weight) = row.map_err(err)?;
        if rejection_context_matches(&norm(&q.context), &previous) {
            return Ok(weight < 0.0);
        }
    }
    Ok(false)
}

/// 独立的准确码索引使召回与原生候选池无关，准入重查不会受池大小影响。
fn recall_candidates(db: &Connection, q: &Query) -> Result<Vec<Candidate>> {
    #[derive(Default)]
    struct RecallStats {
        evidence: Stats,
        latest_accept: f64,
        latest_reject: f64,
    }
    let code = q.input_code.clone();
    if q.schema_id.is_empty() || !recall_code(&code) {
        return Ok(vec![]);
    }
    let mut stmt = db.prepare("SELECT text,normalized,previous,source_app,weight,created FROM evidence WHERE schema_id=?1 AND code=?2 AND origin='' AND source_app<>'' AND undone=0 AND privacy_term_digest(normalized) NOT IN(SELECT text FROM forgotten) ORDER BY created DESC,event_id LIMIT 1000").map_err(err)?;
    let rows = stmt
        .query_map(params![q.schema_id, code], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, f64>(4)?,
                r.get::<_, f64>(5)?,
            ))
        })
        .map_err(err)?;
    let context = norm(&q.context);
    let current_anchors: HashMap<_, _> = context_anchors(&context).into_iter().collect();
    let timestamp = now();
    let mut stats: HashMap<String, RecallStats> = HashMap::new();
    let mut affinities = HashMap::new();
    for row in rows {
        let (text, key, previous, app, weight, created) = row.map_err(err)?;
        let record = stats.entry(key).or_default();
        let s = &mut record.evidence;
        if weight < 0.0 {
            if app == q.source_app
                && rejection_context_matches(&context, &previous)
                && timestamp - created < 14.0 * 86400.0
            {
                record.latest_reject = record.latest_reject.max(created);
            }
            continue;
        }
        if app == q.source_app && rejection_context_matches(&context, &previous) {
            record.latest_accept = record.latest_accept.max(created);
        }
        if s.text.is_empty() {
            s.text = text;
        }
        let decay = (-((timestamp - created).max(0.0)) / (14.0 * 86400.0)).exp();
        let contribution = weight * decay * if app == q.source_app { 1.0 } else { 0.35 };
        s.frequency += contribution;
        s.context += contribution
            * cached_context_affinity(&context, &previous, &current_anchors, &mut affinities);
    }
    let mut eligible: Vec<_> = stats
        .into_iter()
        .filter(|(_, s)| s.evidence.frequency > 0.0 && s.latest_reject <= s.latest_accept)
        .map(|(key, s)| (key, s.evidence))
        .collect();
    eligible.sort_by(|(left_key, left), (right_key, right)| {
        let strength = |s: &Stats| s.frequency.ln_1p() + 1.5 * s.context.ln_1p();
        strength(right)
            .total_cmp(&strength(left))
            .then_with(|| left_key.cmp(right_key))
    });
    let native: HashSet<_> = q.candidates.iter().map(|c| norm(&c.text)).collect();
    // 先限定个人召回窗口再与原生池去重，Admit 不带原生池时仍能重查相同身份。
    Ok(eligible
        .into_iter()
        .take(5)
        .filter(|(key, _)| !native.contains(key))
        .enumerate()
        .map(|(rank, (key, s))| Candidate {
            id: format!(
                "learned:{}",
                digest(&format!("{}\0{}\0{}", q.schema_id, code, key))
            ),
            text: s.text,
            base_rank: rank,
            consumed_len: q.input_code.len(),
            match_type: "exact".into(),
        })
        .collect())
}
/// 接收经过身份及上屏确认的学习反馈；undo 使用原 accept 的 event_id。
pub fn feedback(root: &Path, f: Feedback) -> Result<Value> {
    crate::privacy_operation::ensure_readable(root)?;
    let gate = root_gate(root)?;
    let _guard = gate.lock().map_err(err)?;
    if f.event_id.is_empty()
        || f.event_id.len() > 256
        || f.context_id.len() > 256
        || f.input_code.len() > 256
        || !valid_schema(&f.schema_id)
        || f.previous.chars().count() > 256
        || !safe_app(&f.source_app)
        || !valid_text(&f.text)
        || f.original_rank > 1000
    {
        return Err("学习反馈字段无效或受隐私策略限制".into());
    }
    if !matches!(f.operation.as_str(), "accept" | "undo" | "reject") {
        return Err("不支持的学习操作".into());
    }
    let mut db = db(root)?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(err)?;
    let p = state(&tx)?;
    if !p.enabled || p.epoch != f.learning_epoch {
        return Err("个人学习已关闭或反馈世代过期".into());
    }
    let text = norm(&f.text);
    let mut binding_fields = json!([
        f.context_id,
        f.input_code,
        f.text,
        f.previous,
        f.source_app,
        f.learning_epoch
    ]);
    if !f.schema_id.is_empty() {
        binding_fields
            .as_array_mut()
            .ok_or("invalid_feedback_binding")?
            .push(json!(f.schema_id));
    }
    let binding = digest(&binding_fields.to_string());
    if f.operation == "undo" {
        let original: Option<String> = tx
            .query_row(
                "SELECT binding FROM feedback_bindings WHERE event_id=?1",
                [&f.event_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        if original
            .as_deref()
            .is_some_and(|original| original != binding)
        {
            return Err("撤销反馈与原确认事件不匹配".into());
        }
        let pending = original.is_none();
        if pending {
            tx.execute(
                "INSERT INTO feedback_bindings VALUES(?1,?2)",
                params![f.event_id, binding],
            )
            .map_err(err)?;
        }
        let changed = tx
            .execute(
                "UPDATE evidence SET undone=1 WHERE event_id=?1 AND normalized=?2 AND origin='' AND undone=0",
                params![f.event_id, text],
            )
            .map_err(err)?;
        tx.execute(
            "INSERT OR IGNORE INTO retired_events VALUES(?1)",
            [&f.event_id],
        )
        .map_err(err)?;
        tx.commit().map_err(err)?;
        return Ok(json!({"undone":changed,"pending":pending}));
    }
    if tx
        .query_row(
            "SELECT count(*) FROM retired_events WHERE event_id=?1",
            [&f.event_id],
            |r| r.get::<_, i64>(0),
        )
        .map_err(err)?
        > 0
    {
        return Err("事件已撤销或清除".into());
    }
    let fingerprint = digest(&serde_json::to_string(&f).map_err(err)?);
    if let Some(old) = tx
        .query_row(
            "SELECT fingerprint FROM evidence WHERE event_id=?1",
            [&f.event_id],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(err)?
    {
        if old != fingerprint {
            return Err("学习事件ID重用".into());
        }
        return Ok(json!({"duplicate":true}));
    }
    let weight = if f.operation == "reject" {
        -2.0
    } else if f.explicit_selection && f.original_rank > 0 {
        2.5
    } else if f.explicit_selection {
        // 数字键/鼠标明确选择首项同样表达偏好，不能当作默认空格接受。
        2.0
    } else {
        0.35
    };
    tx.execute("INSERT INTO evidence(event_id,fingerprint,text,normalized,previous,code,weight,created,source_app,schema_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![f.event_id,fingerprint,f.text,text,norm(&f.previous),f.input_code,weight,now(),f.source_app,f.schema_id]).map_err(err)?;
    for (anchor, _) in context_anchors(&norm(&f.previous)) {
        tx.execute(
            "INSERT OR IGNORE INTO evidence_anchors(event_id,anchor) VALUES(?1,?2)",
            params![f.event_id, anchor],
        )
        .map_err(err)?;
    }
    tx.execute(
        "INSERT INTO feedback_bindings VALUES(?1,?2)",
        params![f.event_id, binding],
    )
    .map_err(err)?;
    tx.commit().map_err(err)?;
    Ok(json!({"duplicate":false,"epoch":p.epoch}))
}
fn suffix_context(context: &str, previous: &str) -> bool {
    if previous.is_empty() || !context.ends_with(previous) {
        return false;
    }
    let prefix = &context[..context.len() - previous.len()];
    if previous
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
    {
        return prefix
            .chars()
            .last()
            .is_none_or(|c| !c.is_ascii_alphanumeric());
    }
    true
}
fn validate_origin(root: &Path, db: &Connection, origin: &str, expected: &str) -> Result<bool> {
    let source: Option<String> = db
        .query_row(
            "SELECT item_id FROM imports WHERE origin=?1 AND revision=?2",
            params![origin, expected],
            |r| r.get(0),
        )
        .optional()
        .map_err(err)?;
    let Some(id) = source else {
        return Ok(false);
    };
    let store = crate::knowledge::KnowledgeStore::open_readonly(root)?;
    match store.dispatch("read",json!({"id":id,"revision":expected,"max_chars":1}),false){
 Ok(_)=>Ok(db.query_row("SELECT EXISTS(SELECT 1 FROM imports WHERE origin=?1 AND item_id=?2 AND revision=?3)",params![origin,id,expected],|r|r.get(0)).map_err(err)?),
 Err(error)=>{
 if error.contains("locked")||error.contains("busy"){return Ok(false);}
 #[cfg(test)] checkpoint("origin_invalid");
 let tx=db.unchecked_transaction().map_err(err)?;
 let removed=tx.execute("DELETE FROM imports WHERE origin=?1 AND item_id=?2 AND revision=?3",params![origin,id,expected]).map_err(err)?;
 if removed>0{tx.execute("DELETE FROM evidence WHERE origin=?1 AND origin_revision=?2",params![origin,expected]).map_err(err)?;bump(&tx)?;}
 tx.commit().map_err(err)?;Ok(false)
 }
 }
}
fn bindings_current(db: &Connection, bindings: &HashMap<(String, String), bool>) -> Result<bool> {
    for ((origin, revision), valid) in bindings {
        if !valid {
            continue;
        }
        let current: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM imports WHERE origin=?1 AND revision=?2)",
                params![origin, revision],
                |r| r.get(0),
            )
            .map_err(err)?;
        if !current {
            return Ok(false);
        }
    }
    Ok(true)
}
/// 后台有界轮转核对回填来源；撤销失效贡献并推进世代，不重扫正文。
pub fn reconcile_imports(root: &Path, budget: usize) -> Result<Value> {
    let gate = root_gate(root)?;
    let _guard = gate.lock().map_err(err)?;
    let db = db(root)?;
    let budget = budget.clamp(1, 500);
    let cursor: Option<String> = db
        .query_row("SELECT cursor FROM reconcile_cursor WHERE id=1", [], |r| {
            r.get(0)
        })
        .optional()
        .map_err(err)?;
    let rows = {
        let mut stmt = db
            .prepare("SELECT origin,revision FROM imports WHERE origin>?1 ORDER BY origin LIMIT ?2")
            .map_err(err)?;
        let rows = stmt
            .query_map(params![cursor.unwrap_or_default(), budget as i64], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(err)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(err)?
    };
    let before: i64 = db
        .query_row("SELECT count(*) FROM imports", [], |r| r.get(0))
        .map_err(err)?;
    for (origin, revision) in &rows {
        validate_origin(root, &db, origin, revision)?;
    }
    let next = if rows.len() < budget {
        String::new()
    } else {
        rows.last().map(|r| r.0.clone()).unwrap_or_default()
    };
    db.execute(
        "INSERT OR REPLACE INTO reconcile_cursor VALUES(1,?1)",
        [&next],
    )
    .map_err(err)?;
    let after: i64 = db
        .query_row("SELECT count(*) FROM imports", [], |r| r.get(0))
        .map_err(err)?;
    let p = state(&db)?;
    Ok(
        json!({"checked":rows.len(),"removed":before-after,"remaining":after,"next_cursor":next,"epoch":p.epoch}),
    )
}
fn backfill_page(
    root: &Path,
    source: &str,
    cursor: &str,
    limit: u64,
) -> Result<Vec<(String, String, String)>> {
    let path = root.join(if source == "typed" {
        "knowledge/typed.sqlite"
    } else {
        "integration.db"
    });
    if !path.exists() {
        return Ok(vec![]);
    }
    let db = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(err)?;
    let mut statement=db.prepare(if source=="typed"{"SELECT id,CAST(revision AS TEXT),id FROM typed_segments WHERE id>?1 ORDER BY id LIMIT ?2"}else{"SELECT 'history:'||item_id,CAST(revision AS TEXT),item_id FROM integration_items WHERE item_id>?1 AND source_kind=?3 AND content_type='text' ORDER BY item_id LIMIT ?2"}).map_err(err)?;
    let map = |r: &rusqlite::Row<'_>| Ok((r.get(0)?, r.get(1)?, r.get(2)?));
    if source == "typed" {
        statement
            .query_map(params![cursor, limit + 1], map)
            .map_err(err)?
            .collect::<std::result::Result<_, _>>()
            .map_err(err)
    } else {
        statement
            .query_map(params![cursor, limit + 1, source], map)
            .map_err(err)?
            .collect::<std::result::Result<_, _>>()
            .map_err(err)
    }
}
#[derive(Default)]
struct Stats {
    frequency: f64,
    imported: f64,
    context: f64,
    rejected: f64,
    text: String,
    latest_accept: f64,
    latest_reject: f64,
}
fn rejected_in_current_context(stats: &Stats) -> bool {
    stats.latest_reject > stats.latest_accept
}
/// 只重排提供的候选；预测严格来自已学习的上下文关系或短语前缀。
pub fn query(root: &Path, q: Query) -> Result<Value> {
    crate::privacy_operation::ensure_readable(root)?;
    let gate = root_gate(root)?;
    let _guard = gate.lock().map_err(err)?;
    if q.input_code.len() > 256
        || !valid_schema(&q.schema_id)
        || q.context.chars().count() > 512
        || q.context_id.len() > 256
        || q.candidates.len() > 100
        || q.candidates.iter().any(|c| {
            c.id.len() > 256
                || !valid_text(&c.text)
                || c.base_rank > 1000
                || c.consumed_len > 1024
                || c.match_type.len() > 32
        })
    {
        return Err("候选查询超出限制".into());
    }
    let db = db(root)?;
    let p = state(&db)?;
    let enabled = p.enabled && p.epoch == q.learning_epoch && safe_app(&q.source_app);
    let mut candidates = q.candidates.clone();
    let mut predictions = vec![];
    let mut recalled_candidates: Vec<Candidate> = vec![];
    let mut skipped_origins = 0usize;
    let mut evidence_limited = false;
    let mut suppressed = HashSet::new();
    let mut validated: HashMap<(String, String), bool> = HashMap::new();
    if enabled {
        let context = norm(&q.context);
        let code = q.input_code.clone();
        let mut scores: HashMap<String, Stats> = HashMap::new();
        let mut affinities = HashMap::new();
        let anchors = context_anchors(&context);
        let current_anchors: HashMap<_, _> = anchors.iter().cloned().collect();
        // 分类探测与取证据必须共享快照；取完rows即结束，不能挡住后续
        // 来源失效清理或发布前的策略世代复核。
        let read_snapshot = db.unchecked_transaction().map_err(err)?;
        let mut predicates = Vec::new();
        let mut args: Vec<rusqlite::types::Value> = vec![];
        if !q.candidates.is_empty() {
            predicates.push(format!(
                "normalized IN ({})",
                vec!["?"; q.candidates.len()].join(",")
            ));
            for c in &q.candidates {
                args.push(norm(&c.text).into());
            }
        }
        let mut dense_anchor_matches = false;
        if !context.is_empty() {
            let known_anchors = present_anchors(&db, &anchors)?;
            let anchor_events = bounded_anchor_events(&db, &known_anchors)?;
            dense_anchor_matches = anchor_events.len() > EVIDENCE_QUERY_LIMIT;
            if dense_anchor_matches {
                // 按最近证据流逐条检查锚点，保留原4000条语义；不先物化
                // 常见中文锚点对应的数百万索引行，也不降低证据条数上限。
                predicates.push(format!("EXISTS (SELECT 1 FROM evidence_anchors WHERE evidence_anchors.event_id=evidence.event_id AND anchor IN ({}))", vec!["?"; known_anchors.len()].join(",")));
                args.extend(known_anchors.into_iter().map(Into::into));
            } else if !anchor_events.is_empty() {
                // 稀疏语境走主键ID集合，避免138个稀有/已撤销锚点导致全库
                // 逐行探测。这里包含全部有效命中，而非截断后的采样结果。
                predicates.push(format!(
                    "event_id IN ({})",
                    vec!["?"; anchor_events.len()].join(",")
                ));
                args.extend(anchor_events.into_iter().map(Into::into));
            }
            let suffixes: Vec<_> = context
                .char_indices()
                .rev()
                .take(256)
                .map(|(i, _)| context[i..].to_string())
                .collect();
            predicates.push(format!(
                "previous IN ({})",
                vec!["?"; suffixes.len()].join(",")
            ));
            predicates.push("(normalized>=? AND normalized<?)".into());
            for suffix in suffixes {
                args.push(suffix.into());
            }
            args.push(context.clone().into());
            args.push(format!("{context}\u{10ffff}").into());
        }
        if predicates.is_empty() {
            predicates.push("0".into());
        }
        let index_hint = if dense_anchor_matches {
            "INDEXED BY evidence_active_recency"
        } else {
            ""
        };
        let sql = format!("SELECT text,normalized,previous,code,weight,created,origin,origin_revision,source_app,schema_id FROM evidence {index_hint} WHERE undone=0 AND privacy_term_digest(normalized) NOT IN(SELECT text FROM forgotten) AND ({}) ORDER BY (origin='') DESC,created DESC LIMIT {EVIDENCE_QUERY_LIMIT}", predicates.join(" OR "));
        let rows = {
            let mut stmt = db.prepare(&sql).map_err(err)?;
            let rows = stmt
                .query_map(rusqlite::params_from_iter(args), |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, f64>(4)?,
                        r.get::<_, f64>(5)?,
                        r.get::<_, String>(6)?,
                        r.get::<_, String>(7)?,
                        r.get::<_, String>(8)?,
                        r.get::<_, String>(9)?,
                    ))
                })
                .map_err(err)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(err)?
        };
        read_snapshot.commit().map_err(err)?;
        evidence_limited = rows.len() == EVIDENCE_QUERY_LIMIT;

        let mut skipped: HashSet<String> = HashSet::new();
        let timestamp = now();
        for (
            text,
            key,
            previous,
            event_code,
            weight,
            created,
            origin,
            origin_revision,
            source_app,
            schema_id,
        ) in rows
        {
            let imported = !origin.is_empty();
            if imported {
                let valid = if let Some(valid) =
                    validated.get(&(origin.clone(), origin_revision.clone()))
                {
                    *valid
                } else if validated.len() < 32 {
                    let valid = validate_origin(root, &db, &origin, &origin_revision)?;
                    validated.insert((origin.clone(), origin_revision), valid);
                    valid
                } else {
                    skipped.insert(origin);
                    false
                };
                if !valid {
                    continue;
                }
            }
            // 旧证据仍能提供弱全局先验；不同已知方案不能共享码形偏好。
            if !schema_id.is_empty() && !q.schema_id.is_empty() && schema_id != q.schema_id {
                continue;
            }
            let decay = (-((timestamp - created).max(0.0)) / (14.0 * 86400.0)).exp();
            let stat = scores.entry(key).or_default();
            if stat.text.is_empty() {
                stat.text = text;
            }
            if weight < 0.0 {
                if schema_id == q.schema_id
                    && event_code == code
                    && (source_app.is_empty() || source_app == q.source_app)
                    && rejection_context_matches(&context, &previous)
                {
                    stat.rejected += -weight * decay;
                    if timestamp - created < 14.0 * 86400.0 {
                        stat.latest_reject = stat.latest_reject.max(created);
                    }
                }
                continue;
            }
            let app_weight = if source_app == q.source_app {
                1.0
            } else if source_app.is_empty() {
                0.6
            } else {
                0.35
            };
            let contribution = weight * decay * if imported { 1.0 } else { app_weight };
            if !imported
                && schema_id == q.schema_id
                && event_code == code
                && (source_app.is_empty() || source_app == q.source_app)
                && rejection_context_matches(&context, &previous)
            {
                stat.latest_accept = stat.latest_accept.max(created);
            }
            if code.is_empty() || event_code.is_empty() || event_code == code {
                if imported {
                    let contribution = (0.75 - stat.imported).max(0.0).min(contribution);
                    stat.imported += contribution;
                    stat.frequency += contribution;
                } else {
                    stat.frequency += contribution;
                }
            }
            stat.context += contribution
                * cached_context_affinity(&context, &previous, &current_anchors, &mut affinities);
        }
        skipped_origins = skipped.len();
        let mut base_gains = HashMap::new();
        if !context.is_empty() {
            for c in &candidates {
                let key = norm(&c.text);
                // 明确拒绝占优时，公共词频不能把用户拒绝的词重新顶回来。
                if scores.get(&key).is_some_and(|s| {
                    rejected_in_current_context(s) || s.rejected > s.frequency + s.context
                }) {
                    continue;
                }
                base_gains.insert(key, base_context_gain(&db, &context, &c.text, &anchors)?);
            }
        }
        let gain = |c: &Candidate| -> f64 {
            scores
                .get(&norm(&c.text))
                .map(|s| {
                    if rejected_in_current_context(s) {
                        return -4.0;
                    }
                    // 长期频率与当前语境各自限幅；频率不能占满语境的预算。
                    // context 已含匹配强度，不能再按 context/frequency 二次折损。
                    (1.5 * s.frequency.ln_1p()).min(6.0) + 4.0 * (1.0 - (-s.context / 2.0).exp())
                        - (2.0 * s.rejected.ln_1p()).min(4.0)
                })
                .unwrap_or(0.0)
                + base_gains.get(&norm(&c.text)).copied().unwrap_or(0.0)
        };
        // 相同拼音消耗长度和匹配类别才可互相重排，保留每组的原始槽位。
        let mut groups: HashMap<(usize, String), Vec<usize>> = HashMap::new();
        for (i, c) in candidates.iter().enumerate() {
            groups
                .entry((c.consumed_len, c.match_type.clone()))
                .or_default()
                .push(i);
        }
        for positions in groups.values() {
            let mut group: Vec<_> = positions.iter().map(|i| candidates[*i].clone()).collect();
            group.sort_by(|a, b| {
                // 引擎名次不是等间距的对数概率；递减间距保留先验顺序，
                // 让有证据的组合词能跨过前几项；频率、语境分别使用有界增益。
                (1.5 * (a.base_rank as f64).ln_1p() - gain(a))
                    .total_cmp(&(1.5 * (b.base_rank as f64).ln_1p() - gain(b)))
                    .then_with(|| a.base_rank.cmp(&b.base_rank))
            });
            for (i, c) in positions.iter().zip(group) {
                candidates[*i] = c;
            }
        }
        recalled_candidates = recall_candidates(&db, &q)?;
        if code.is_empty() {
            let mut proposed: HashMap<String, (String, f64)> = HashMap::new();
            for (key, s) in &scores {
                if rejected_in_current_context(s) || s.rejected > s.context + s.frequency {
                    suppressed.insert(key.clone());
                    continue;
                }
                if s.context > 0.0 && key != &context {
                    proposed.insert(key.clone(), (s.text.clone(), s.context));
                }
                // 前缀必须从词首开始；英文不能从词中间截断制造错误补全。
                if !context.is_empty() && key.starts_with(&context) && key != &context {
                    let suffix = &key[context.len()..];
                    let boundary = !context
                        .chars()
                        .last()
                        .is_some_and(|c| c.is_ascii_alphanumeric())
                        || suffix
                            .chars()
                            .next()
                            .is_some_and(|c| !c.is_ascii_alphanumeric());
                    if boundary {
                        let text = remaining_prefix(&s.text, &context)
                            .unwrap_or(suffix)
                            .trim()
                            .to_string();
                        if !text.is_empty() {
                            proposed
                                .entry(text.clone())
                                .or_insert((text, s.frequency * 0.5));
                        }
                    }
                }
            }
            let mut list: Vec<_> = proposed
                .into_values()
                .filter(|(_, weight)| *weight > 0.0)
                .collect();
            list.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            predictions = list
                .into_iter()
                .take(q.limit.clamp(1, 10))
                .map(|(text, _)| json!({"id":format!("learned:{}",digest(&text)),"text":text}))
                .collect();
        }
    }
    if q.input_code.is_empty()
        && !q.context.trim().is_empty()
        && safe_app(&q.source_app)
        && p.epoch == q.learning_epoch
    {
        let mut seen: HashSet<String> = predictions
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str).map(norm))
            .collect();
        for text in base_predictions(&db, &q.context, q.limit.clamp(1, 10))? {
            let key = norm(&text);
            let rejected = scoped_rejected(&db, &q, &key)?;
            if suppressed.contains(&key) || rejected || !seen.insert(key) {
                continue;
            }
            if predictions.len() >= q.limit.clamp(1, 10) {
                break;
            }
            predictions.push(json!({"id":format!("base:{}",digest(&text)),"text":text}));
        }
    }
    #[cfg(test)]
    checkpoint("query_finish");
    crate::privacy_operation::ensure_readable(root)?;
    let final_policy = state(&db)?;
    if final_policy.epoch != p.epoch
        || final_policy.enabled != p.enabled
        || !bindings_current(&db, &validated)?
    {
        return Ok(
            json!({"enabled":false,"epoch":final_policy.epoch,"ordered_ids":q.candidates.iter().map(|c|&c.id).collect::<Vec<_>>(),"predictions":[],"recalled_candidates":[],"context_id":q.context_id,"invalidated":true}),
        );
    }
    Ok(
        json!({"enabled":enabled,"epoch":p.epoch,"ordered_ids":candidates.iter().map(|c|&c.id).collect::<Vec<_>>(),"predictions":predictions,"recalled_candidates":recalled_candidates,"context_id":q.context_id,"skipped_unverified_sources":skipped_origins,"evidence_limited":evidence_limited,"warnings":if skipped_origins>0 {vec!["本次相关来源验证达到32条预算，其余来源未参与排序"]}else{vec![]}}),
    )
}
fn status(root: &Path, db: &Connection) -> Result<Value> {
    let origins = {
        let mut stmt = db
            .prepare("SELECT origin,revision FROM imports ORDER BY origin LIMIT 32")
            .map_err(err)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(err)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(err)?
    };
    let mut verified = HashMap::new();
    for (origin, revision) in origins {
        let valid = validate_origin(root, db, &origin, &revision)?;
        verified.insert((origin, revision), valid);
    }
    let p = state(db)?;
    let mut permitted = "origin=''".to_string();
    let mut args: Vec<rusqlite::types::Value> = vec![];
    for ((origin, revision), valid) in &verified {
        if *valid {
            permitted.push_str(" OR (origin=? AND origin_revision=?)");
            args.push(origin.clone().into());
            args.push(revision.clone().into());
        }
    }
    let filter =
        format!("undone=0 AND privacy_term_digest(normalized) NOT IN(SELECT text FROM forgotten) AND ({permitted})");
    let sql=format!("SELECT text,sum(weight) AS score FROM evidence WHERE {filter} GROUP BY normalized ORDER BY score DESC LIMIT 100");
    let mut stmt = db.prepare(&sql).map_err(err)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(args.iter()), |r| {
            Ok(json!({"text":r.get::<_,String>(0)?,"score":r.get::<_,f64>(1)?}))
        })
        .map_err(err)?;
    let terms = rows
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(err)?;
    let term_count: i64 = db
        .query_row(
            &format!("SELECT count(DISTINCT normalized) FROM evidence WHERE {filter}"),
            rusqlite::params_from_iter(args.iter()),
            |r| r.get(0),
        )
        .map_err(err)?;
    let event_count: i64 = db
        .query_row(
            &format!("SELECT count(*) FROM evidence WHERE {filter}"),
            rusqlite::params_from_iter(args.iter()),
            |r| r.get(0),
        )
        .map_err(err)?;
    let imports: i64 = db
        .query_row("SELECT count(*) FROM imports", [], |r| r.get(0))
        .map_err(err)?;
    let base: i64 = db
        .query_row("SELECT count(*) FROM base_phrases", [], |r| r.get(0))
        .map_err(err)?;
    let final_policy = state(db)?;
    let valid = final_policy.epoch == p.epoch
        && final_policy.enabled == p.enabled
        && bindings_current(db, &verified)?;
    Ok(
        json!({"enabled":final_policy.enabled,"epoch":final_policy.epoch,"counts":{"terms":if valid{term_count}else{0},"events":if valid{event_count}else{0},"imports":imports,"base_terms":base},"learned_terms":if valid{terms}else{vec![]},"backfill_summary":{"tracked_sources":imports},"terms_partial":imports>verified.len() as i64,"invalidated":!valid}),
    )
}
fn phrases(text: &str) -> Vec<String> {
    let mut out = vec![];
    let mut seen = HashSet::new();
    for line in text.split(['\n', '。', '！', '？', ';', '；']) {
        let line = line.trim();
        if (2..=64).contains(&line.chars().count()) && seen.insert(norm(line)) {
            out.push(line.to_string());
        }
        if out.len() == 20 {
            break;
        }
    }
    out
}
/// 主服务已耐久接受后调用；本域写入、拒绝重放屏障和回执属于同一个事务。
pub(crate) fn privacy_receipt(root: &Path, id: &str, digest: &[u8]) -> Result<Option<u64>> {
    let path = root.join("knowledge/personalization.sqlite");
    if !path.exists() {
        return Ok(None);
    }
    let db = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(err)?;
    let present: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='privacy_domain_receipts')",
            [],
            |r| r.get(0),
        )
        .map_err(err)?;
    if !present {
        return Ok(None);
    }
    let old: Option<(Vec<u8>, u64)> = db
        .query_row(
            "SELECT digest,result_epoch FROM privacy_domain_receipts WHERE operation_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(err)?;
    match old {
        Some((old, epoch)) if old == digest => Ok(Some(epoch)),
        Some(_) => Err("privacy_domain_conflict".into()),
        None => Ok(None),
    }
}

pub(crate) fn apply_privacy(
    root: &Path,
    request: &crate::privacy_operation::PrivacyRequest,
    request_digest: &[u8],
) -> Result<u64> {
    request.validate()?;
    if request_digest.len() != 32 {
        return Err("privacy_domain_digest_invalid".into());
    }
    let gate = root_gate(root)?;
    let _guard = gate.lock().map_err(err)?;
    let mut db = db(root)?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS privacy_domain_receipts(operation_id TEXT PRIMARY KEY,digest BLOB NOT NULL,result_epoch INTEGER NOT NULL)").map_err(err)?;
    let old: Option<(Vec<u8>, u64)> = db
        .query_row(
            "SELECT digest,result_epoch FROM privacy_domain_receipts WHERE operation_id=?1",
            [&request.operation_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(err)?;
    if let Some((digest, epoch)) = old {
        return if digest == request_digest {
            Ok(epoch)
        } else {
            Err("privacy_domain_conflict".into())
        };
    }
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(err)?;
    if matches!(
        request.scope,
        crate::privacy_operation::PrivacyScope::ForgetTerm { .. }
    ) {
        let crate::privacy_operation::PrivacyScope::ForgetTerm { term: text } = &request.scope
        else {
            unreachable!()
        };
        let key = norm(text);
        tx.execute(
            "INSERT OR IGNORE INTO forgotten VALUES(privacy_term_digest(?1))",
            [&key],
        )
        .map_err(err)?;
        tx.execute("INSERT OR IGNORE INTO retired_events SELECT event_id FROM evidence WHERE normalized=?1 OR instr(lower(previous),?1)>0",[&key]).map_err(err)?;
        tx.execute("DELETE FROM evidence_anchors WHERE event_id IN(SELECT event_id FROM evidence WHERE normalized=?1 OR instr(lower(previous),?1)>0)", [&key]).map_err(err)?;
        tx.execute(
            "DELETE FROM evidence WHERE normalized=?1 OR instr(lower(previous),?1)>0",
            [&key],
        )
        .map_err(err)?;
    } else {
        tx.execute(
            "INSERT OR IGNORE INTO retired_events SELECT event_id FROM evidence",
            [],
        )
        .map_err(err)?;
        tx.execute(
            "INSERT OR IGNORE INTO forgotten SELECT privacy_term_digest(normalized) FROM evidence",
            [],
        )
        .map_err(err)?;
        tx.execute("DELETE FROM evidence", []).map_err(err)?;
        tx.execute("DELETE FROM evidence_anchors", [])
            .map_err(err)?;
        tx.execute("DELETE FROM imports", []).map_err(err)?;
    }
    bump(&tx)?;
    let epoch = state(&tx)?.epoch;
    tx.execute(
        "INSERT INTO privacy_domain_receipts(operation_id,digest,result_epoch) VALUES(?1,?2,?3)",
        params![request.operation_id, request_digest, epoch],
    )
    .map_err(err)?;
    tx.commit().map_err(err)?;
    Ok(epoch)
}

/// 本地管理动作；历史回填必须由用户显式调用。
pub fn manage(root: &Path, action: &str, payload: &Value) -> Result<Value> {
    crate::privacy_operation::ensure_readable(root)?;
    let gate = root_gate(root)?;
    let _guard = gate.lock().map_err(err)?;
    if action == "personalization_enabled" {
        set_enabled_inner(
            root,
            payload
                .get("enabled")
                .and_then(Value::as_bool)
                .ok_or("enabled必须是布尔值")?,
        )?;
    }
    let mut db = db(root)?;
    match action {
        "personalization_status" | "personalization_enabled" => {}
        "personalization_forget" | "personalization_clear" => {
            return Err("privacy_operation_required".into())
        }
        "personalization_backfill" => {
            let import_policy = state(&db)?;
            if !import_policy.enabled {
                return Err("个人学习已关闭".into());
            }
            let source = payload
                .get("source")
                .and_then(Value::as_str)
                .ok_or("缺少source")?;
            let source_id = match source {
                "typed" => "history:saved_snippet",
                "voice" => "history:voice",
                "clipboard" => "history:clipboard",
                _ => return Err("不支持的回填来源".into()),
            };
            let limit = payload
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(200)
                .clamp(1, 500);
            let store = crate::knowledge::KnowledgeStore::open_readonly(root)?;
            let sources = store.dispatch("sources", json!({}), false)?;
            if !sources["sources"].as_array().is_some_and(|ss| {
                ss.iter()
                    .any(|s| s["id"] == source_id && s["enabled"] == true)
            }) {
                return Err("此来源未参与知识检索，请先启用来源".into());
            }
            let cursor = payload.get("cursor").and_then(Value::as_str).unwrap_or("");
            if cursor.len() > 512 {
                return Err("回填游标过长".into());
            }
            let page = backfill_page(root, source, cursor, limit)?;
            let mut has_more = page.len() > limit as usize;
            let mut imported = 0;
            let mut skipped = 0;
            let mut processed = 0;
            let mut next_cursor = None;
            let mut warnings = Vec::new();
            for (id, revision, key_cursor) in page.into_iter().take(limit as usize) {
                if state(&db)?.epoch != import_policy.epoch {
                    has_more = true;
                    warnings.push("学习世代已变化，回填停止，请重新发起".into());
                    break;
                }
                processed += 1;
                next_cursor = Some(key_cursor);
                let origin = format!("{source}:{id}");
                let known: Option<String> = db
                    .query_row(
                        "SELECT revision FROM imports WHERE origin=?1",
                        [&origin],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(err)?;
                if known.as_deref() == Some(revision.as_str())
                    && validate_origin(root, &db, &origin, &revision)?
                {
                    skipped += 1;
                    continue;
                }
                let read = match store.dispatch(
                    "read",
                    json!({"id":id,"revision":revision,"max_chars":8000}),
                    false,
                ) {
                    Ok(read) => read,
                    Err(_) => {
                        validate_origin(root, &db, &origin, &revision)?;
                        skipped += 1;
                        continue;
                    }
                };
                if matches!(
                    crate::decision::classify_text_safety(
                        read.get("text").and_then(Value::as_str).unwrap_or("")
                    ),
                    crate::decision::LocalTextClass::Sensitive
                        | crate::decision::LocalTextClass::Noise
                ) {
                    skipped += 1;
                    warnings.push("明显敏感或无效的短内容未进入个人学习".to_string());
                    continue;
                }
                if read["truncated"] == true && warnings.is_empty() {
                    warnings.push("单条来源仅处理前8000字符，超长正文未完整导入".to_string());
                }
                let list = phrases(read["text"].as_str().unwrap_or(""));
                if list.is_empty() {
                    skipped += 1;
                    continue;
                }
                let tx = db
                    .transaction_with_behavior(TransactionBehavior::Immediate)
                    .map_err(err)?;
                let current = state(&tx)?;
                if !current.enabled || current.epoch != import_policy.epoch {
                    return Err("回填期间学习设置已改变，请重新发起".into());
                }
                tx.execute("DELETE FROM evidence WHERE origin=?1", [&origin])
                    .map_err(err)?;
                tx.execute(
                    "INSERT OR REPLACE INTO imports VALUES(?1,?2,?3,?4)",
                    params![origin, id, revision, source],
                )
                .map_err(err)?;
                for (i, text) in list.iter().enumerate() {
                    let key = norm(text);
                    let event = format!("import:{}", digest(&format!("{origin}:{revision}:{i}")));
                    tx.execute("INSERT OR IGNORE INTO evidence(event_id,fingerprint,text,normalized,previous,code,weight,created,origin,origin_revision) SELECT ?1,?1,?2,?3,'','',0.25,?4,?5,?6 WHERE NOT EXISTS(SELECT 1 FROM forgotten WHERE text=privacy_term_digest(?3))",params![event,text,key,now(),origin,revision]).map_err(err)?;
                }
                tx.commit().map_err(err)?;
                imported += 1;
            }
            let mut result = status(root, &db)?;
            result["backfill_summary"] = json!({"imported":imported,"skipped":skipped,"source":source,"processed":processed,"has_more":has_more,"next_cursor":if has_more{next_cursor}else{None},"warnings":warnings});
            return Ok(result);
        }
        _ => return Err("未知个人学习管理操作".into()),
    }
    status(root, &db)
}

fn remaining_prefix<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let text = text.trim();
    let mut normalized = String::new();
    for (i, c) in text.char_indices() {
        normalized.extend(c.to_lowercase());
        if normalized == prefix {
            return Some(&text[i + c.len_utf8()..]);
        }
        if normalized.len() > prefix.len() {
            return None;
        }
    }
    None
}

/// 索引随应用打包的通用词频TSV，不读取用户词典。词库来源和许可证由打包层保留。
pub fn install_base_lexicon(root: &Path, path: &Path) -> Result<Value> {
    let gate = root_gate(root)?;
    let _guard = gate.lock().map_err(err)?;
    use std::io::Read;
    let metadata = std::fs::metadata(path).map_err(err)?;
    if !metadata.is_file() || metadata.len() > 64 * 1024 * 1024 {
        return Err("通用词库大小无效".into());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(err)?
        .take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(err)?;
    if bytes.len() > 64 * 1024 * 1024 {
        return Err("通用词库过大".into());
    }
    let text = std::str::from_utf8(&bytes).map_err(err)?;
    let sha = digest(text);
    let mut db = db(root)?;
    let old: Option<String> = db
        .query_row("SELECT sha256 FROM base_meta WHERE id=1", [], |r| r.get(0))
        .optional()
        .map_err(err)?;
    if old.as_deref() == Some(&sha) {
        return Ok(json!({"changed":false,"sha256":sha}));
    }
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(err)?;
    tx.execute("DELETE FROM base_phrases", []).map_err(err)?;
    let mut count = 0;
    {
        let mut insert=tx.prepare("INSERT INTO base_phrases VALUES(?1,?2,?3) ON CONFLICT(normalized) DO UPDATE SET frequency=max(frequency,excluded.frequency)").map_err(err)?;
        for line in text.lines() {
            if line.starts_with('#') || line.trim().is_empty() {
                continue;
            }
            let Some((phrase, freq)) = line.split_once('\t') else {
                return Err("通用词库必须为词语与频次两列TSV".into());
            };
            let freq: f64 = freq.parse().map_err(|_| "通用词库频次无效")?;
            if !freq.is_finite() || freq < 0.0 {
                return Err("通用词库频次无效".into());
            }
            if !(2..=32).contains(&phrase.chars().count()) || !valid_text(phrase) || freq == 0.0 {
                continue;
            }
            insert
                .execute(params![norm(phrase), phrase, freq])
                .map_err(err)?;
            count += 1;
        }
    }
    if count == 0 {
        return Err("通用词库没有有效短语".into());
    }
    tx.execute("INSERT OR REPLACE INTO base_meta VALUES(1,?1)", [&sha])
        .map_err(err)?;
    tx.commit().map_err(err)?;
    Ok(json!({"changed":true,"sha256":sha,"entries":count}))
}
fn base_predictions(db: &Connection, context: &str, limit: usize) -> Result<Vec<String>> {
    let context = norm(context);
    let forgotten = {
        let mut stmt = db.prepare("SELECT text FROM forgotten").map_err(err)?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(err)?;
        rows.collect::<std::result::Result<HashSet<_>, _>>()
            .map_err(err)?
    };
    let starts: Vec<_> = context
        .char_indices()
        .rev()
        .take(16)
        .map(|(i, _)| i)
        .collect();
    for start in starts.into_iter().rev() {
        let prefix = &context[start..];
        if !suffix_context(&context, prefix) {
            continue;
        }
        let mut stmt=db.prepare("SELECT text,frequency FROM base_phrases WHERE normalized>?1 AND normalized<?2 AND privacy_term_digest(normalized) NOT IN(SELECT text FROM forgotten) ORDER BY frequency DESC,normalized LIMIT 100").map_err(err)?;
        let rows = stmt
            .query_map(params![prefix, format!("{prefix}\u{10ffff}")], |r| {
                r.get::<_, String>(0)
            })
            .map_err(err)?;
        let mut output = Vec::new();
        let mut seen = HashSet::new();
        for row in rows {
            let text = row.map_err(err)?;
            let Some(suffix) = remaining_prefix(&text, prefix) else {
                continue;
            };
            if prefix
                .chars()
                .last()
                .is_some_and(|c| c.is_ascii_alphanumeric())
                && suffix
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphanumeric())
            {
                continue;
            }
            let suffix = suffix.trim();
            if !suffix.is_empty()
                && !forgotten.contains(
                    &db.query_row("SELECT privacy_term_digest(?1)", [suffix], |r| {
                        r.get::<_, String>(0)
                    })
                    .map_err(err)?,
                )
                && seen.insert(norm(suffix))
            {
                output.push(suffix.to_owned());
            }
            if output.len() == limit {
                break;
            }
        }
        if !output.is_empty() {
            return Ok(output);
        }
    }
    Ok(vec![])
}

fn base_context_gain(
    db: &Connection,
    context: &str,
    text: &str,
    anchors: &[(String, f64)],
) -> Result<f64> {
    let context = norm(context);
    let text = norm(text);
    if context.is_empty() || text.is_empty() {
        return Ok(0.0);
    }
    let starts: Vec<_> = context
        .char_indices()
        .rev()
        .take(16)
        .map(|(i, _)| i)
        .collect();
    let mut prefixes: Vec<(String, f64)> = starts
        .into_iter()
        .rev()
        .map(|start| context[start..].to_owned())
        .filter(|prefix| suffix_context(&context, prefix))
        .map(|prefix| (prefix, 1.0))
        .collect();
    let mut seen: HashSet<_> = prefixes.iter().map(|(prefix, _)| prefix.clone()).collect();
    for (anchor, weight) in anchors {
        if seen.insert(anchor.clone()) {
            prefixes.push((anchor.clone(), 0.6 * weight));
        }
    }
    let mut weights = HashMap::new();
    for (prefix, context_weight) in prefixes {
        let separator = if prefix
            .chars()
            .last()
            .is_some_and(|c| c.is_ascii_alphanumeric())
            && text
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphanumeric())
        {
            " "
        } else {
            ""
        };
        let joined = format!("{prefix}{separator}{text}");
        weights.insert(joined, context_weight);
    }
    if weights.is_empty() {
        return Ok(0.0);
    }
    // 全句锚点一次索引查询，避免为每个候选发出上百次短查询。
    let sql = format!("SELECT normalized,frequency FROM base_phrases WHERE normalized IN ({}) AND privacy_term_digest(normalized) NOT IN(SELECT text FROM forgotten) AND NOT EXISTS(SELECT 1 FROM forgotten WHERE text=privacy_term_digest(?))", vec!["?"; weights.len()].join(","));
    let mut args: Vec<String> = weights.keys().cloned().collect();
    args.push(text);
    let mut stmt = db.prepare(&sql).map_err(err)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(args), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))
        })
        .map_err(err)?;
    let mut strongest: f64 = 0.0;
    for row in rows {
        let (key, freq) = row.map_err(err)?;
        // 低频组合只有弱证据，不能因名次间距缩小而压过常用候选。
        let reliability = freq / (freq + 50.0);
        strongest = strongest.max(
            (freq.ln_1p() / 3.0).min(3.0) * reliability * weights.get(&key).copied().unwrap_or(0.0),
        );
    }
    Ok(strongest)
}

#[cfg(test)]
mod context_cache_tests {
    use super::*;

    #[test]
    fn recent_clause_cache_does_not_share_exact_context_confidence() {
        let context = "第一句话。项目开发方案";
        let anchors = context_anchors(context).into_iter().collect();
        let mut cache = HashMap::new();
        let weak = cached_context_affinity(context, "其他前句。项目开发方案", &anchors, &mut cache);
        assert!(weak > 0.0 && weak < 1.0);
        assert_eq!(
            cached_context_affinity(context, context, &anchors, &mut cache),
            1.0
        );
        assert_eq!(
            cached_context_affinity(context, "又一前句。项目开发方案", &anchors, &mut cache),
            weak
        );
        assert_eq!(cache.len(), 1);
    }
}

#[cfg(test)]
type TestCheckpoint = std::sync::Arc<dyn Fn(&str) + Send + Sync>;
#[cfg(test)]
static CHECKPOINT: std::sync::Mutex<Option<TestCheckpoint>> = std::sync::Mutex::new(None);
#[cfg(test)]
fn checkpoint(name: &str) {
    let hook = CHECKPOINT.lock().unwrap().clone();
    if let Some(hook) = hook {
        hook(name);
    }
}
#[cfg(test)]
mod race_tests {
    use super::*;
    fn q(root: &Path, context: &str) -> Query {
        Query {
            schema_id: String::new(),
            input_code: String::new(),
            context: context.into(),
            context_id: "target".into(),
            source_app: "com.example.editor".into(),
            learning_epoch: policy(root).unwrap().epoch,
            candidates: vec![],
            limit: 5,
        }
    }

    #[test]
    fn rerank_accepts_only_real_ids_within_eligibility_groups() {
        let candidates = vec![
            Candidate {
                id: "a".into(),
                text: "甲".into(),
                base_rank: 0,
                consumed_len: 2,
                match_type: "exact".into(),
            },
            Candidate {
                id: "b".into(),
                text: "乙".into(),
                base_rank: 1,
                consumed_len: 2,
                match_type: "exact".into(),
            },
            Candidate {
                id: "c".into(),
                text: "丙".into(),
                base_rank: 2,
                consumed_len: 1,
                match_type: "abbreviation".into(),
            },
        ];
        let ids = vec!["b".into(), "a".into(), "c".into()];
        let output = apply_rerank(&candidates, &ids).unwrap();
        assert_eq!(
            output.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            vec!["b", "a", "c"]
        );
        assert_eq!(
            apply_rerank(&candidates, &["missing".into()]).unwrap_err(),
            "candidate_rerank_identity"
        );
    }
    fn pause_at(
        stage: &'static str,
    ) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
        let (arrived_tx, arrived_rx) = std::sync::mpsc::channel();
        let (resume_tx, resume_rx) = std::sync::mpsc::channel();
        let receiver = std::sync::Mutex::new(resume_rx);
        *CHECKPOINT.lock().unwrap() = Some(std::sync::Arc::new(move |name| {
            if name == stage {
                arrived_tx.send(()).unwrap();
                receiver.lock().unwrap().recv().unwrap();
            }
        }));
        (arrived_rx, resume_tx)
    }
    #[test]
    fn concurrent_epoch_change_and_revision_cas_do_not_leak_or_delete_new_data() {
        // 精确暂停在输出之前，用独立SQLite连接模拟另一进程修改权限世代。
        for operation in ["disable", "forget", "clear"] {
            let tmp = tempfile::tempdir().unwrap();
            let root = tmp.path();
            feedback(
                root,
                Feedback {
                    event_id: "accept".into(),
                    context_id: "target".into(),
                    schema_id: String::new(),
                    input_code: "".into(),
                    text: "待撤销秘密".into(),
                    previous: "前文".into(),
                    explicit_selection: true,
                    original_rank: 2,
                    source_app: "com.example.editor".into(),
                    learning_epoch: 1,
                    operation: "accept".into(),
                },
            )
            .unwrap();
            let request = q(root, "前文");
            let (arrived, resume) = pause_at("query_finish");
            let path = root.to_owned();
            let thread = std::thread::spawn(move || query(&path, request).unwrap());
            arrived
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
            let db = Connection::open(root.join("knowledge/personalization.sqlite")).unwrap();
            match operation {
                "disable" => {
                    db.execute("UPDATE learning_policy SET enabled=0,epoch=epoch+1", [])
                        .unwrap();
                }
                "forget" => {
                    db.execute_batch("BEGIN IMMEDIATE; INSERT INTO forgotten VALUES('待撤销秘密'); DELETE FROM evidence; UPDATE learning_policy SET epoch=epoch+1; COMMIT;").unwrap();
                }
                _ => {
                    db.execute_batch("BEGIN IMMEDIATE; DELETE FROM evidence; UPDATE learning_policy SET epoch=epoch+1; COMMIT;").unwrap();
                }
            }
            resume.send(()).unwrap();
            let result = thread.join().unwrap();
            assert_eq!(result["predictions"], json!([]));
            assert_eq!(result["invalidated"], true);
            assert_eq!(result["epoch"], 2);
            *CHECKPOINT.lock().unwrap() = None;
        }
        // R1校验已经失败、删除尚未执行时，另一连接替换成R2。CAS不得误删R2。
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        crate::knowledge::KnowledgeStore::open(root).unwrap();
        let capture = crate::typed_history::set_capture(root, true).unwrap();
        let item = crate::typed_history::record(
            root,
            "a",
            "s",
            "原文片段",
            "com.example.editor",
            capture.epoch,
        )
        .unwrap();
        manage(root, "personalization_backfill", &json!({"source":"typed"})).unwrap();
        crate::typed_history::record(root, "b", "s", "更新", "com.example.editor", capture.epoch)
            .unwrap();
        let request = q(root, "原文");
        let (arrived, resume) = pause_at("origin_invalid");
        let path = root.to_owned();
        let thread = std::thread::spawn(move || query(&path, request).unwrap());
        arrived
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let db = Connection::open(root.join("knowledge/personalization.sqlite")).unwrap();
        let origin = format!("typed:{}", item["id"].as_str().unwrap());
        db.execute_batch("BEGIN IMMEDIATE").unwrap();
        db.execute("UPDATE imports SET revision='2' WHERE origin=?1", [&origin])
            .unwrap();
        db.execute("UPDATE evidence SET text='原文片段更新',normalized='原文片段更新',origin_revision='2' WHERE origin=?1",[&origin]).unwrap();
        db.execute_batch("COMMIT").unwrap();
        resume.send(()).unwrap();
        let result = thread.join().unwrap();
        assert_eq!(result["predictions"], json!([]));
        *CHECKPOINT.lock().unwrap() = None;
        assert_eq!(
            db.query_row(
                "SELECT revision FROM imports WHERE origin=?1",
                [&origin],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "2"
        );
        assert_eq!(
            db.query_row(
                "SELECT text FROM evidence WHERE origin=?1",
                [&origin],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "原文片段更新"
        );
        assert_eq!(
            query(root, q(root, "原文")).unwrap()["predictions"][0]["text"],
            "片段更新"
        );
    }
}
