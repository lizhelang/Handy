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
/// 联想查询结束后放回的已初始化学习库连接，键为库文件路径，附文件身份。
type LearnedPool = std::collections::BTreeMap<std::path::PathBuf, (u64, Connection)>;
static LEARNED_POOL: std::sync::Mutex<LearnedPool> =
    std::sync::Mutex::new(std::collections::BTreeMap::new());

fn learned_identity(path: &Path) -> Option<u64> {
    let metadata = std::fs::metadata(path).ok().filter(|m| m.is_file())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(metadata.ino() ^ metadata.dev().rotate_left(32))
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        None
    }
}

/// 优先取回查询放回的连接：建表检查、隐私密钥与摘要函数只在首次打开时完成。
/// 库文件被删除或替换（身份变化）时丢弃旧连接重新初始化。
fn db(root: &Path) -> Result<Connection> {
    let path = root.join("knowledge/personalization.sqlite");
    if let Some(identity) = learned_identity(&path) {
        if let Some((known, conn)) = LEARNED_POOL.lock().map_err(err)?.remove(&path) {
            if known == identity && conn.is_autocommit() {
                return Ok(conn);
            }
        }
    }
    open_db(root)
}

/// 联想查询结束后归还连接；仍有事务或库文件身份变化时直接关闭。
fn release_db(root: &Path, conn: Connection) {
    let path = root.join("knowledge/personalization.sqlite");
    if !conn.is_autocommit() {
        return;
    }
    if let (Some(identity), Ok(mut pool)) = (learned_identity(&path), LEARNED_POOL.lock()) {
        // 生产只有一个数据目录；上限防止测试或多目录场景长期占用文件句柄。
        pool.retain(|known, _| known.is_file());
        while pool.len() >= 4 {
            pool.pop_first();
        }
        pool.insert(path, (identity, conn));
    }
}

