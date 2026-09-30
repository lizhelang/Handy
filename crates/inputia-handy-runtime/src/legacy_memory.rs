//! 旧派生学习域由历史服务单写者拥有；没有旧写者交接证据时不打开旧库。
use inputia_core::{
    memory_snapshot::{read_query_snapshot, MemoryQuery},
    AppContext, AppPolicy, MemoryTerm,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::{cell::Cell, path::PathBuf};
type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryOrigin {
    Typed,
    Voice,
    Clipboard,
}
impl MemoryOrigin {
    fn name(self) -> &'static str {
        match self {
            Self::Typed => "typed",
            Self::Voice => "voice",
            Self::Clipboard => "clipboard",
        }
    }
    fn counts(self) -> (u64, u64, u64) {
        match self {
            Self::Typed => (1, 0, 0),
            Self::Voice => (0, 1, 0),
            Self::Clipboard => (0, 0, 1),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryIntent {
    pub operation_id: String,
    pub event_id: String,
    pub expected_epoch: u64,
    pub source: MemoryOrigin,
    pub text: String,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryImportSelection {
    History,
    Clipboard,
    Both,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MemoryImportRequest {
    pub operation_id: String,
    pub expected_epoch: u64,
    pub selection: MemoryImportSelection,
    pub limit: usize,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryMutationState {
    Applied,
    AlreadyContributed,
    Revoked,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryMutationReceipt {
    pub operation_id: String,
    pub applied_at_epoch: u64,
    pub domain_uuid: String,
    pub generation: u64,
    pub state: MemoryMutationState,
    pub replayed: bool,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryImportState {
    Accepted,
    Processing,
    Completed,
    PartialFailure,
    Revoked,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryImportStatus {
    pub operation_id: String,
    pub applied_at_epoch: u64,
    pub state: MemoryImportState,
    pub history_imported: usize,
    pub clipboard_imported: usize,
    pub skipped: usize,
    pub failure: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MemoryOperationStatus {
    Learn(MemoryMutationReceipt),
    Import(MemoryImportStatus),
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryDomainState {
    NotConfigured,
    HandoffRequired,
    Ready,
    RepairRequired,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryCoverage {
    PrimaryOnly,
    AllDomains,
    LegacyCoverageUnresolved,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryDomainStatus {
    pub state: MemoryDomainState,
    pub domain_uuid: Option<String>,
    pub generation: u64,
    pub policy_epoch: u64,
    pub coverage: MemoryCoverage,
    pub reason: Option<String>,
}
pub struct MemoryQueryLease {
    pub terms: Vec<MemoryTerm>,
    pub domain_uuid: String,
    pub generation: u64,
    pub epoch: u64,
    pub max_age_ms: u64,
}

/// 生产调用者只能声明固定 profile 路径。旧写者停写证明接入前，不存在布尔授权捷径。
#[derive(Clone)]
pub struct LegacyMemoryContext {
    path: Option<PathBuf>,
    profile_id: String,
    exclusive: bool,
}
impl LegacyMemoryContext {
    pub fn unconfigured() -> Self {
        Self {
            path: None,
            profile_id: String::new(),
            exclusive: false,
        }
    }
    pub fn handoff_required(path: PathBuf, profile_id: String) -> Self {
        Self {
            path: Some(path),
            profile_id,
            exclusive: false,
        }
    }
    pub(crate) fn configured(&self) -> bool {
        self.path.is_some()
    }
    #[cfg(test)]
    pub(crate) fn fixture(path: PathBuf, profile_id: String) -> Self {
        Self {
            path: Some(path),
            profile_id,
            exclusive: true,
        }
    }
}

/// 不能从 wire 反序列化。源证据由同一 worker 读取，提交证据仅供认证后的真实读回适配器构造。
#[derive(Clone)]
pub struct VerifiedMemoryEvidence {
    pub(crate) store_id: String,
    pub(crate) record_id: String,
    pub(crate) revision: u64,
    pub(crate) epoch: u64,
    pub(crate) source: MemoryOrigin,
    text: String,
    app: String,
}
impl VerifiedMemoryEvidence {
    pub fn confirmed_commit(
        commit_id: String,
        target_id: String,
        committed_text: String,
        observed_text: String,
        epoch: u64,
        source_app: String,
        policy: &AppPolicy,
    ) -> Result<Self> {
        valid_id(&commit_id)?;
        valid_id(&target_id)?;
        if committed_text != observed_text || policy.excludes(&AppContext::new(&source_app)) {
            return Err("memory_commit_unverified".into());
        }
        valid_text(&committed_text)?;
        valid_epoch(epoch)?;
        Ok(Self {
            store_id: format!("commit:{target_id}"),
            record_id: commit_id,
            revision: 1,
            epoch,
            source: MemoryOrigin::Typed,
            text: committed_text,
            app: source_app,
        })
    }
    pub(crate) fn source(
        store_id: String,
        record_id: String,
        revision: u64,
        epoch: u64,
        source: MemoryOrigin,
        text: String,
        app: String,
    ) -> Result<Self> {
        valid_id(&store_id)?;
        valid_id(&record_id)?;
        valid_epoch(revision)?;
        valid_epoch(epoch)?;
        valid_text(&text)?;
        Ok(Self {
            store_id,
            record_id,
            revision,
            epoch,
            source,
            text,
            app,
        })
    }
    #[cfg(test)]
    pub(crate) fn intent(&self, id: String) -> MemoryIntent {
        MemoryIntent {
            operation_id: id.clone(),
            event_id: id,
            expected_epoch: self.epoch,
            source: self.source,
            text: self.text.clone(),
        }
    }
}
fn valid_id(value: &str) -> Result<()> {
    inputia_core::integration::events::Identifier::parse(value)
        .map(|_| ())
        .map_err(|_| "memory_identifier_invalid".into())
}
fn valid_epoch(value: u64) -> Result<()> {
    if value > 0 && value < i64::MAX as u64 {
        Ok(())
    } else {
        Err("memory_epoch_invalid".into())
    }
}
fn display_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}
fn valid_text(value: &str) -> Result<()> {
    let text = display_text(value);
    if text.is_empty() || text.len() > 64 * 1024 || text.chars().any(char::is_control) {
        Err("memory_text_invalid".into())
    } else {
        Ok(())
    }
}
fn db_err(_: rusqlite::Error) -> String {
    "memory_storage_unavailable".into()
}
fn hmac(key: &[u8], scope: &[u8], bytes: &[u8]) -> Vec<u8> {
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key);
    let mut ctx = ring::hmac::Context::with_key(&key);
    ctx.update(b"inputia-legacy-memory-v1\0");
    ctx.update(scope);
    ctx.update(b"\0");
    ctx.update(bytes);
    ctx.sign().as_ref().to_vec()
}
fn term_id(key: &[u8], text: &str) -> Vec<u8> {
    hmac(
        key,
        b"term-whitespace-lowercase-v1",
        display_text(text).to_lowercase().as_bytes(),
    )
}
fn new_uuid() -> Result<String> {
    let mut b = [0u8; 16];
    getrandom::getrandom(&mut b).map_err(|_| "memory_random_unavailable")?;
    b[6] = (b[6] & 15) | 64;
    b[8] = (b[8] & 63) | 128;
    let h: String = b.iter().map(|v| format!("{v:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    ))
}

struct ImportRecord {
    request: MemoryImportRequest,
    status: MemoryImportStatus,
    history_store: Option<String>,
    clipboard_store: Option<String>,
}
struct ImportRow {
    request: String,
    status: String,
    history: Option<String>,
    clipboard: Option<String>,
    digest: Vec<u8>,
}
pub(crate) struct PendingMemoryImport {
    pub request: MemoryImportRequest,
    pub clipboard: bool,
    pub cursor: Option<String>,
    pub store: String,
}
pub(crate) struct LegacyMemory {
    context: LegacyMemoryContext,
    db: Option<Connection>,
    key: [u8; 32],
    domain: Option<String>,
    failure: Option<String>,
    observed_version: Cell<(u64, u64)>,
}
impl LegacyMemory {
    pub(crate) fn unavailable(context: LegacyMemoryContext, key: [u8; 32], reason: String) -> Self {
        Self {
            context,
            db: None,
            key,
            domain: None,
            failure: Some(reason),
            observed_version: Cell::new((0, 0)),
        }
    }
    pub(crate) fn open(
        context: LegacyMemoryContext,
        key: [u8; 32],
        epoch: u64,
        expected_domain: Option<&str>,
    ) -> Result<Self> {
        if !context.exclusive {
            return Ok(Self {
                context,
                db: None,
                key,
                domain: None,
                failure: None,
                observed_version: Cell::new((0, 0)),
            });
        }
        let path = context.path.as_ref().ok_or("memory_handoff_required")?;
        #[cfg(unix)]
        if path.exists() {
            use std::os::unix::fs::MetadataExt;
            let md = std::fs::symlink_metadata(path).map_err(|_| "memory_storage_unavailable")?;
            if !md.is_file() || md.nlink() != 1 || md.uid() != unsafe { libc::geteuid() } {
                return Err("memory_unsafe_path".into());
            }
        }
        if expected_domain.is_some() && !path.exists() {
            return Err("memory_domain_missing".into());
        }
        let mut db = Connection::open(path).map_err(db_err)?;
        let function_key = key;
        db.create_scalar_function(
            "memory_term_id",
            1,
            rusqlite::functions::FunctionFlags::SQLITE_UTF8
                | rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
            move |ctx| {
                let text: String = ctx.get(0)?;
                Ok(term_id(&function_key, &text))
            },
        )
        .map_err(db_err)?;
        db.busy_timeout(std::time::Duration::from_millis(100))
            .map_err(db_err)?;
        db.pragma_update(None, "journal_mode", "WAL")
            .map_err(db_err)?;
        db.pragma_update(None, "synchronous", "FULL")
            .map_err(db_err)?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_err)?;
        tx.execute_batch("CREATE TABLE IF NOT EXISTS inputia_terms(text TEXT PRIMARY KEY,typed_count INTEGER NOT NULL DEFAULT 0,voice_count INTEGER NOT NULL DEFAULT 0,clipboard_count INTEGER NOT NULL DEFAULT 0,last_used_tick INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE IF NOT EXISTS inputia_events(id INTEGER PRIMARY KEY AUTOINCREMENT,source TEXT NOT NULL,text TEXT,app_bundle_id TEXT NOT NULL,privacy_decision TEXT NOT NULL,created_tick INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS memory_domain_meta(singleton INTEGER PRIMARY KEY CHECK(singleton=1),domain_uuid TEXT NOT NULL,profile_id TEXT NOT NULL,key_id BLOB NOT NULL,normalization_version INTEGER NOT NULL,schema_version INTEGER NOT NULL,privacy_epoch INTEGER NOT NULL,generation INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS memory_clock(singleton INTEGER PRIMARY KEY CHECK(singleton=1),tick INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS memory_operations(operation_id TEXT PRIMARY KEY,request_hmac BLOB NOT NULL,origin TEXT NOT NULL,epoch INTEGER NOT NULL,result TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS memory_event_receipts(event_id TEXT PRIMARY KEY,request_hmac BLOB NOT NULL);
        CREATE TABLE IF NOT EXISTS memory_word_spans(span_id TEXT PRIMARY KEY,identity_hmac BLOB NOT NULL,revision INTEGER NOT NULL,epoch INTEGER NOT NULL,revoked INTEGER NOT NULL,sealed INTEGER NOT NULL DEFAULT 0);
        CREATE TABLE IF NOT EXISTS memory_sources(store_id TEXT NOT NULL,record_id TEXT NOT NULL,revision INTEGER NOT NULL,deleted INTEGER NOT NULL,PRIMARY KEY(store_id,record_id));
        CREATE TABLE IF NOT EXISTS memory_contributions(store_id TEXT NOT NULL,record_id TEXT NOT NULL,revision INTEGER NOT NULL,event_id TEXT NOT NULL,term_hmac BLOB NOT NULL,text TEXT NOT NULL,source TEXT NOT NULL,typed_count INTEGER NOT NULL,voice_count INTEGER NOT NULL,clipboard_count INTEGER NOT NULL,tick INTEGER NOT NULL,PRIMARY KEY(store_id,record_id,revision,term_hmac));
        CREATE INDEX IF NOT EXISTS memory_contributions_text ON memory_contributions(text);
        CREATE TABLE IF NOT EXISTS memory_event_sources(event_row INTEGER PRIMARY KEY,store_id TEXT NOT NULL,record_id TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS memory_forgotten(term_hmac BLOB PRIMARY KEY,epoch INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS memory_source_revocations(operation_id TEXT PRIMARY KEY,request_hmac BLOB NOT NULL,domain_uuid TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS memory_privacy_receipts(operation_id TEXT PRIMARY KEY,request_hmac BLOB NOT NULL,epoch INTEGER NOT NULL,domain_uuid TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS memory_imports(operation_id TEXT PRIMARY KEY,request_hmac BLOB NOT NULL,request TEXT NOT NULL,result TEXT NOT NULL,history_cursor TEXT,clipboard_cursor TEXT,history_done INTEGER NOT NULL DEFAULT 0,clipboard_done INTEGER NOT NULL DEFAULT 0,history_store TEXT,clipboard_store TEXT,history_scanned INTEGER NOT NULL DEFAULT 0,clipboard_scanned INTEGER NOT NULL DEFAULT 0);").map_err(db_err)?;
        let columns: Vec<String> = tx
            .prepare("PRAGMA table_info(memory_word_spans)")
            .map_err(db_err)?
            .query_map([], |r| r.get(1))
            .map_err(db_err)?
            .collect::<std::result::Result<_, _>>()
            .map_err(db_err)?;
        if !columns.iter().any(|name| name == "sealed") {
            tx.execute(
                "ALTER TABLE memory_word_spans ADD COLUMN sealed INTEGER NOT NULL DEFAULT 0",
                [],
            )
            .map_err(db_err)?;
        }
        let existing:Option<(String,String,Vec<u8>,u32,u32)>=tx.query_row("SELECT domain_uuid,profile_id,key_id,normalization_version,schema_version FROM memory_domain_meta WHERE singleton=1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional().map_err(db_err)?;
        let check = hmac(&key, b"key-id", b"");
        let domain = match existing {
            Some((id, profile, key_id, 1, 1))
                if profile == context.profile_id
                    && key_id == check
                    && expected_domain.is_none_or(|v| v == id) =>
            {
                id
            }
            Some(_) => return Err("memory_domain_conflict".into()),
            None if expected_domain.is_some() => return Err("memory_domain_missing".into()),
            None => {
                let id = new_uuid()?;
                tx.execute(
                    "INSERT INTO memory_domain_meta VALUES(1,?1,?2,?3,1,1,?4,1)",
                    params![id, context.profile_id, check, epoch],
                )
                .map_err(db_err)?;
                // 旧计数仅标记一次迁移来源，不伪造历史record来源；迁移保留原排名计数。
                let rows:Vec<(String,u64,u64,u64,u64)>=tx.prepare("SELECT text,typed_count,voice_count,clipboard_count,last_used_tick FROM inputia_terms").map_err(db_err)?.query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).map_err(db_err)?.collect::<std::result::Result<_,_>>().map_err(db_err)?;
                for (text, typed, voice, clipboard, tick) in rows {
                    valid_text(&text)?;
                    let tid = term_id(&key, &text);
                    tx.execute("INSERT INTO memory_contributions VALUES(?1,hex(?8),1,'migration',?2,?3,'migration',?4,?5,?6,?7)",params![format!("migration:{id}"),tid,text,typed,voice,clipboard,tick,hmac(&key,b"migration-display",display_text(&text).as_bytes())]).map_err(db_err)?;
                }
                id
            }
        };
        // 新单写者实例不继承内存许可；未sealed旧span必须先撤销，迟到证据不能复活。
        let abandoned: Vec<String> = tx
            .prepare("SELECT span_id FROM memory_word_spans WHERE revoked=0 AND sealed=0")
            .map_err(db_err)?
            .query_map([], |r| r.get(0))
            .map_err(db_err)?
            .collect::<std::result::Result<_, _>>()
            .map_err(db_err)?;
        for span in abandoned {
            clear_span(&tx, &format!("commit:span:{span}"))?;
            tx.execute(
                "UPDATE memory_word_spans SET revoked=1,revision=revision+1 WHERE span_id=?1",
                [span],
            )
            .map_err(db_err)?;
        }
        // 版本用于失效快照；业务时钟独立继承旧词和事件的最近使用顺序。
        tx.execute_batch("UPDATE memory_domain_meta SET generation=MAX(1,generation) WHERE singleton=1;
        INSERT INTO memory_clock SELECT 1,MAX(COALESCE((SELECT MAX(last_used_tick) FROM inputia_terms),0),COALESCE((SELECT MAX(created_tick) FROM inputia_events),0),COALESCE((SELECT MAX(tick) FROM memory_contributions),0)) WHERE 1
        ON CONFLICT(singleton) DO UPDATE SET tick=MAX(tick,excluded.tick);").map_err(db_err)?;
        let version = tx
            .query_row(
                "SELECT privacy_epoch,generation FROM memory_domain_meta WHERE singleton=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(db_err)?;
        tx.commit().map_err(db_err)?;
        Ok(Self {
            context,
            db: Some(db),
            key,
            domain: Some(domain),
            failure: None,
            observed_version: Cell::new(version),
        })
    }
    fn db(&self) -> Result<&Connection> {
        let db = self.db.as_ref().ok_or_else(|| {
            self.failure.clone().unwrap_or_else(|| {
                if self.context.configured() {
                    "memory_handoff_required"
                } else {
                    "memory_not_configured"
                }
                .into()
            })
        })?;
        #[cfg(unix)]
        if let Some(path) = db.path() {
            use std::os::unix::fs::MetadataExt;
            let md = std::fs::symlink_metadata(path).map_err(|_| "memory_domain_missing")?;
            if !md.is_file() || md.nlink() != 1 || md.uid() != unsafe { libc::geteuid() } {
                return Err("memory_unsafe_path".into());
            }
            let mut moved = 0;
            let result = unsafe {
                rusqlite::ffi::sqlite3_file_control(
                    db.handle(),
                    c"main".as_ptr(),
                    rusqlite::ffi::SQLITE_FCNTL_HAS_MOVED,
                    (&mut moved as *mut i32).cast(),
                )
            };
            if result != rusqlite::ffi::SQLITE_OK || moved != 0 {
                return Err("memory_domain_replaced".into());
            }
        }
        let (domain,key,profile,version):(String,Vec<u8>,String,u32)=db.query_row("SELECT domain_uuid,key_id,profile_id,normalization_version FROM memory_domain_meta WHERE singleton=1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(db_err)?;
        if self.domain.as_deref() != Some(domain.as_str())
            || key != hmac(&self.key, b"key-id", b"")
            || profile != self.context.profile_id
            || version != 1
        {
            return Err("memory_domain_conflict".into());
        }
        let version: (u64, u64) = db
            .query_row(
                "SELECT privacy_epoch,generation FROM memory_domain_meta WHERE singleton=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(db_err)?;
        let prior = self.observed_version.get();
        if version.0 < prior.0 || version.1 < prior.1 {
            return Err("memory_domain_rolled_back".into());
        }
        self.observed_version.set(version);
        Ok(db)
    }
    pub(crate) fn status(&self) -> Result<MemoryDomainStatus> {
        let (generation, epoch) = match &self.db {
            Some(_) => self
                .db()?
                .query_row(
                    "SELECT generation,privacy_epoch FROM memory_domain_meta WHERE singleton=1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(db_err)?,
            None => (0, 0),
        };
        Ok(MemoryDomainStatus {
            state: if self.failure.is_some() {
                MemoryDomainState::RepairRequired
            } else if self.db.is_some() {
                MemoryDomainState::Ready
            } else if self.context.configured() {
                MemoryDomainState::HandoffRequired
            } else {
                MemoryDomainState::NotConfigured
            },
            domain_uuid: self.domain.clone(),
            generation,
            policy_epoch: epoch,
            coverage: if self.context.configured() {
                MemoryCoverage::AllDomains
            } else {
                MemoryCoverage::PrimaryOnly
            },
            reason: self.failure.clone().or_else(|| {
                self.db.is_none().then(|| {
                    if self.context.configured() {
                        "memory_handoff_required"
                    } else {
                        "memory_not_configured"
                    }
                    .into()
                })
            }),
        })
    }
    pub(crate) fn configured(&self) -> bool {
        self.context.configured()
    }
    fn require_epoch(&self, epoch: u64) -> Result<()> {
        if self.status()?.policy_epoch != epoch {
            Err("memory_epoch_revoked".into())
        } else {
            Ok(())
        }
    }
    pub(crate) fn query(&self, query: &MemoryQuery, epoch: u64) -> Result<MemoryQueryLease> {
        self.require_epoch(epoch)?;
        let snap = read_query_snapshot(self.db()?, query).map_err(|_| "memory_query_invalid")?;
        let status = self.status()?;
        Ok(MemoryQueryLease {
            terms: snap.terms().to_vec(),
            domain_uuid: status.domain_uuid.ok_or("memory_domain_missing")?,
            generation: status.generation,
            epoch,
            max_age_ms: crate::privacy_operation::READER_LEASE_MS,
        })
    }
    pub(crate) fn operation(&self, id: &str) -> Result<Option<MemoryOperationStatus>> {
        valid_id(id)?;
        if let Some(record) = self.import_record(id)? {
            return Ok(Some(MemoryOperationStatus::Import(record.status)));
        }
        let row: Option<String> = self
            .db()?
            .query_row(
                "SELECT result FROM memory_operations WHERE operation_id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_err)?;
        row.map(|s| serde_json::from_str(&s).map_err(|_| "memory_receipt_invalid".into()))
            .transpose()
    }
    pub(crate) fn apply_intent(
        &mut self,
        intent: &MemoryIntent,
        evidence: &VerifiedMemoryEvidence,
    ) -> Result<MemoryMutationReceipt> {
        valid_id(&intent.operation_id)?;
        valid_id(&intent.event_id)?;
        valid_text(&intent.text)?;
        if intent.expected_epoch != evidence.epoch
            || intent.source != evidence.source
            || display_text(&intent.text) != display_text(&evidence.text)
        {
            return Err("memory_evidence_mismatch".into());
        }
        let bytes = serde_json::to_vec(&(
            intent,
            &evidence.store_id,
            &evidence.record_id,
            evidence.revision,
        ))
        .map_err(|_| "memory_request_invalid")?;
        let digest = hmac(&self.key, b"intent", &bytes);
        if let Some((prior, response)) = self
            .db()?
            .query_row(
                "SELECT request_hmac,result FROM memory_operations WHERE operation_id=?1",
                [&intent.operation_id],
                |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(db_err)?
        {
            if prior != digest {
                return Err("memory_operation_conflict".into());
            }
            let MemoryOperationStatus::Learn(mut receipt) =
                serde_json::from_str(&response).map_err(|_| "memory_receipt_invalid")?
            else {
                return Err("memory_operation_conflict".into());
            };
            receipt.replayed = true;
            return Ok(receipt);
        }
        if self.operation(&intent.operation_id)?.is_some() {
            return Err("memory_operation_conflict".into());
        }
        self.require_epoch(intent.expected_epoch)?;
        let key = self.key;
        let domain = self.domain.clone().ok_or("memory_domain_missing")?;
        let db = self.db.as_mut().ok_or("memory_not_configured")?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_err)?;
        let event_digest = hmac(
            &key,
            b"event",
            &serde_json::to_vec(&(
                intent.expected_epoch,
                intent.source,
                &intent.text,
                &evidence.store_id,
                &evidence.record_id,
                evidence.revision,
            ))
            .map_err(|_| "memory_request_invalid")?,
        );
        let previous: Option<Vec<u8>> = tx
            .query_row(
                "SELECT request_hmac FROM memory_event_receipts WHERE event_id=?1",
                [&intent.event_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_err)?;
        let state = if let Some(previous) = previous {
            if previous != event_digest {
                return Err("memory_event_conflict".into());
            }
            MemoryMutationState::AlreadyContributed
        } else {
            let result = contribute(&tx, &key, evidence, &intent.event_id)?;
            tx.execute(
                "INSERT INTO memory_event_receipts VALUES(?1,?2)",
                params![intent.event_id, event_digest],
            )
            .map_err(db_err)?;
            result
        };
        let generation: u64 = tx
            .query_row(
                "SELECT generation FROM memory_domain_meta WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .map_err(db_err)?;
        let receipt = MemoryMutationReceipt {
            operation_id: intent.operation_id.clone(),
            applied_at_epoch: intent.expected_epoch,
            domain_uuid: domain,
            generation,
            state,
            replayed: false,
        };
        tx.execute(
            "INSERT INTO memory_operations VALUES(?1,?2,'learn',?3,?4)",
            params![
                intent.operation_id,
                digest,
                intent.expected_epoch,
                serde_json::to_string(&MemoryOperationStatus::Learn(receipt.clone()))
                    .map_err(|_| "memory_receipt_invalid")?
            ],
        )
        .map_err(db_err)?;
        tx.commit().map_err(db_err)?;
        self.db()?;
        Ok(receipt)
    }
}
fn rebuild(db: &Connection) -> Result<()> {
    db.execute_batch("DELETE FROM inputia_terms; INSERT INTO inputia_terms SELECT text,MIN(4294967295,SUM(typed_count)),MIN(4294967295,SUM(voice_count)),MIN(4294967295,SUM(clipboard_count)),MAX(tick) FROM memory_contributions GROUP BY text;
    UPDATE memory_domain_meta SET generation=generation+1 WHERE singleton=1;").map_err(db_err)
}
fn contribute(
    db: &Connection,
    key: &[u8],
    e: &VerifiedMemoryEvidence,
    event_id: &str,
) -> Result<MemoryMutationState> {
    let old: Option<(u64, bool)> = db
        .query_row(
            "SELECT revision,deleted FROM memory_sources WHERE store_id=?1 AND record_id=?2",
            params![e.store_id, e.record_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(db_err)?;
    if old.is_some_and(|(rev, deleted)| deleted || rev > e.revision) {
        return Ok(MemoryMutationState::Revoked);
    }
    if old.is_some_and(|(rev, _)| rev < e.revision) {
        revoke_record(db, &e.store_id, &e.record_id, e.revision, false)?;
    }
    let tid = term_id(key, &e.text);
    let forgotten: bool = db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM memory_forgotten WHERE term_hmac=?1)",
            [&tid],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    if forgotten {
        return Ok(MemoryMutationState::Revoked);
    }
    db.execute("INSERT INTO memory_sources VALUES(?1,?2,?3,0) ON CONFLICT(store_id,record_id) DO UPDATE SET revision=excluded.revision",params![e.store_id,e.record_id,e.revision]).map_err(db_err)?;
    let (typed, voice, clipboard) = e.source.counts();
    let tick: u64 = db
        .query_row("SELECT tick FROM memory_clock WHERE singleton=1", [], |r| {
            r.get(0)
        })
        .map_err(db_err)?;
    let tick = tick
        .checked_add(1)
        .filter(|v| *v <= i64::MAX as u64)
        .ok_or("memory_clock_exhausted")?;
    let inserted = db
        .execute(
            "INSERT OR IGNORE INTO memory_contributions VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                e.store_id,
                e.record_id,
                e.revision,
                event_id,
                tid,
                display_text(&e.text),
                e.source.name(),
                typed,
                voice,
                clipboard,
                tick
            ],
        )
        .map_err(db_err)?;
    if inserted == 0 {
        return Ok(MemoryMutationState::AlreadyContributed);
    }
    db.execute("UPDATE memory_clock SET tick=?1 WHERE singleton=1", [tick])
        .map_err(db_err)?;
    db.execute("INSERT INTO inputia_events(source,text,app_bundle_id,privacy_decision,created_tick) VALUES(?1,?2,?3,'learn',?4)",params![e.source.name(),display_text(&e.text),e.app,tick]).map_err(db_err)?;
    db.execute(
        "INSERT INTO memory_event_sources VALUES(?1,?2,?3)",
        params![db.last_insert_rowid(), e.store_id, e.record_id],
    )
    .map_err(db_err)?;
    db.execute("INSERT INTO inputia_terms VALUES(?1,?2,?3,?4,?5) ON CONFLICT(text) DO UPDATE SET typed_count=MIN(4294967295,typed_count+excluded.typed_count),voice_count=MIN(4294967295,voice_count+excluded.voice_count),clipboard_count=MIN(4294967295,clipboard_count+excluded.clipboard_count),last_used_tick=excluded.last_used_tick",params![display_text(&e.text),typed,voice,clipboard,tick]).map_err(db_err)?;
    db.execute(
        "UPDATE memory_domain_meta SET generation=generation+1 WHERE singleton=1",
        [],
    )
    .map_err(db_err)?;
    Ok(MemoryMutationState::Applied)
}
fn revoke_record(
    db: &Connection,
    store: &str,
    record: &str,
    revision: u64,
    deleted: bool,
) -> Result<()> {
    db.execute("DELETE FROM inputia_events WHERE id IN(SELECT event_row FROM memory_event_sources WHERE store_id=?1 AND record_id=?2)",params![store,record]).map_err(db_err)?;
    db.execute(
        "DELETE FROM memory_event_sources WHERE store_id=?1 AND record_id=?2",
        params![store, record],
    )
    .map_err(db_err)?;
    db.execute(
        "DELETE FROM memory_contributions WHERE store_id=?1 AND record_id=?2",
        params![store, record],
    )
    .map_err(db_err)?;
    db.execute("INSERT INTO memory_sources VALUES(?1,?2,?3,?4) ON CONFLICT(store_id,record_id) DO UPDATE SET revision=max(revision,excluded.revision),deleted=max(deleted,excluded.deleted)",params![store,record,revision,deleted]).map_err(db_err)?;
    rebuild(db)
}
impl LegacyMemory {
    pub(crate) fn advance_epoch(&mut self, epoch: u64) -> Result<()> {
        valid_epoch(epoch)?;
        let db = self.db()?;
        let current: u64 = db
            .query_row(
                "SELECT privacy_epoch FROM memory_domain_meta WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .map_err(db_err)?;
        if epoch < current {
            return Err("memory_epoch_revoked".into());
        }
        db.execute(
            "UPDATE memory_domain_meta SET privacy_epoch=?1 WHERE singleton=1",
            [epoch],
        )
        .map_err(db_err)?;
        self.db()?;
        Ok(())
    }
    pub(crate) fn privacy_receipt(
        &self,
        id: &str,
        digest: &[u8],
        epoch: u64,
        domain: &str,
    ) -> Result<bool> {
        if self.domain.as_deref() != Some(domain) {
            return Err("memory_domain_conflict".into());
        }
        let row:Option<(Vec<u8>,u64,String)>=self.db()?.query_row("SELECT request_hmac,epoch,domain_uuid FROM memory_privacy_receipts WHERE operation_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(db_err)?;
        match row {
            None => Ok(false),
            Some((hash, version, id))
                if hash == digest
                    && version == epoch
                    && id == domain
                    && self.status()?.policy_epoch >= epoch =>
            {
                Ok(true)
            }
            Some(_) => Err("memory_privacy_conflict".into()),
        }
    }
    pub(crate) fn apply_privacy(
        &mut self,
        request: &crate::privacy_operation::PrivacyRequest,
        digest: &[u8],
        epoch: u64,
    ) -> Result<()> {
        request.validate()?;
        if digest.len() != 32 || epoch != request.expected_epoch + 1 {
            return Err("memory_privacy_invalid".into());
        }
        let domain = self.domain.clone().ok_or("memory_handoff_required")?;
        if self.privacy_receipt(&request.operation_id, digest, epoch, &domain)? {
            return Ok(());
        }
        let current = self.status()?.policy_epoch;
        if current >= epoch {
            return Err("memory_privacy_receipt_missing".into());
        }
        let key = self.key;
        let db = self.db.as_mut().ok_or("memory_handoff_required")?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_err)?;
        match &request.scope {
            crate::privacy_operation::PrivacyScope::ForgetTerm { term } => {
                let tid = term_id(&key, term);
                tx.execute(
                    "INSERT OR REPLACE INTO memory_forgotten VALUES(?1,?2)",
                    params![tid, epoch],
                )
                .map_err(db_err)?;
                tx.execute(
                    "DELETE FROM inputia_events WHERE text IS NOT NULL AND memory_term_id(text)=?1",
                    [&tid],
                )
                .map_err(db_err)?;
                tx.execute(
                    "DELETE FROM memory_contributions WHERE term_hmac=?1",
                    [&tid],
                )
                .map_err(db_err)?;
            }
            crate::privacy_operation::PrivacyScope::ClearLearned {} => {
                tx.execute("INSERT OR REPLACE INTO memory_forgotten SELECT term_hmac,?1 FROM memory_contributions",[epoch]).map_err(db_err)?;
                tx.execute("INSERT OR REPLACE INTO memory_forgotten SELECT memory_term_id(text),?1 FROM inputia_events WHERE text IS NOT NULL",[epoch]).map_err(db_err)?;
                tx.execute_batch("DELETE FROM inputia_events;DELETE FROM memory_contributions;")
                    .map_err(db_err)?;
            }
        }
        tx.execute("DELETE FROM memory_event_sources WHERE event_row NOT IN(SELECT id FROM inputia_events)",[]).map_err(db_err)?;
        rebuild(&tx)?;
        tx.execute(
            "UPDATE memory_domain_meta SET privacy_epoch=?1 WHERE singleton=1",
            [epoch],
        )
        .map_err(db_err)?;
        tx.execute(
            "INSERT INTO memory_privacy_receipts VALUES(?1,?2,?3,?4)",
            params![request.operation_id, digest, epoch, domain],
        )
        .map_err(db_err)?;
        tx.commit().map_err(db_err)?;
        self.db()?;
        Ok(())
    }
    /// 源投影必须先经过完整同步屏障；新修订或墓碑撤销旧贡献，绝不自动重学新正文。
    pub(crate) fn reconcile_projection(&mut self, projection: &Connection) -> Result<()> {
        if self.db.is_none() {
            return Ok(());
        }
        let refs:Vec<(String,String,u64)>=self.db()?.prepare("SELECT store_id,record_id,revision FROM memory_sources WHERE deleted=0 AND store_id NOT LIKE 'commit:%'").map_err(db_err)?.query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(db_err)?.collect::<std::result::Result<_,_>>().map_err(db_err)?;
        let mut revoke = vec![];
        for (store, record, revision) in refs {
            let current:Option<u64>=projection.query_row("SELECT item.revision FROM integration_items item JOIN integration_sources source ON source.logical_name=item.logical_name WHERE source.active_store_id=?1 AND item.record_id=?2",params![store,record],|r|r.get(0)).optional().map_err(db_err)?;
            if current != Some(revision) {
                revoke.push((
                    store,
                    record,
                    current.unwrap_or(revision),
                    current.is_none(),
                ));
            }
        }
        if revoke.is_empty() {
            return Ok(());
        }
        let tx = self
            .db
            .as_mut()
            .ok_or("memory_not_configured")?
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_err)?;
        for (store, record, revision, deleted) in revoke {
            revoke_record(&tx, &store, &record, revision, deleted)?;
        }
        tx.commit().map_err(db_err)?;
        self.db()?;
        Ok(())
    }
    pub(crate) fn begin_import(
        &mut self,
        request: &MemoryImportRequest,
        history_store: Option<&str>,
        clipboard_store: Option<&str>,
    ) -> Result<MemoryImportStatus> {
        valid_id(&request.operation_id)?;
        valid_epoch(request.expected_epoch)?;
        if !(1..=2000).contains(&request.limit) {
            return Err("memory_import_limit_invalid".into());
        }
        let request_json = serde_json::to_string(request).map_err(|_| "memory_request_invalid")?;
        let digest = import_hash(&self.key, request, history_store, clipboard_store)?;
        if let Some(record) = self.import_record(&request.operation_id)? {
            if record.request != *request {
                return Err("memory_operation_conflict".into());
            }
            return Ok(record.status);
        }
        if self.operation(&request.operation_id)?.is_some() {
            return Err("memory_operation_conflict".into());
        }
        self.require_epoch(request.expected_epoch)?;
        let result = MemoryImportStatus {
            operation_id: request.operation_id.clone(),
            applied_at_epoch: request.expected_epoch,
            state: MemoryImportState::Accepted,
            history_imported: 0,
            clipboard_imported: 0,
            skipped: 0,
            failure: None,
        };
        self.db()?.execute("INSERT INTO memory_imports(operation_id,request_hmac,request,result,history_done,clipboard_done,history_store,clipboard_store) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![request.operation_id,digest,request_json,serde_json::to_string(&result).map_err(|_|"memory_receipt_invalid")?,request.selection==MemoryImportSelection::Clipboard,request.selection==MemoryImportSelection::History,history_store,clipboard_store]).map_err(db_err)?;
        Ok(result)
    }
    pub(crate) fn import_remaining(
        &self,
        id: &str,
        clipboard: bool,
        limit: usize,
    ) -> Result<usize> {
        let scanned: usize = self
            .db()?
            .query_row(
                if clipboard {
                    "SELECT clipboard_scanned FROM memory_imports WHERE operation_id=?1"
                } else {
                    "SELECT history_scanned FROM memory_imports WHERE operation_id=?1"
                },
                [id],
                |r| r.get(0),
            )
            .map_err(db_err)?;
        Ok(limit.saturating_sub(scanned))
    }
    pub(crate) fn next_import(&self) -> Result<Option<PendingMemoryImport>> {
        if self.db.is_none() {
            return Ok(None);
        }
        let row:Option<(String,bool,Option<String>)>=self.db()?.query_row("SELECT operation_id,history_done,CASE WHEN history_done=0 THEN history_cursor ELSE clipboard_cursor END FROM memory_imports WHERE history_done=0 OR clipboard_done=0 ORDER BY rowid LIMIT 1",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(db_err)?;
        row.map(|(id, clipboard, cursor)| {
            let record = self.import_record(&id)?.ok_or("memory_import_missing")?;
            Ok(PendingMemoryImport {
                request: record.request,
                clipboard,
                cursor,
                store: if clipboard {
                    record.clipboard_store
                } else {
                    record.history_store
                }
                .ok_or("memory_source_unavailable")?,
            })
        })
        .transpose()
    }

    pub(crate) fn apply_import_page(
        &mut self,
        request: &MemoryImportRequest,
        clipboard: bool,
        records: &[crate::source::SnapshotRecord],
        store: &str,
        epoch: u64,
    ) -> Result<MemoryImportStatus> {
        let ImportRecord {
            request: original,
            mut status,
            history_store,
            clipboard_store,
        } = self
            .import_record(&request.operation_id)?
            .ok_or("memory_import_missing")?;
        let bound = if clipboard {
            clipboard_store
        } else {
            history_store
        };
        if original != *request || bound.as_deref() != Some(store) {
            return Err("memory_import_source_conflict".into());
        }
        let current = self.status()?.policy_epoch;
        let remaining = self.import_remaining(&request.operation_id, clipboard, request.limit)?;
        if records.len() > 128 {
            return Err("memory_import_page_limit".into());
        }
        let key = self.key;
        let tx = self
            .db
            .as_mut()
            .ok_or("memory_not_configured")?
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_err)?;
        if current != request.expected_epoch || epoch != request.expected_epoch {
            status.state = MemoryImportState::Revoked;
            status.failure = Some("memory_epoch_revoked".into());
            tx.execute(
                "UPDATE memory_imports SET history_done=1,clipboard_done=1 WHERE operation_id=?1",
                [&request.operation_id],
            )
            .map_err(db_err)?;
        } else {
            let mut eligible = 0usize;
            let mut cursor = None;
            for record in records {
                if eligible >= remaining {
                    break;
                }
                cursor = Some(record.record_id.as_str());
                let Some(payload) = &record.payload else {
                    status.skipped += 1;
                    continue;
                };
                let Some(text) = &payload.text else {
                    status.skipped += 1;
                    continue;
                };
                if payload.content_type != crate::store::ContentType::Text
                    || display_text(text).is_empty()
                {
                    status.skipped += 1;
                    continue;
                }
                eligible += 1;
                let source = if clipboard {
                    MemoryOrigin::Clipboard
                } else {
                    MemoryOrigin::Voice
                };
                let app = payload.source_app.clone().unwrap_or_default();
                if valid_text(text).is_err()
                    || AppPolicy::default().excludes(&AppContext::new(&app))
                {
                    status.skipped += 1;
                    continue;
                }
                let e = VerifiedMemoryEvidence::source(
                    store.into(),
                    record.record_id.clone(),
                    record.revision,
                    epoch,
                    source,
                    text.clone(),
                    app,
                )?;
                let state = contribute(
                    &tx,
                    &key,
                    &e,
                    &format!("{}:{}", request.operation_id, record.record_id),
                )?;
                if state == MemoryMutationState::Applied {
                    if clipboard {
                        status.clipboard_imported += 1
                    } else {
                        status.history_imported += 1
                    }
                } else {
                    status.skipped += 1
                }
            }
            let (cursor_column, done_column, scanned_column) = if clipboard {
                ("clipboard_cursor", "clipboard_done", "clipboard_scanned")
            } else {
                ("history_cursor", "history_done", "history_scanned")
            };
            // 游标、贡献和统计同一事务；调用方每页<=128，空页证明该源扫描结束。
            tx.execute(&format!("UPDATE memory_imports SET {cursor_column}=?2,{scanned_column}={scanned_column}+?3,{done_column}=(?4 OR {scanned_column}+?3>=?5) WHERE operation_id=?1"),params![request.operation_id,cursor,eligible,records.len()<128,request.limit]).map_err(db_err)?;
            let done:bool=tx.query_row("SELECT history_done AND clipboard_done FROM memory_imports WHERE operation_id=?1",[&request.operation_id],|r|r.get(0)).map_err(db_err)?;
            status.state = if done {
                MemoryImportState::Completed
            } else {
                MemoryImportState::Processing
            };
        }
        let response = serde_json::to_string(&status).map_err(|_| "memory_receipt_invalid")?;
        tx.execute(
            "UPDATE memory_imports SET result=?2 WHERE operation_id=?1",
            params![request.operation_id, response],
        )
        .map_err(db_err)?;
        let digest: Vec<u8> = tx
            .query_row(
                "SELECT request_hmac FROM memory_imports WHERE operation_id=?1",
                [&request.operation_id],
                |r| r.get(0),
            )
            .map_err(db_err)?;
        tx.execute("INSERT INTO memory_operations VALUES(?1,?2,'import',?3,?4) ON CONFLICT(operation_id) DO UPDATE SET result=excluded.result",params![request.operation_id,digest,request.expected_epoch,serde_json::to_string(&MemoryOperationStatus::Import(status.clone())).map_err(|_|"memory_receipt_invalid")?]).map_err(db_err)?;
        tx.commit().map_err(db_err)?;
        self.db()?;
        Ok(status)
    }
    pub(crate) fn fail_import(&mut self, id: &str, reason: &str) -> Result<()> {
        let raw: String = self
            .db()?
            .query_row(
                "SELECT result FROM memory_imports WHERE operation_id=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(db_err)?;
        let mut status: MemoryImportStatus =
            serde_json::from_str(&raw).map_err(|_| "memory_receipt_invalid")?;
        status.state = MemoryImportState::PartialFailure;
        status.failure = Some(reason.into());
        self.db()?.execute("UPDATE memory_imports SET result=?2,history_done=1,clipboard_done=1 WHERE operation_id=?1",params![id,serde_json::to_string(&status).map_err(|_|"memory_receipt_invalid")?]).map_err(db_err)?;
        Ok(())
    }
}

impl LegacyMemory {
    pub(crate) fn verify_source_revocation(
        &self,
        request: &crate::deletion_lifecycle::DeleteRequest,
    ) -> Result<bool> {
        let hash = hmac(
            &self.key,
            b"source-delete",
            &serde_json::to_vec(request).map_err(|_| "memory_request_invalid")?,
        );
        let result:Option<(Vec<u8>,String)>=self.db()?.query_row("SELECT request_hmac,domain_uuid FROM memory_source_revocations WHERE operation_id=?1",[&request.operation_id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(db_err)?;
        match result {
            None => Ok(false),
            Some((prior, domain))
                if prior == hash && self.domain.as_deref() == Some(domain.as_str()) =>
            {
                Ok(true)
            }
            Some(_) => Err("memory_source_receipt_conflict".into()),
        }
    }
    pub(crate) fn revoke_source(
        &mut self,
        request: &crate::deletion_lifecycle::DeleteRequest,
    ) -> Result<()> {
        request.validate().map_err(|_| "memory_request_invalid")?;
        if self.verify_source_revocation(request)? {
            return Ok(());
        }
        let unknown: bool = self
            .db()?
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM memory_contributions WHERE source='migration')",
                [],
                |r| r.get(0),
            )
            .map_err(db_err)?;
        if unknown {
            return Err("memory_migration_provenance_unresolved".into());
        }
        let hash = hmac(
            &self.key,
            b"source-delete",
            &serde_json::to_vec(request).map_err(|_| "memory_request_invalid")?,
        );
        let domain = self.domain.clone().ok_or("memory_domain_missing")?;
        let tx = self
            .db
            .as_mut()
            .ok_or("memory_handoff_required")?
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_err)?;
        let revision: Option<u64> = tx
            .query_row(
                "SELECT revision FROM memory_sources WHERE store_id=?1 AND record_id=?2",
                params![request.store_id, request.record_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(db_err)?;
        if revision.is_some_and(|v| v > request.expected_revision + 1) {
            return Err("memory_source_revision".into());
        }
        revoke_record(
            &tx,
            &request.store_id,
            &request.record_id,
            request.expected_revision + 1,
            true,
        )?;
        tx.execute(
            "INSERT INTO memory_source_revocations VALUES(?1,?2,?3)",
            params![request.operation_id, hash, domain],
        )
        .map_err(db_err)?;
        tx.commit().map_err(db_err)?;
        self.db()?;
        Ok(())
    }
}

fn import_hash(
    key: &[u8],
    request: &MemoryImportRequest,
    history: Option<&str>,
    clipboard: Option<&str>,
) -> Result<Vec<u8>> {
    Ok(hmac(
        key,
        b"import",
        &serde_json::to_vec(&(request, history, clipboard))
            .map_err(|_| "memory_request_invalid")?,
    ))
}
impl LegacyMemory {
    fn import_record(&self, id: &str) -> Result<Option<ImportRecord>> {
        let row:Option<ImportRow>=self.db()?.query_row("SELECT request,result,history_store,clipboard_store,request_hmac FROM memory_imports WHERE operation_id=?1",[id],|r|Ok(ImportRow{request:r.get(0)?,status:r.get(1)?,history:r.get(2)?,clipboard:r.get(3)?,digest:r.get(4)?})).optional().map_err(db_err)?;
        row.map(
            |ImportRow {
                 request,
                 status,
                 history,
                 clipboard,
                 digest,
             }| {
                let request: MemoryImportRequest =
                    serde_json::from_str(&request).map_err(|_| "memory_request_invalid")?;
                let status: MemoryImportStatus =
                    serde_json::from_str(&status).map_err(|_| "memory_receipt_invalid")?;
                if request.operation_id != id
                    || status.operation_id != id
                    || status.applied_at_epoch != request.expected_epoch
                    || digest
                        != import_hash(
                            &self.key,
                            &request,
                            history.as_deref(),
                            clipboard.as_deref(),
                        )?
                {
                    return Err("memory_import_conflict".into());
                }
                Ok(ImportRecord {
                    request,
                    status,
                    history_store: history,
                    clipboard_store: clipboard,
                })
            },
        )
        .transpose()
    }
}

fn span_identity(key: &[u8], identity: &crate::memory_commit::CommitIdentity) -> Result<Vec<u8>> {
    Ok(hmac(
        key,
        b"word-span-identity",
        &serde_json::to_vec(&(
            &identity.client_instance,
            &identity.server_instance,
            identity.permission_epoch,
            identity.policy_epoch,
            &identity.target,
        ))
        .map_err(|_| "memory_span_identity_invalid")?,
    ))
}
fn clear_span(db: &Connection, store: &str) -> Result<()> {
    db.execute("DELETE FROM inputia_events WHERE id IN(SELECT event_row FROM memory_event_sources WHERE store_id=?1)",[store]).map_err(db_err)?;
    db.execute(
        "DELETE FROM memory_event_sources WHERE store_id=?1",
        [store],
    )
    .map_err(db_err)?;
    db.execute(
        "DELETE FROM memory_contributions WHERE store_id=?1",
        [store],
    )
    .map_err(db_err)?;
    db.execute("DELETE FROM memory_sources WHERE store_id=?1", [store])
        .map_err(db_err)?;
    rebuild(db)
}
impl LegacyMemory {
    pub(crate) fn apply_word_span(
        &mut self,
        verified: &crate::memory_word_span::VerifiedWordSpan,
    ) -> Result<MemoryMutationReceipt> {
        let identity = verified.identity();
        self.require_epoch(identity.policy_epoch)?;
        let app = identity
            .target
            .source_app
            .as_deref()
            .ok_or("memory_span_identity_invalid")?;
        if AppPolicy::default().excludes(&AppContext::new(app)) {
            return Err("memory_span_sensitive".into());
        }
        let identity_digest = span_identity(&self.key, identity)?;
        let words: Vec<_> = verified
            .words()
            .iter()
            .map(|word| (word.offset, word.text.as_str()))
            .collect();
        let request_digest = hmac(
            &self.key,
            b"word-span-revision",
            &serde_json::to_vec(&(
                verified.span_id(),
                verified.operation_id(),
                verified.revision(),
                verified.sealed(),
                &identity_digest,
                words,
            ))
            .map_err(|_| "memory_span_request_invalid")?,
        );
        let domain = self.domain.clone().ok_or("memory_domain_missing")?;
        let key = self.key;
        let tx = self
            .db
            .as_mut()
            .ok_or("memory_handoff_required")?
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_err)?;
        if let Some(receipt) = span_replay(&tx, verified.operation_id(), &request_digest)? {
            return Ok(receipt);
        }
        let prior: Option<(Vec<u8>, u64, bool, bool)> = tx
            .query_row(
                "SELECT identity_hmac,revision,revoked,sealed FROM memory_word_spans WHERE span_id=?1",
                [verified.span_id()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?,r.get(3)?)),
            )
            .optional()
            .map_err(db_err)?;
        let previous = if let Some((digest, revision, revoked, sealed)) = prior {
            if digest != identity_digest || revoked || sealed {
                return Err("memory_span_revoked".into());
            }
            revision
        } else {
            0
        };
        if verified.revision() != previous + 1 {
            return Err("memory_span_revision_conflict".into());
        }
        let store = format!("commit:span:{}", verified.span_id());
        clear_span(&tx, &store)?;
        let mut state = MemoryMutationState::Applied;
        for word in verified.words() {
            let evidence = VerifiedMemoryEvidence::source(
                store.clone(),
                word.offset.to_string(),
                verified.revision(),
                identity.policy_epoch,
                MemoryOrigin::Typed,
                word.text.clone(),
                app.into(),
            )?;
            if contribute(&tx, &key, &evidence, verified.operation_id())?
                == MemoryMutationState::Revoked
            {
                state = MemoryMutationState::Revoked;
            }
        }
        tx.execute("INSERT INTO memory_word_spans VALUES(?1,?2,?3,?4,0,?5) ON CONFLICT(span_id) DO UPDATE SET revision=excluded.revision,sealed=excluded.sealed",params![verified.span_id(),identity_digest,verified.revision(),identity.policy_epoch,verified.sealed()]).map_err(db_err)?;
        let receipt = span_finish(
            &tx,
            verified.operation_id(),
            &request_digest,
            identity.policy_epoch,
            domain,
            state,
        )?;
        tx.commit().map_err(db_err)?;
        self.db()?;
        Ok(receipt)
    }
    pub(crate) fn revoke_word_span(
        &mut self,
        revoked: &crate::memory_word_span::RevokedWordSpan,
    ) -> Result<MemoryMutationReceipt> {
        self.db()?;
        let identity_digest = span_identity(&self.key, revoked.identity())?;
        let digest = hmac(
            &self.key,
            b"word-span-revoke",
            &serde_json::to_vec(&(revoked.span_id(), revoked.revision(), &identity_digest))
                .map_err(|_| "memory_span_request_invalid")?,
        );
        let domain = self.domain.clone().ok_or("memory_domain_missing")?;
        let tx = self
            .db
            .as_mut()
            .ok_or("memory_handoff_required")?
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(db_err)?;
        if let Some(receipt) = span_replay(&tx, revoked.operation_id(), &digest)? {
            return Ok(receipt);
        }
        let prior: Option<(Vec<u8>, u64,bool,bool)> = tx
            .query_row(
                "SELECT identity_hmac,revision,sealed,revoked FROM memory_word_spans WHERE span_id=?1",
                [revoked.span_id()],
                |r| Ok((r.get(0)?, r.get(1)?,r.get(2)?,r.get(3)?)),
            )
            .optional()
            .map_err(db_err)?;
        if let Some((hash, revision, sealed, already_revoked)) = prior {
            if hash != identity_digest {
                return Err("memory_span_revision_conflict".into());
            }
            if sealed || already_revoked {
                let receipt = span_finish(
                    &tx,
                    revoked.operation_id(),
                    &digest,
                    revoked.identity().policy_epoch,
                    domain,
                    if sealed {
                        MemoryMutationState::AlreadyContributed
                    } else {
                        MemoryMutationState::Revoked
                    },
                )?;
                tx.commit().map_err(db_err)?;
                self.db()?;
                return Ok(receipt);
            }
            if revision >= revoked.revision() {
                return Err("memory_span_revision_conflict".into());
            }
        }
        clear_span(&tx, &format!("commit:span:{}", revoked.span_id()))?;
        tx.execute("INSERT INTO memory_word_spans VALUES(?1,?2,?3,?4,1,0) ON CONFLICT(span_id) DO UPDATE SET revision=excluded.revision,revoked=1",params![revoked.span_id(),identity_digest,revoked.revision(),revoked.identity().policy_epoch]).map_err(db_err)?;
        let receipt = span_finish(
            &tx,
            revoked.operation_id(),
            &digest,
            revoked.identity().policy_epoch,
            domain,
            MemoryMutationState::Revoked,
        )?;
        tx.commit().map_err(db_err)?;
        self.db()?;
        Ok(receipt)
    }
}
fn span_replay(db: &Connection, id: &str, digest: &[u8]) -> Result<Option<MemoryMutationReceipt>> {
    let prior: Option<(Vec<u8>, String)> = db
        .query_row(
            "SELECT request_hmac,result FROM memory_operations WHERE operation_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(db_err)?;
    prior
        .map(|(hash, raw)| {
            if hash != digest {
                return Err("memory_operation_conflict".into());
            }
            let MemoryOperationStatus::Learn(mut receipt) =
                serde_json::from_str(&raw).map_err(|_| "memory_receipt_invalid")?
            else {
                return Err("memory_operation_conflict".into());
            };
            receipt.replayed = true;
            Ok(receipt)
        })
        .transpose()
}
fn span_finish(
    db: &Connection,
    id: &str,
    digest: &[u8],
    epoch: u64,
    domain: String,
    state: MemoryMutationState,
) -> Result<MemoryMutationReceipt> {
    let generation = db
        .query_row(
            "SELECT generation FROM memory_domain_meta WHERE singleton=1",
            [],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    let receipt = MemoryMutationReceipt {
        operation_id: id.into(),
        applied_at_epoch: epoch,
        domain_uuid: domain,
        generation,
        state,
        replayed: false,
    };
    db.execute(
        "INSERT INTO memory_operations VALUES(?1,?2,'word_span',?3,?4)",
        params![
            id,
            digest,
            epoch,
            serde_json::to_string(&MemoryOperationStatus::Learn(receipt.clone()))
                .map_err(|_| "memory_receipt_invalid")?
        ],
    )
    .map_err(db_err)?;
    Ok(receipt)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn memory(temp: &tempfile::TempDir) -> LegacyMemory {
        LegacyMemory::open(
            LegacyMemoryContext::fixture(temp.path().join("memory.db"), "fixture".into()),
            [9; 32],
            1,
            None,
        )
        .unwrap()
    }
    fn evidence(record: &str, revision: u64, epoch: u64, text: &str) -> VerifiedMemoryEvidence {
        VerifiedMemoryEvidence::source(
            "fixture-source".into(),
            record.into(),
            revision,
            epoch,
            MemoryOrigin::Voice,
            text.into(),
            "com.example.editor".into(),
        )
        .unwrap()
    }
    fn count(m: &LegacyMemory, text: &str) -> u64 {
        m.db()
            .unwrap()
            .query_row(
                "SELECT voice_count FROM inputia_terms WHERE text=?1",
                [text],
                |r| r.get(0),
            )
            .optional()
            .unwrap()
            .unwrap_or(0)
    }
    #[test]
    fn missing_handoff_never_opens_or_creates_database() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("never-open.db");
        let m = LegacyMemory::open(
            LegacyMemoryContext::handoff_required(path.clone(), "fixture".into()),
            [9; 32],
            1,
            None,
        )
        .unwrap();
        assert_eq!(
            m.status().unwrap().state,
            MemoryDomainState::HandoffRequired
        );
        assert!(!path.exists());
        assert!(m
            .query(&MemoryQuery::VoiceHotwords { limit: 5 }, 1)
            .is_err());
    }
    #[test]
    fn contributions_are_source_revision_and_event_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        let mut m = memory(&temp);
        let e = evidence("1", 1, 1, "privateword");
        let i = e.intent("op1".into());
        assert_eq!(
            m.apply_intent(&i, &e).unwrap().state,
            MemoryMutationState::Applied
        );
        assert!(m.apply_intent(&i, &e).unwrap().replayed);
        assert_eq!(
            m.apply_intent(&e.intent("op2".into()), &e).unwrap().state,
            MemoryMutationState::AlreadyContributed
        );
        assert_eq!(count(&m, "privateword"), 1);
        let e2 = evidence("2", 1, 1, "different");
        let mut conflict = e2.intent("op3".into());
        conflict.event_id = "op1".into();
        assert!(m.apply_intent(&conflict, &e2).is_err());
        let revised = evidence("1", 2, 1, "revised");
        m.apply_intent(&revised.intent("op4".into()), &revised)
            .unwrap();
        assert_eq!(count(&m, "privateword"), 0);
        assert_eq!(count(&m, "revised"), 1);
        let stale = evidence("1", 1, 1, "privateword");
        assert_eq!(
            m.apply_intent(&stale.intent("op5".into()), &stale)
                .unwrap()
                .state,
            MemoryMutationState::Revoked
        );
        let text: usize = m
            .db()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM inputia_events WHERE text='privateword'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(text, 0);
    }
    #[test]
    fn privacy_receipt_is_real_and_new_operations_cannot_restore_forgotten_terms() {
        let temp = tempfile::tempdir().unwrap();
        let mut m = memory(&temp);
        let e = evidence("1", 1, 1, "Private Word");
        m.apply_intent(&e.intent("learn".into()), &e).unwrap();
        let request = crate::privacy_operation::PrivacyRequest {
            operation_id: "forget".into(),
            expected_epoch: 1,
            scope: crate::privacy_operation::PrivacyScope::ForgetTerm {
                term: "private   word".into(),
            },
        };
        let digest = request.digest(&[9; 32]).unwrap();
        m.apply_privacy(&request, &digest, 2).unwrap();
        m.apply_privacy(&request, &digest, 2).unwrap();
        assert!(m
            .privacy_receipt("forget", &digest, 2, m.domain.as_deref().unwrap())
            .unwrap());
        assert_eq!(count(&m, "Private Word"), 0);
        let new = evidence("2", 1, 2, "PRIVATE WORD");
        assert_eq!(
            m.apply_intent(&new.intent("new-id".into()), &new)
                .unwrap()
                .state,
            MemoryMutationState::Revoked
        );
        assert_eq!(
            m.db()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM inputia_events", [], |r| r
                    .get::<_, u64>(0))
                .unwrap(),
            0
        );
        let metadata: String = m
            .db()
            .unwrap()
            .query_row(
                "SELECT group_concat(result) FROM memory_operations",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!metadata.to_lowercase().contains("private word"));
        assert!(m
            .privacy_receipt("forget", &[0; 32], 2, m.domain.as_deref().unwrap())
            .is_err());
    }
    #[test]
    fn import_pages_are_bounded_and_repeat_revision_only_contributes_once() {
        let temp = tempfile::tempdir().unwrap();
        let mut m = memory(&temp);
        let request = MemoryImportRequest {
            operation_id: "import-a".into(),
            expected_epoch: 1,
            selection: MemoryImportSelection::History,
            limit: 2000,
        };
        m.begin_import(&request, Some("fixture-source"), None)
            .unwrap();
        assert!(matches!(
            m.operation("import-a").unwrap(),
            Some(MemoryOperationStatus::Import(MemoryImportStatus {
                state: MemoryImportState::Accepted,
                ..
            }))
        ));
        let records: Vec<_> = (0..128)
            .map(|n| crate::source::SnapshotRecord {
                record_id: n.to_string(),
                revision: 1,
                payload: Some(crate::store::ItemSnapshot {
                    source_kind: crate::store::SourceKind::Voice,
                    content_type: crate::store::ContentType::Text,
                    text: Some(format!("term{n}")),
                    title: None,
                    starred: false,
                    pinned: false,
                    created_at_ms: 0,
                    asset_ref: None,
                    source_app: Some("com.example.editor".into()),
                    source_trust: crate::store::SourceTrust::Verified,
                }),
            })
            .collect();
        let first = m
            .apply_import_page(&request, false, &records, "fixture-source", 1)
            .unwrap();
        assert_eq!(first.history_imported, 128);
        assert_eq!(first.state, MemoryImportState::Processing);
        assert_eq!(m.import_remaining("import-a", false, 2000).unwrap(), 1872);
        m.apply_import_page(&request, false, &[], "fixture-source", 1)
            .unwrap();
        let second = MemoryImportRequest {
            operation_id: "import-b".into(),
            ..request
        };
        m.begin_import(&second, Some("fixture-source"), None)
            .unwrap();
        let status = m
            .apply_import_page(&second, false, &records, "fixture-source", 1)
            .unwrap();
        assert_eq!(status.history_imported, 0);
        assert_eq!(status.skipped, 128);
        assert_eq!(count(&m, "term1"), 1);
    }
    #[test]
    fn legacy_case_variants_preserve_counts_but_share_forgetting_barrier() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("memory.db");
        let mut old = inputia_core::SqliteMemory::open(&path, AppPolicy::default()).unwrap();
        for text in ["Private Word", "private word"] {
            old.learn(
                inputia_core::MemorySource::Voice,
                text,
                &AppContext::new("com.example.editor"),
            )
            .unwrap();
        }
        drop(old);
        let mut m = memory(&temp);
        assert_eq!(count(&m, "Private Word"), 1);
        assert_eq!(count(&m, "private word"), 1);
        let r = crate::privacy_operation::PrivacyRequest {
            operation_id: "forget-case".into(),
            expected_epoch: 1,
            scope: crate::privacy_operation::PrivacyScope::ForgetTerm {
                term: "private word".into(),
            },
        };
        m.apply_privacy(&r, &r.digest(&[9; 32]).unwrap(), 2)
            .unwrap();
        assert_eq!(count(&m, "Private Word"), 0);
        assert_eq!(count(&m, "private word"), 0);
    }
    #[test]
    fn source_binding_tamper_cannot_redirect_an_accepted_import() {
        let temp = tempfile::tempdir().unwrap();
        let mut m = memory(&temp);
        let r = MemoryImportRequest {
            operation_id: "bound-import".into(),
            expected_epoch: 1,
            selection: MemoryImportSelection::History,
            limit: 2000,
        };
        m.begin_import(&r, Some("source-one"), None).unwrap();
        m.db()
            .unwrap()
            .execute("UPDATE memory_imports SET history_store='source-two'", [])
            .unwrap();
        assert!(m.next_import().is_err());
        assert!(m.begin_import(&r, Some("source-two"), None).is_err());
    }

    #[test]
    fn initial_empty_and_migrated_snapshots_pass_the_real_wire_contract() {
        use crate::legacy_memory_wire::{MemoryRequest, MemorySnapshotReply};
        for migrated in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            if migrated {
                let mut old = inputia_core::SqliteMemory::open(
                    temp.path().join("memory.db"),
                    AppPolicy::default(),
                )
                .unwrap();
                old.learn(
                    inputia_core::MemorySource::Voice,
                    "existing",
                    &AppContext::new("com.example.editor"),
                )
                .unwrap();
            }
            let memory = memory(&temp);
            let query = MemoryQuery::VoiceHotwords { limit: 5 };
            let snapshot = memory.query(&query, 1).unwrap();
            assert_eq!(snapshot.generation, 1);
            assert_eq!(snapshot.terms.len(), usize::from(migrated));
            let request: MemoryRequest = serde_json::from_value(serde_json::json!({
                "request_id":"request","client_instance":"host","server_instance":"server","policy_epoch":1,
                "memory_domain":{"kind":"query","query_id":"query","query_generation":1,"composing":"",
                    "query":query,"target":{"target_id":"field","host_instance":"host","controller_id":"controller",
                    "activation_generation":1,"field_id":"field","selection_generation":1,"composition_generation":1,
                    "source_app":"com.example.editor"}}
            })).unwrap();
            let reply = MemorySnapshotReply {
                format_version: 1,
                request_id: "request".into(),
                server_instance: "server".into(),
                profile_id: "fixture".into(),
                policy_epoch: 1,
                domain_uuid: snapshot.domain_uuid,
                generation: snapshot.generation,
                query_id: "query".into(),
                query_generation: 1,
                query_digest: request.query_digest().unwrap(),
                query,
                terms: snapshot.terms,
                lease_id: "lease".into(),
                max_age_ms: snapshot.max_age_ms,
            };
            reply.validate_for(&request, "fixture").unwrap();
        }
    }

    #[test]
    fn migration_keeps_u32_counts_and_new_learning_uses_the_old_tick_high_water() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("memory.db");
        drop(inputia_core::SqliteMemory::open(&path, AppPolicy::default()).unwrap());
        let old = Connection::open(path).unwrap();
        old.execute(
            "INSERT INTO inputia_terms VALUES('old',0,1,1,1000),('large',0,4294967294,0,900)",
            [],
        )
        .unwrap();
        old.execute("INSERT INTO inputia_events(source,text,app_bundle_id,privacy_decision,created_tick) VALUES('clipboard','old','com.example.editor','learn',1500)", []).unwrap();
        drop(old);
        let mut m = memory(&temp);
        let mut e = evidence("new", 1, 1, "new");
        e.source = MemoryOrigin::Clipboard;
        m.apply_intent(&e.intent("new-op".into()), &e).unwrap();
        let tick: u64 = m
            .db()
            .unwrap()
            .query_row(
                "SELECT last_used_tick FROM inputia_terms WHERE text='new'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tick, 1501);
        assert_eq!(
            m.query(&MemoryQuery::Clipboard { limit: 2 }, 1)
                .unwrap()
                .terms[0]
                .text,
            "new"
        );
        for n in 0..3 {
            let e = evidence(&format!("large-{n}"), 1, 1, "large");
            m.apply_intent(&e.intent(format!("large-op-{n}")), &e)
                .unwrap();
        }
        assert_eq!(count(&m, "large"), u32::MAX as u64);
        rebuild(m.db().unwrap()).unwrap();
        assert_eq!(count(&m, "large"), u32::MAX as u64);
        drop(m);
        let mut m = memory(&temp);
        let e = evidence("latest", 1, 1, "latest");
        m.apply_intent(&e.intent("latest-op".into()), &e).unwrap();
        let tick: u64 = m
            .db()
            .unwrap()
            .query_row(
                "SELECT last_used_tick FROM inputia_terms WHERE text='latest'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tick, 1505);
    }

    #[test]
    fn in_place_rollback_cannot_be_masked_by_advancing_the_policy_epoch() {
        for epoch_rollback in [true, false] {
            let temp = tempfile::tempdir().unwrap();
            let mut m = memory(&temp);
            let e = evidence("old", 1, 1, "private");
            m.apply_intent(&e.intent("learn".into()), &e).unwrap();
            let before = m.status().unwrap();
            let request = crate::privacy_operation::PrivacyRequest {
                operation_id: "forget".into(),
                expected_epoch: 1,
                scope: crate::privacy_operation::PrivacyScope::ForgetTerm {
                    term: "private".into(),
                },
            };
            m.apply_privacy(&request, &request.digest(&[9; 32]).unwrap(), 2)
                .unwrap();
            // 同 inode、同 UUID：模拟旧库备份在运行中覆盖正文和版本。
            let mut other = Connection::open(temp.path().join("memory.db")).unwrap();
            let tx = other.transaction().unwrap();
            tx.execute("DELETE FROM memory_privacy_receipts", [])
                .unwrap();
            tx.execute("INSERT INTO inputia_terms VALUES('private',0,1,0,1)", [])
                .unwrap();
            if epoch_rollback {
                tx.execute("UPDATE memory_domain_meta SET privacy_epoch=1", [])
                    .unwrap();
            } else {
                tx.execute(
                    "UPDATE memory_domain_meta SET generation=?1",
                    [before.generation],
                )
                .unwrap();
            }
            tx.commit().unwrap();
            assert_eq!(m.advance_epoch(2).unwrap_err(), "memory_domain_rolled_back");
            assert!(m
                .query(&MemoryQuery::VoiceHotwords { limit: 5 }, 2)
                .is_err());
        }
    }
}