fn open_db(root: &Path) -> Result<Connection> {
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
    db.execute_batch("CREATE INDEX IF NOT EXISTS evidence_schema_code ON evidence(schema_id,code,created DESC); CREATE TABLE IF NOT EXISTS evidence_anchors(event_id TEXT NOT NULL,anchor TEXT NOT NULL,PRIMARY KEY(event_id,anchor)); CREATE INDEX IF NOT EXISTS evidence_anchor_lookup ON evidence_anchors(anchor,event_id); CREATE INDEX IF NOT EXISTS evidence_active_recency ON evidence((origin='') DESC,created DESC) WHERE undone=0; CREATE TABLE IF NOT EXISTS prediction_meta(id INTEGER PRIMARY KEY,path TEXT NOT NULL,sha256 TEXT NOT NULL); CREATE INDEX IF NOT EXISTS evidence_previous_last ON evidence(substr(previous,-1)); CREATE TABLE IF NOT EXISTS history_bigrams(origin TEXT NOT NULL,revision TEXT NOT NULL,previous TEXT NOT NULL,normalized TEXT NOT NULL,text TEXT NOT NULL,count INTEGER NOT NULL,PRIMARY KEY(origin,previous,normalized)) WITHOUT ROWID; CREATE INDEX IF NOT EXISTS history_bigrams_rank ON history_bigrams(previous,normalized,count DESC,origin); CREATE INDEX IF NOT EXISTS evidence_origin_text ON evidence(origin,normalized); CREATE TABLE IF NOT EXISTS history_bigram_totals(previous TEXT NOT NULL,normalized TEXT NOT NULL,text TEXT NOT NULL,count INTEGER NOT NULL,PRIMARY KEY(previous,normalized)) WITHOUT ROWID; CREATE INDEX IF NOT EXISTS history_bigram_totals_rank ON history_bigram_totals(previous,count DESC,normalized); CREATE TRIGGER IF NOT EXISTS history_bigrams_add AFTER INSERT ON history_bigrams BEGIN INSERT INTO history_bigram_totals VALUES(NEW.previous,NEW.normalized,NEW.text,NEW.count) ON CONFLICT(previous,normalized) DO UPDATE SET count=count+excluded.count; END; CREATE TRIGGER IF NOT EXISTS history_bigrams_remove AFTER DELETE ON history_bigrams BEGIN UPDATE history_bigram_totals SET count=count-OLD.count WHERE previous=OLD.previous AND normalized=OLD.normalized; DELETE FROM history_bigram_totals WHERE previous=OLD.previous AND normalized=OLD.normalized AND count<=0; END;").map_err(err)?;
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
    // 方案与码形加一元加号，不让查询按 (schema_id,code,created) 索引扫描全部空方案证据；
    // 按 (origin,normalized) 索引只取该词的证据，结果相同。
    let mut stmt = db.prepare_cached("SELECT previous,weight FROM evidence WHERE normalized=?1 AND +schema_id=?2 AND +code=?3 AND (source_app=?4 OR source_app='') AND origin='' AND undone=0 AND created>?5 ORDER BY created DESC LIMIT 100").map_err(err)?;
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
 if removed>0{tx.execute("DELETE FROM evidence WHERE origin=?1 AND origin_revision=?2",params![origin,expected]).map_err(err)?;tx.execute("DELETE FROM history_bigrams WHERE origin=?1 AND revision=?2",params![origin,expected]).map_err(err)?;bump(&tx)?;}
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
    // 登记了联想表时，空输入联想改用公共二元表与个人证据的合并概率。
    let engine = if q.input_code.is_empty() {
        prediction_engine(&db, &norm(&q.context))?
    } else {
        None
    };
    let completion_prefix = engine
        .as_ref()
        .and_then(|engine| completion_prefix(&norm(&q.context), &engine.tail));
    let mut personal =
        PersonalContinuations::for_context(norm(&q.context), completion_prefix.clone());
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
            // 上屏后联想不读导入整句；前缀条件带上来源，走 (origin,normalized) 索引直接跳过它们。
            predicates.push(if engine.is_some() {
                "(origin='' AND normalized>=? AND normalized<?)".into()
            } else {
                "(normalized>=? AND normalized<?)".into()
            });
            for suffix in suffixes {
                args.push(suffix.into());
            }
            args.push(context.clone().into());
            args.push(format!("{context}\u{10ffff}").into());
        }
        if let Some(prefix) = &completion_prefix {
            predicates.push("(origin='' AND normalized>? AND normalized<?)".into());
            args.push(prefix.clone().into());
            args.push(format!("{prefix}\u{10ffff}").into());
            // 前文以同一个词结尾的证据：按末字走表达式索引，再在内存中核对整词。
            if let Some(last) = prefix.chars().last() {
                predicates.push("substr(previous,-1)=?".into());
                args.push(last.to_string().into());
            }
        }
        if predicates.is_empty() {
            predicates.push("0".into());
        }
        let index_hint = if dense_anchor_matches {
            "INDEXED BY evidence_active_recency"
        } else {
            ""
        };
        // 上屏后联想由历史词对代表已回填的历史；整句导入在开发段上降低命中且要逐条核验来源，
        // 只留给拼音候选重排使用。
        let origin_filter = if engine.is_some() {
            "AND origin=''"
        } else {
            ""
        };
        // 遗忘检查放在最后，隐私摘要只对命中前文条件的行计算，不必为每条证据算一次。
        let sql = format!("SELECT text,normalized,previous,code,weight,created,origin,origin_revision,source_app,schema_id FROM evidence {index_hint} WHERE undone=0 {origin_filter} AND ({}) AND privacy_term_digest(normalized) NOT IN(SELECT text FROM forgotten) ORDER BY (origin='') DESC,created DESC LIMIT {EVIDENCE_QUERY_LIMIT}", predicates.join(" OR "));
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
            let personal_key = (engine.is_some() && weight >= 0.0).then(|| key.clone());
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
            let affinity =
                cached_context_affinity(&context, &previous, &current_anchors, &mut affinities);
            stat.context += contribution * affinity;
            if let Some(key) = personal_key {
                personal.observe(&key, &stat.text, &previous, contribution, affinity);
            }
        }
        skipped_origins = skipped.len();
        if let (Some(_), Some(word)) = (&engine, completion_prefix.as_deref()) {
            let weight = crate::prediction::tuning().history;
            for (text, count) in history_continuations(&db, word)? {
                personal.add_history(norm(&text), &text, count * weight);
            }
        }
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
                if engine.is_some() {
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
        && engine.is_some()
    {
        if let Some(engine) = &engine {
            predictions = blended_predictions(&db, &q, engine, &personal, &suppressed)?;
            let backed: Vec<String> = predictions
                .iter()
                .filter_map(|p| p["text"].as_str().map(norm))
                .filter(|key| personal.history_keys.contains(key))
                .collect();
            if let (Some(word), false) = (completion_prefix.as_deref(), backed.is_empty()) {
                // 撤销会推进学习世代，下方世代复核随即让本轮作废，失效内容不会展示。
                revoke_stale_history_sources(root, &db, word, &backed)?;
            }
        }
    } else if q.input_code.is_empty()
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
    let reply = json!({"enabled":enabled,"epoch":p.epoch,"ordered_ids":candidates.iter().map(|c|&c.id).collect::<Vec<_>>(),"predictions":predictions,"recalled_candidates":recalled_candidates,"context_id":q.context_id,"skipped_unverified_sources":skipped_origins,"evidence_limited":evidence_limited,"warnings":if skipped_origins>0 {vec!["本次相关来源验证达到32条预算，其余来源未参与排序"]}else{vec![]}});
    release_db(root, db);
    Ok(reply)
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
        tx.execute(
            "DELETE FROM history_bigrams WHERE normalized=?1 OR instr(previous,?1)>0",
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
        tx.execute("DELETE FROM history_bigrams", []).map_err(err)?;
        tx.execute("DELETE FROM history_bigram_totals", [])
            .map_err(err)?;
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
            // 已登记联想表时，同时抽取"上一个词→下一个词"供上屏后联想使用。
            let vocabulary = match registered_prediction_table(&db)? {
                Some((path, sha)) => {
                    crate::prediction::with_table(&path, &sha, |table| table.vocabulary())?
                }
                None => None,
            };
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
                let pairs = vocabulary.as_ref().map_or_else(HashMap::new, |vocabulary| {
                    crate::prediction::bigrams(read["text"].as_str().unwrap_or(""), vocabulary)
                });
                if list.is_empty() && pairs.is_empty() {
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
                tx.execute("DELETE FROM history_bigrams WHERE origin=?1", [&origin])
                    .map_err(err)?;
                for ((previous, next), count) in &pairs {
                    tx.execute("INSERT OR IGNORE INTO history_bigrams SELECT ?1,?2,?3,?4,?5,?6 WHERE NOT EXISTS(SELECT 1 FROM forgotten WHERE text IN(privacy_term_digest(?3),privacy_term_digest(?4)))",params![origin,revision,norm(previous),norm(next),next,count]).map_err(err)?;
                }
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

/// 登记随应用打包的联想二元表。只记录路径与摘要，查询时只读打开，不复制进学习库。
pub fn install_prediction_table(root: &Path, path: &Path) -> Result<Value> {
    let gate = root_gate(root)?;
    let _guard = gate.lock().map_err(err)?;
    let path = std::fs::canonicalize(path).map_err(err)?;
    let text = path.to_str().ok_or("联想表路径无效")?.to_owned();
    let sha = crate::prediction::inspect(&path)?;
    let db = db(root)?;
    let old: Option<(String, String)> = db
        .query_row(
            "SELECT path,sha256 FROM prediction_meta WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(err)?;
    if old.is_some_and(|(p, s)| p == text && s == sha) {
        return Ok(json!({"changed":false,"sha256":sha}));
    }
    db.execute(
        "INSERT OR REPLACE INTO prediction_meta VALUES(1,?1,?2)",
        params![text, sha],
    )
    .map_err(err)?;
    Ok(json!({"changed":true,"sha256":sha}))
}

fn registered_prediction_table(db: &Connection) -> Result<Option<(std::path::PathBuf, String)>> {
    db.query_row(
        "SELECT path,sha256 FROM prediction_meta WHERE id=1",
        [],
        |r| {
            Ok((
                std::path::PathBuf::from(r.get::<_, String>(0)?),
                r.get::<_, String>(1)?,
            ))
        },
    )
    .optional()
    .map_err(err)
}

/// 用户确认回填的历史里，前一个词为 word 的下一个词及次数。汇总表由触发器随 history_bigrams
/// 增删维护；来源失效、重新回填、遗忘与清空都会删除对应词对，汇总随之扣减。
fn history_continuations(db: &Connection, word: &str) -> Result<Vec<(String, f64)>> {
    let mut stmt = db
        .prepare_cached("SELECT text,count FROM history_bigram_totals WHERE previous=?1 ORDER BY count DESC,normalized LIMIT 32")
        .map_err(err)?;
    let rows = stmt
        .query_map([word], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?))
        })
        .map_err(err)?;
    rows.collect::<std::result::Result<_, _>>().map_err(err)
}

pub(crate) type SharedSource = std::sync::Arc<std::sync::Mutex<Connection>>;

/// 来源库只读连接按路径与文件身份复用：只读连接每条语句读取最新已提交的数据；
/// 文件被删除或整体替换（如从备份恢复）时丢弃旧连接重新打开。
pub(crate) fn source_db(path: &Path) -> Result<Option<SharedSource>> {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};
    static SOURCES: Mutex<BTreeMap<std::path::PathBuf, (u64, SharedSource)>> =
        Mutex::new(BTreeMap::new());
    let metadata = std::fs::metadata(path).ok().filter(|m| m.is_file());
    let mut cache = SOURCES.lock().map_err(err)?;
    let Some(metadata) = metadata else {
        cache.remove(path);
        return Ok(None);
    };
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        metadata.ino() ^ metadata.dev().rotate_left(32)
    };
    #[cfg(not(unix))]
    let identity = 0;
    if let Some((known, shared)) = cache.get(path) {
        if *known == identity {
            return Ok(Some(shared.clone()));
        }
    }
    let connection = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(err)?;
    connection
        .busy_timeout(std::time::Duration::from_millis(250))
        .map_err(err)?;
    let shared = Arc::new(Mutex::new(connection));
    cache.retain(|known, _| known.is_file());
    while cache.len() >= 12 {
        cache.pop_first();
    }
    cache.insert(path.to_owned(), (identity, shared.clone()));
    Ok(Some(shared))
}

/// 与知识库历史来源开关同一判定（未设置时默认开放），经复用的只读连接读取。
fn history_kind_open(root: &Path, kind: &str) -> Result<bool> {
    let Some(shared) = source_db(&root.join("knowledge/history-access.sqlite"))? else {
        return Ok(true);
    };
    let conn = shared.lock().map_err(err)?;
    conn.query_row(
        "SELECT enabled FROM history_access WHERE kind=?1",
        [kind],
        |r| r.get::<_, bool>(0),
    )
    .optional()
    .map(|enabled| enabled.unwrap_or(true))
    .map_err(err)
}

/// 历史词对的一个来源：(origin, revision, item_id, kind)。
type HistorySource = (String, String, String, String);
/// 展示前每批核对的来源数；常见续写背后可能有上百条来源，找到一条现存的即可。
const HISTORY_CHECK_BATCH: i64 = 8;

/// 展示前核对：每个含历史词对贡献的续写，按次数从高到低分批核对来源，至少有一条现存来源才可展示。
/// 途中发现已删除、已修改、已关闭或应用被排除的来源，立即撤销其全部学习（只删学习库中的派生数据，
/// 不触碰历史原文），判定与后台来源核对一致；撤销推进学习世代，使本轮作废后重查。
/// 未被抽到核对的失效来源仍由后台核对扣除，只影响排序，不会让已删内容单独出现。返回撤销的来源数。
fn revoke_stale_history_sources(
    root: &Path,
    db: &Connection,
    word: &str,
    keys: &[String],
) -> Result<usize> {
    let page = |key: &str, offset: i64| -> Result<Vec<HistorySource>> {
        let mut stmt = db
            .prepare_cached("SELECT b.origin,b.revision,i.item_id,i.kind FROM history_bigrams b JOIN imports i ON i.origin=b.origin AND i.revision=b.revision WHERE b.previous=?1 AND b.normalized=?2 ORDER BY b.count DESC,b.origin LIMIT ?3 OFFSET ?4")
            .map_err(err)?;
        let rows = stmt
            .query_map(params![word, key, HISTORY_CHECK_BATCH, offset], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
            })
            .map_err(err)?;
        rows.collect::<std::result::Result<_, _>>().map_err(err)
    };
    // 先把各续写的第一批合并核对，只为没有现存来源的续写继续翻页。
    let mut pending: Vec<(&String, Vec<HistorySource>, i64)> = Vec::new();
    for key in keys {
        pending.push((key, page(key, 0)?, 0));
    }
    let mut stale: Vec<HistorySource> = Vec::new();
    while !pending.is_empty() {
        let batch: Vec<HistorySource> = pending
            .iter()
            .flat_map(|(_, rows, _)| rows.clone())
            .collect();
        let current = current_history_sources(root, &batch)?;
        let mut next = Vec::new();
        for (key, rows, offset) in pending {
            let (valid, invalid): (Vec<_>, Vec<_>) =
                rows.into_iter().partition(|(_, revision, item, _)| {
                    current.contains(&(item.clone(), revision.clone()))
                });
            let exhausted = valid.is_empty() && invalid.len() < HISTORY_CHECK_BATCH as usize;
            stale.extend(invalid);
            if valid.is_empty() && !exhausted {
                let offset = offset + HISTORY_CHECK_BATCH;
                let rows = page(key, offset)?;
                if !rows.is_empty() {
                    next.push((key, rows, offset));
                }
            }
        }
        pending = next;
    }
    stale.sort();
    stale.dedup();
    if stale.is_empty() {
        return Ok(0);
    }
    let tx = db.unchecked_transaction().map_err(err)?;
    for (origin, revision, item, _) in &stale {
        tx.execute(
            "DELETE FROM imports WHERE origin=?1 AND item_id=?2 AND revision=?3",
            params![origin, item, revision],
        )
        .map_err(err)?;
        tx.execute(
            "DELETE FROM evidence WHERE origin=?1 AND origin_revision=?2",
            params![origin, revision],
        )
        .map_err(err)?;
        tx.execute(
            "DELETE FROM history_bigrams WHERE origin=?1 AND revision=?2",
            params![origin, revision],
        )
        .map_err(err)?;
    }
    bump(&tx)?;
    tx.commit().map_err(err)?;
    Ok(stale.len())
}

/// 一批历史来源中仍然有效的 (item_id, revision)：来源类型已开放、应用未被排除、记录仍存在且修订一致；
/// 语音与剪贴板另核对统一投影背后的源库身份与源记录修订，与历史引用读取一致。
fn current_history_sources(
    root: &Path,
    sources: &[HistorySource],
) -> Result<HashSet<(String, String)>> {
    let mut current: HashSet<(String, String)> = HashSet::new();
    let (typed, history): (Vec<_>, Vec<_>) = sources.iter().partition(|s| s.3 == "typed");
    if !typed.is_empty() && history_kind_open(root, "saved_snippet")? {
        let path = root.join("knowledge/typed.sqlite");
        if let Some(shared) = source_db(&path)? {
            let source = shared.lock().map_err(err)?;
            let ids: Vec<&String> = typed.iter().map(|s| &s.2).collect();
            let mut stmt = source.prepare_cached(&format!("SELECT id,CAST(revision AS TEXT),source_app FROM typed_segments WHERE id IN ({})", vec!["?"; ids.len()].join(","))).map_err(err)?;
            let rows = stmt
                .query_map(params_from_iter(ids), |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })
                .map_err(err)?;
            for row in rows {
                let (id, revision, app) = row.map_err(err)?;
                if safe_app(&app) {
                    current.insert((id, revision));
                }
            }
        }
    }
    let kinds: HashSet<&str> = history.iter().map(|s| s.3.as_str()).collect();
    let open: HashSet<&str> = kinds
        .into_iter()
        .filter(|kind| history_kind_open(root, kind).unwrap_or(false))
        .collect();
    let path = root.join("integration.db");
    let shared = if open.is_empty() {
        None
    } else {
        source_db(&path)?
    };
    if let Some(shared) = shared {
        let index = shared.lock().map_err(err)?;
        let ids: Vec<&str> = history
            .iter()
            .filter_map(|s| s.2.strip_prefix("history:"))
            .collect();
        let mut stmt = index.prepare_cached(&format!("SELECT item_id,CAST(revision AS TEXT),source_kind,COALESCE(json_extract(snapshot,'$.source_app'),''),store_id,record_id,revision FROM integration_items WHERE item_id IN ({})", vec!["?"; ids.len().max(1)].join(","))).map_err(err)?;
        let rows = stmt
            .query_map(params_from_iter(ids), |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, i64>(6)?,
                ))
            })
            .map_err(err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(err)?;
        // 与历史引用读取相同：统一投影存在之外，源库身份一致且源记录仍为同一修订。
        let mut live: HashMap<&str, HashMap<String, i64>> = HashMap::new();
        for kind in &open {
            live.insert(kind, live_source_revisions(root, kind, &rows)?);
        }
        for (id, revision, kind, app, store, record, number) in &rows {
            let source_current = live
                .get(kind.as_str())
                .and_then(|records| records.get(&format!("{store}\0{record}")))
                .is_some_and(|live| live == number);
            if open.contains(kind.as_str()) && source_current && (app.is_empty() || safe_app(app)) {
                current.insert((format!("history:{id}"), revision.clone()));
            }
        }
    }
    Ok(current)
}

/// 源库中仍存在的记录及其修订，键为 "store_id\0record_id"；源库缺失、身份或版本不符时返回空集合。
fn live_source_revisions(
    root: &Path,
    kind: &str,
    rows: &[(String, String, String, String, String, String, i64)],
) -> Result<HashMap<String, i64>> {
    let (file, table, logical) = match kind {
        "voice" => ("history.db", "transcription_history", "history"),
        "clipboard" => ("clipboard.db", "clipboard_history", "clipboard"),
        _ => return Ok(HashMap::new()),
    };
    let Some(shared) = source_db(&root.join(file))? else {
        return Ok(HashMap::new());
    };
    let source = shared.lock().map_err(err)?;
    let meta: Option<(i64, String, String)> = source
        .query_row("SELECT schema_version,store_id,logical_name FROM unified_source_meta WHERE singleton=1", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .optional()
        .unwrap_or(None);
    let Some((schema, store, name)) = meta else {
        return Ok(HashMap::new());
    };
    if schema != crate::source::SOURCE_SCHEMA_VERSION || name != logical {
        return Ok(HashMap::new());
    }
    let records: Vec<&str> = rows
        .iter()
        .filter(|row| row.2 == kind && row.4 == store)
        .map(|row| row.5.as_str())
        .collect();
    if records.is_empty() {
        return Ok(HashMap::new());
    }
    // 先用 s.id=v.record_id 走主键定位（整数主键按数值比较），再核对文本形式完全相同；
    // 只写 CAST(s.id AS TEXT) 会每条全表扫描。
    let mut stmt = source
        .prepare_cached(&format!("SELECT v.record_id,v.revision FROM unified_source_versions v JOIN {table} s ON s.id=v.record_id AND CAST(s.id AS TEXT)=v.record_id WHERE v.record_id IN ({})", vec!["?"; records.len()].join(",")))
        .map_err(err)?;
    let found = stmt
        .query_map(params_from_iter(records), |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })
        .map_err(err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(err)?;
    Ok(found
        .into_iter()
        .map(|(record, revision)| (format!("{store}\0{record}"), revision))
        .collect())
}

struct PredictionEngine {
    path: std::path::PathBuf,
    sha256: String,
    tail: crate::prediction::Tail,
}

fn prediction_engine(db: &Connection, context: &str) -> Result<Option<PredictionEngine>> {
    let Some((path, sha256)) = db
        .query_row(
            "SELECT path,sha256 FROM prediction_meta WHERE id=1",
            [],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(err)?
    else {
        return Ok(None);
    };
    let path = std::path::PathBuf::from(path);
    let tail = crate::prediction::with_table(&path, &sha256, |table| table.tail(context))?;
    Ok(tail.map(|tail| PredictionEngine { path, sha256, tail }))
}

/// 个人补全的前缀：前文末尾的汉语词，或末尾的完整英文词。
fn completion_prefix(context: &str, tail: &crate::prediction::Tail) -> Option<String> {
    if let Some(word) = tail.last_word() {
        return Some(word.to_owned());
    }
    let mut word: Vec<char> = context
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    word.reverse();
    (!word.is_empty() && word.len() <= 64).then(|| word.into_iter().collect())
}

/// 个人证据对每个续写的贡献；总量决定个人层在合并中的权重。
#[derive(Default)]
struct PersonalContinuations {
    /// 规范化后的当前前文，以及其最后一个词（汉语词或完整英文词）。
    context: String,
    prefix: Option<String>,
    total: f64,
    texts: HashMap<String, (String, f64)>,
    /// 含回填历史词对贡献的续写；展示前须核对其来源。
    history_keys: HashSet<String>,
}
impl PersonalContinuations {
    fn for_context(context: String, prefix: Option<String>) -> Self {
        Self {
            context,
            prefix,
            ..Self::default()
        }
    }

    fn add_history(&mut self, key: String, text: &str, amount: f64) {
        if amount > 0.0 {
            self.history_keys.insert(key.clone());
        }
        self.add(key, text, amount);
    }

    fn add(&mut self, key: String, text: &str, amount: f64) {
        if amount <= 0.0 || key.is_empty() {
            return;
        }
        self.total += amount;
        self.texts
            .entry(key)
            .or_insert_with(|| (text.to_owned(), 0.0))
            .1 += amount;
    }

    /// 下一个词只认两种证据：完整前文相同（原权重）与上一个词相同（打折）。
    /// 只有片段重合不说明接下来写什么，不计入；"前文 + 该词"正好是当前前文结尾的，
    /// 就是刚上屏的这一步，再推会重复刚打的字。以前文最后一个词开头的已学短语按一半权重作为补全。
    fn observe(&mut self, key: &str, text: &str, previous: &str, contribution: f64, affinity: f64) {
        let tuning = crate::prediction::tuning();
        let context = self.context.as_str();
        let echo = context.ends_with(&format!("{previous}{key}"));
        let strength = if suffix_context(context, previous) {
            1.0
        } else if self
            .prefix
            .as_deref()
            .is_some_and(|word| ends_with_word(previous, word))
        {
            tuning.last_word
        } else {
            affinity * tuning.anchor
        };
        if key != context && !echo {
            self.add(key.to_owned(), text, contribution * strength);
        }
        let Some(prefix) = self.prefix.clone() else {
            return;
        };
        let prefix = prefix.as_str();
        let Some(rest) = key.strip_prefix(prefix).filter(|rest| !rest.is_empty()) else {
            return;
        };
        // 前缀必须从词首开始；英文不能从词中间截断制造错误补全。
        let boundary = !prefix
            .chars()
            .last()
            .is_some_and(|c| c.is_ascii_alphanumeric())
            || rest
                .chars()
                .next()
                .is_some_and(|c| !c.is_ascii_alphanumeric());
        if boundary {
            let shown = remaining_prefix(text, prefix).unwrap_or(rest).trim();
            if !shown.is_empty() {
                self.add(norm(shown), shown, contribution * 0.5);
            }
        }
    }
}

/// 前文以该词结尾；英文词前面必须是词边界，避免 "shello" 当作以 "hello" 结尾。
fn ends_with_word(previous: &str, word: &str) -> bool {
    let Some(head) = previous.strip_suffix(word) else {
        return false;
    };
    let splits_ascii_word = word
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
        && head
            .chars()
            .last()
            .is_some_and(|c| c.is_ascii_alphanumeric());
    !word.is_empty() && !splits_ascii_word
}

fn forgotten_term(db: &Connection, text: &str) -> Result<bool> {
    db.prepare_cached("SELECT EXISTS(SELECT 1 FROM forgotten WHERE text=privacy_term_digest(?1))")
        .and_then(|mut stmt| stmt.query_row([text], |r| r.get(0)))
        .map_err(err)
}

/// 合并概率 P = (1-λ)·P公共 + λ·P个人，λ = n/(n+κ)。两类候选同一标尺，
/// 个人证据少时不能整页占位；低于阈值的不展示，宁可空框也不给明显错误的续写。
/// 公共表的"？""！"与词同一标尺参与排序（不随个人证据折减），展示中最多一个标点；匹配到的表情固定放在第二位，
/// 不论展示几项前几位都相同，准入重查因此仍能找到已展示的项。
fn blended_predictions(
    db: &Connection,
    q: &Query,
    engine: &PredictionEngine,
    personal: &PersonalContinuations,
    suppressed: &HashSet<String>,
) -> Result<Vec<Value>> {
    let tuning = crate::prediction::tuning();
    let (mut public, marks, emoji) =
        crate::prediction::with_table(&engine.path, &engine.sha256, |table| {
            let emoji = if tuning.emoji > 0.0 {
                table.emoji(&personal.context)?
            } else {
                Vec::new()
            };
            Ok((
                table.continuations(&engine.tail, tuning)?,
                table.punctuation(&engine.tail, &personal.context, tuning)?,
                emoji,
            ))
        })?
        .unwrap_or_default();
    // 只有语气词结尾的问号可以排第一，其余标点最多排在第一个词之后。
    let trailing: HashSet<String> = marks
        .iter()
        .filter(|(_, _, lead)| !lead)
        .map(|(mark, _, _)| norm(mark))
        .collect();
    public.extend(marks.into_iter().map(|(mark, p, _)| (mark, p)));
    // 证据不足的个人续写既不展示，也不抬高个人层权重。
    let retained: Vec<_> = personal
        .texts
        .iter()
        .filter(|(_, (_, amount))| *amount >= tuning.min_personal)
        .collect();
    let total: f64 = retained.iter().map(|(_, (_, amount))| amount).sum();
    let lambda = if total > 0.0 {
        total / (total + tuning.kappa)
    } else {
        0.0
    };
    let mut merged: HashMap<String, (String, f64, f64)> = HashMap::new();
    for (text, probability) in public {
        merged
            .entry(norm(&text))
            .or_insert_with(|| (text.clone(), 0.0, 0.0))
            .1 += probability;
    }
    for (key, (text, amount)) in retained {
        merged
            .entry(key.clone())
            .or_insert_with(|| (text.clone(), 0.0, 0.0))
            .2 += amount / total;
    }
    let word = engine.tail.last_word().unwrap_or("");
    let mut ranked: Vec<_> = merged
        .into_iter()
        .map(|(key, (text, public, own))| {
            // 个人层（含历史词对）在标点处断开，对标点没有反面证据；标点不随个人证据总量折减，
            // 用户自己选过的标点再按个人比例加分。
            let score = if crate::prediction::is_punctuation(&key) {
                public + lambda * own
            } else {
                (1.0 - lambda) * public + lambda * own
            };
            (score, key, text, own > 0.0)
        })
        .filter(|(score, key, _, _)| *score >= tuning.tau && !suppressed.contains(key))
        .collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    // 按分数从高到低逐项核对拒绝与遗忘，凑满展示数即停，不为排不上的候选查库。
    let limit = q.limit.clamp(1, 10);
    let mut shown = Vec::with_capacity(limit);
    let mut punctuation_shown = false;
    let mut held = None;
    for (_, key, text, own) in ranked {
        if shown.len() == limit {
            break;
        }
        let punctuation = crate::prediction::is_punctuation(&key);
        if (punctuation && punctuation_shown)
            || scoped_rejected(db, q, &key)?
            || forgotten_term(db, &key)?
            || (!word.is_empty() && forgotten_term(db, &format!("{word}{key}"))?)
        {
            continue;
        }
        punctuation_shown |= punctuation;
        let source = if own { "learned" } else { "base" };
        let item = json!({"id":format!("{source}:{}",digest(&text)),"text":text});
        if shown.is_empty() && trailing.contains(&key) && !own {
            held = Some(item);
            continue;
        }
        shown.push(item);
        if let Some(item) = held.take() {
            shown.push(item);
        }
    }
    shown.extend(held);
    shown.truncate(limit);
    for text in emoji {
        let key = norm(&text);
        if shown.iter().any(|item| item["text"] == text.as_str()) {
            break;
        }
        if suppressed.contains(&key) || scoped_rejected(db, q, &key)? || forgotten_term(db, &key)? {
            continue;
        }
        shown.insert(
            shown.len().min(1),
            json!({"id":format!("base:{}",digest(&text)),"text":text}),
        );
        shown.truncate(limit);
        break;
    }
    Ok(shown)
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
