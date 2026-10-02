//! 受管附件的导入、引用和回收。只处理已审计登记的精确文件，不扫描目录猜测孤儿。
//! 所有数据库调用由 HistoryService 的唯一写入线程执行；文件副作用各有持久恢复阶段。

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Cursor, Read, Write},
    path::{Component, Path, PathBuf},
    sync::{Arc, Condvar, Mutex},
};

pub type Result<T> = std::result::Result<T, AttachmentError>;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentKind {
    Recording,
    Image,
    External,
}
impl AttachmentKind {
    fn directory(self) -> &'static str {
        match self {
            Self::Recording => "recordings",
            Self::Image => "clipboard_images",
            Self::External => "",
        }
    }
    fn extension(self) -> &'static str {
        match self {
            Self::Recording => "wav",
            Self::Image => "png",
            Self::External => "external",
        }
    }
    fn parse(value: &str) -> Result<Self> {
        match value {
            "wav" => Ok(Self::Recording),
            "png" => Ok(Self::Image),
            "external" => Ok(Self::External),
            _ => Err(AttachmentError::Invalid("attachment kind")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PauseReason {
    Capacity,
    InsufficientSpace,
    SpaceUnavailable,
    FileTooLarge,
    InvalidFormat,
    UnsafePath,
    MissingFile,
    IdentityChanged,
    BusyPin,
    SourceAuditPending,
    LegacyManifestUnknown,
    Maintenance,
    IoFailure,
}

#[derive(Debug)]
pub enum AttachmentError {
    Sqlite(rusqlite::Error),
    Json(serde_json::Error),
    Paused(PauseReason),
    Invalid(&'static str),
}
impl std::fmt::Display for AttachmentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sqlite(_) => f.write_str("attachment database operation failed"),
            Self::Json(_) => f.write_str("attachment metadata invalid"),
            Self::Paused(reason) => write!(f, "attachment paused: {reason:?}"),
            Self::Invalid(reason) => write!(f, "invalid attachment operation: {reason}"),
        }
    }
}
impl std::error::Error for AttachmentError {}
impl From<rusqlite::Error> for AttachmentError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}
impl From<serde_json::Error> for AttachmentError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}
impl AttachmentError {
    pub fn reason(&self) -> PauseReason {
        match self {
            Self::Paused(reason) => *reason,
            _ => PauseReason::IoFailure,
        }
    }
}
fn io_error(error: std::io::Error) -> AttachmentError {
    if error.kind() == std::io::ErrorKind::NotFound {
        AttachmentError::Paused(PauseReason::MissingFile)
    } else if error.raw_os_error() == Some(28) {
        AttachmentError::Paused(PauseReason::InsufficientSpace)
    } else {
        AttachmentError::Paused(PauseReason::IoFailure)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttachmentBudget {
    pub capacity_bytes: u64,
    pub max_file_bytes: u64,
    pub minimum_free_bytes: u64,
}
impl Default for AttachmentBudget {
    fn default() -> Self {
        Self {
            capacity_bytes: 4 * 1024 * 1024 * 1024,
            max_file_bytes: 256 * 1024 * 1024,
            minimum_free_bytes: 512 * 1024 * 1024,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AttachmentImport {
    pub attachment_id: String,
    pub file_name: String,
    pub operation_id: String,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PinPurpose {
    Active,
    Export,
    Update,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AttachmentLease {
    pub lease_id: String,
    pub instance_id: String,
    pub attachment_id: Option<String>,
    pub purpose: PinPurpose,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AttachmentReference {
    pub store_id: String,
    pub record_id: String,
    pub revision: u64,
    pub kind: AttachmentKind,
    pub path: Option<String>,
    pub retained: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AttachmentHealth {
    pub used_bytes: u64,
    pub capacity_bytes: u64,
    pub minimum_free_bytes: u64,
    pub paused: Option<PauseReason>,
    pub pending_gc: u64,
    pub blocked_imports: u64,
    pub blocked_deletions: u64,
}

#[derive(Default)]
pub struct AttachmentMutationGate {
    state: Mutex<(usize, bool)>,
    changed: Condvar,
}
pub struct AttachmentMutationGuard {
    gate: Arc<AttachmentMutationGate>,
}
pub struct AttachmentGcGuard {
    gate: Arc<AttachmentMutationGate>,
}
impl AttachmentMutationGate {
    pub fn enter(self: &Arc<Self>) -> AttachmentMutationGuard {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        while state.1 {
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
        }
        state.0 += 1;
        AttachmentMutationGuard { gate: self.clone() }
    }
    /// worker 绝不等待持源 guard 的 manager，避免 manager 同步回调 worker 时互锁。
    pub fn try_gc(self: &Arc<Self>) -> Option<AttachmentGcGuard> {
        let mut state = self.state.try_lock().ok()?;
        if state.0 != 0 || state.1 {
            return None;
        }
        state.1 = true;
        Some(AttachmentGcGuard { gate: self.clone() })
    }
}
impl Drop for AttachmentMutationGuard {
    fn drop(&mut self) {
        let mut state = self.gate.state.lock().unwrap_or_else(|e| e.into_inner());
        state.0 -= 1;
        self.gate.changed.notify_all();
    }
}
impl Drop for AttachmentGcGuard {
    fn drop(&mut self) {
        let mut state = self.gate.state.lock().unwrap_or_else(|e| e.into_inner());
        state.1 = false;
        self.gate.changed.notify_all();
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AttachmentImportStatus {
    pub operation_id: String,
    pub attachment_id: String,
    pub state: String,
    pub reason: Option<PauseReason>,
    pub released: bool,
}
pub(crate) struct AttachmentRead {
    kind: AttachmentKind,
    name: String,
    digest: String,
    size: u64,
    device: u64,
    inode: u64,
}
#[derive(Clone)]
pub(crate) struct ImportWork {
    kind: AttachmentKind,
    id: String,
    digest: String,
    name: String,
    temp: String,
    operation_id: String,
    size: u64,
    limit: u64,
    state: String,
}
pub(crate) struct PublishedImport {
    work: ImportWork,
    device: u64,
    inode: u64,
}

#[derive(Clone)]
pub struct AttachmentStore {
    root: PathBuf,
    instance: String,
    gc_cursor: Arc<Mutex<Option<String>>>,
}
pub fn new_operation_id() -> Result<String> {
    token()
}
fn token() -> Result<String> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes)
        .map_err(|_| AttachmentError::Invalid("random identity unavailable"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn identifier(value: &str) -> Result<()> {
    inputia_core::integration::events::Identifier::parse(value)
        .map(|_| ())
        .map_err(|_| AttachmentError::Invalid("identifier"))
}
fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && Path::new(name).components().count() == 1
        && matches!(
            Path::new(name).components().next(),
            Some(Component::Normal(_))
        )
}

impl AttachmentStore {
    pub fn new(root: &Path) -> Result<Self> {
        let metadata = fs::symlink_metadata(root).map_err(io_error)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(AttachmentError::Paused(PauseReason::UnsafePath));
        }
        let root = root.canonicalize().map_err(io_error)?;
        secure_directory(&root)?;
        for directory in ["recordings", "clipboard_images"] {
            let path = root.join(directory);
            match fs::create_dir(&path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(io_error(error)),
            }
            secure_directory(&path)?;
        }
        Ok(Self {
            root,
            instance: token()?,
            gc_cursor: Arc::new(Mutex::new(None)),
        })
    }
    pub fn instance_id(&self) -> &str {
        &self.instance
    }
    pub fn initialize(conn: &Connection) -> Result<()> {
        conn.execute_batch("CREATE TABLE IF NOT EXISTS attachment_files(id TEXT PRIMARY KEY,kind TEXT NOT NULL,name TEXT NOT NULL,digest TEXT NOT NULL,size INTEGER NOT NULL,device INTEGER NOT NULL,inode INTEGER NOT NULL,state TEXT NOT NULL,quarantine TEXT,reason TEXT);
            CREATE UNIQUE INDEX IF NOT EXISTS attachment_live_name ON attachment_files(kind,name) WHERE state!='removed';
            CREATE TABLE IF NOT EXISTS attachment_imports(operation_id TEXT PRIMARY KEY,attachment_id TEXT NOT NULL,digest TEXT NOT NULL,kind TEXT NOT NULL,temp_name TEXT NOT NULL,instance TEXT NOT NULL,state TEXT NOT NULL,size INTEGER NOT NULL,reason TEXT,released INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE IF NOT EXISTS attachment_refs(store_id TEXT NOT NULL,record_id TEXT NOT NULL,revision INTEGER NOT NULL,role TEXT NOT NULL,attachment_id TEXT NOT NULL REFERENCES attachment_files(id),PRIMARY KEY(store_id,record_id,revision,role,attachment_id));
            CREATE TABLE IF NOT EXISTS attachment_acquire_cancellations(instance TEXT NOT NULL,operation_id TEXT NOT NULL,PRIMARY KEY(instance,operation_id));
            CREATE TABLE IF NOT EXISTS attachment_pins(lease_id TEXT PRIMARY KEY,instance TEXT NOT NULL,attachment_id TEXT,purpose TEXT NOT NULL,active INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS attachment_owner_status(store_id TEXT NOT NULL,record_id TEXT NOT NULL,revision INTEGER NOT NULL,reason TEXT,PRIMARY KEY(store_id,record_id));
            CREATE TABLE IF NOT EXISTS attachment_deletions(operation_id TEXT PRIMARY KEY,request_digest TEXT NOT NULL,ids TEXT NOT NULL,state TEXT NOT NULL,reason TEXT,manifest_json TEXT NOT NULL,manifest_digest TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS attachment_config(singleton INTEGER PRIMARY KEY CHECK(singleton=1),budget TEXT NOT NULL,paused TEXT);")?;
        conn.execute(
            "INSERT OR IGNORE INTO attachment_config(singleton,budget) VALUES(1,?1)",
            [serde_json::to_string(&AttachmentBudget::default())?],
        )?;
        Ok(())
    }
    pub fn configure(&self, conn: &Connection, budget: &AttachmentBudget) -> Result<()> {
        if budget.max_file_bytes == 0
            || budget.max_file_bytes > budget.capacity_bytes
            || budget.minimum_free_bytes > budget.capacity_bytes
            || budget.capacity_bytes > i64::MAX as u64
        {
            return Err(AttachmentError::Invalid("storage budget"));
        }
        conn.execute(
            "UPDATE attachment_config SET budget=?1 WHERE singleton=1",
            [serde_json::to_string(budget)?],
        )?;
        Ok(())
    }
    pub fn pause(&self, conn: &Connection, reason: Option<PauseReason>) -> Result<()> {
        conn.execute(
            "UPDATE attachment_config SET paused=?1 WHERE singleton=1",
            [reason.map(|r| serde_json::to_string(&r)).transpose()?],
        )?;
        Ok(())
    }
    fn budget(&self, conn: &Connection) -> Result<AttachmentBudget> {
        let raw: String = conn.query_row(
            "SELECT budget FROM attachment_config WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        Ok(serde_json::from_str(&raw)?)
    }
    pub fn health(&self, conn: &Connection) -> Result<AttachmentHealth> {
        let budget = self.budget(conn)?;
        let used:u64=conn.query_row("SELECT (SELECT COALESCE(SUM(size),0) FROM attachment_files WHERE state!='removed' AND kind!='external')+(SELECT COALESCE(SUM(size),0) FROM attachment_imports WHERE state IN('prepared','verified'))",[],|r|r.get(0))?;
        let paused: Option<String> = conn.query_row(
            "SELECT paused FROM attachment_config WHERE singleton=1",
            [],
            |r| r.get(0),
        )?;
        Ok(AttachmentHealth {
            blocked_imports:conn.query_row("SELECT COUNT(*) FROM attachment_imports WHERE reason IS NOT NULL AND state!='abandoned'",[],|r|r.get(0))?,
            blocked_deletions:conn.query_row("SELECT COUNT(*) FROM attachment_deletions WHERE state='blocked'",[],|r|r.get(0))?,
            used_bytes: used,
            capacity_bytes: budget.capacity_bytes,
            minimum_free_bytes: budget.minimum_free_bytes,
            paused: paused.map(|r| serde_json::from_str(&r)).transpose()?,
            pending_gc: conn.query_row(
                "SELECT COUNT(*) FROM attachment_files WHERE state IN('scheduled','quarantined')",
                [],
                |r| r.get(0),
            )?,
        })
    }
    pub fn preflight(&self, conn: &Connection, bytes: u64, available: u64) -> Result<()> {
        let health = self.health(conn)?;
        let budget = self.budget(conn)?;
        if let Some(reason) = health.paused {
            return Err(AttachmentError::Paused(reason));
        }
        if bytes > budget.max_file_bytes {
            return Err(AttachmentError::Paused(PauseReason::FileTooLarge));
        }
        if health.used_bytes.saturating_add(bytes) > budget.capacity_bytes {
            return Err(AttachmentError::Paused(PauseReason::Capacity));
        }
        if available.saturating_sub(bytes) < budget.minimum_free_bytes {
            return Err(AttachmentError::Paused(PauseReason::InsufficientSpace));
        }
        Ok(())
    }
    fn path(&self, kind: AttachmentKind, name: &str) -> Result<PathBuf> {
        if kind == AttachmentKind::External || !plain_name(name) {
            return Err(AttachmentError::Paused(PauseReason::UnsafePath));
        }
        let directory = self.root.join(kind.directory());
        secure_directory(&directory)?;
        Ok(directory.join(name))
    }
    fn read(&self, kind: AttachmentKind, name: &str, limit: u64) -> Result<(Vec<u8>, u64, u64)> {
        let path = self.path(kind, name)?;
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = options.open(&path).map_err(io_error)?;
        let meta = file.metadata().map_err(io_error)?;
        let (device, inode) = secure_file(&meta)?;
        if meta.len() > limit {
            return Err(AttachmentError::Paused(PauseReason::FileTooLarge));
        }
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(io_error)?;
        if bytes.len() as u64 != meta.len() {
            return Err(AttachmentError::Paused(PauseReason::IdentityChanged));
        }
        Ok((bytes, device, inode))
    }
    /// 阶段日志先提交，文件经验证及 fsync 后才原子发布；同名文件绝不被覆盖。
    pub fn import(
        &self,
        conn: &mut Connection,
        kind: AttachmentKind,
        operation_id: &str,
        bytes: &[u8],
    ) -> Result<AttachmentImport> {
        let work =
            self.prepare_import(conn, kind, operation_id, &hash(bytes), bytes.len() as u64)?;
        let published = self.publish_import(work, bytes)?;
        self.commit_import(conn, published)
    }
    /// 只持久化空间预留与文件操作身份；编码、完整验证和文件 IO 在调用者阻塞池进行。
    pub(crate) fn prepare_import(
        &self,
        conn: &Connection,
        kind: AttachmentKind,
        operation_id: &str,
        digest: &str,
        size: u64,
    ) -> Result<ImportWork> {
        identifier(operation_id)?;
        if kind == AttachmentKind::External
            || digest.len() != 64
            || !digest.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err(AttachmentError::Invalid("import kind or digest"));
        }
        let name = format!("{digest}.{}", kind.extension());
        if let Some((prior,prior_digest,temp,state))=conn.query_row("SELECT attachment_id,digest,temp_name,state FROM attachment_imports WHERE operation_id=?1",[operation_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?))).optional()? {
            if prior_digest!=digest || state=="abandoned" {return Err(AttachmentError::Invalid("import identity conflict"));}
            return Ok(ImportWork {kind,id:prior,digest:digest.into(),name,temp,operation_id:operation_id.into(),size,limit:self.budget(conn)?.max_file_bytes,state});
        }
        self.preflight(conn, size, available_space(&self.root)?)?;
        // 内容摘要决定文件名；attachment_id 另外绑定本次物理代际，已回收 ID 永不复活。
        let existing:Option<(String,String)>=conn.query_row("SELECT id,state FROM attachment_files WHERE kind=?1 AND name=?2 AND state!='removed'",params![kind.extension(),name],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let id = match existing {
            Some((id, state)) if state == "ready" => id,
            Some(_) => return Err(AttachmentError::Paused(PauseReason::BusyPin)),
            None => format!("{}-{digest}-{}", kind.extension(), token()?),
        };
        let temp = format!(".inputia-import-{}", token()?);
        conn.execute(
            "INSERT INTO attachment_imports(operation_id,attachment_id,digest,kind,temp_name,instance,state,size) VALUES(?1,?2,?3,?4,?5,?6,'prepared',?7)",
            params![
                operation_id,
                id,
                digest,
                kind.extension(),
                temp,
                self.instance,
                size
            ],
        )?;
        Ok(ImportWork {
            kind,
            id,
            digest: digest.into(),
            name,
            temp,
            operation_id: operation_id.into(),
            size,
            limit: self.budget(conn)?.max_file_bytes,
            state: "prepared".into(),
        })
    }
    pub(crate) fn publish_import(&self, work: ImportWork, bytes: &[u8]) -> Result<PublishedImport> {
        if bytes.len() as u64 != work.size || hash(bytes) != work.digest {
            return Err(AttachmentError::Invalid("import payload changed"));
        }
        validate(work.kind, bytes)?;
        let source = self.path(work.kind, &work.temp)?;
        let destination = self.path(work.kind, &work.name)?;
        if !matches!(work.state.as_str(), "published" | "attached") {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
            }
            match options.open(&source) {
                Ok(mut file) => {
                    file.write_all(bytes).map_err(io_error)?;
                    file.sync_all().map_err(io_error)?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(io_error(error)),
            }
            // 崩溃可留下同 inode 两链接。其他硬链接或错误临时正文不能被重试覆盖。
            let left = fs::symlink_metadata(&source).map_err(io_error)?;
            let linked = fs::symlink_metadata(&destination)
                .ok()
                .filter(|right| same_file(&left, right));
            if linked.is_some() {
                check_import_pair(&left)?;
            } else {
                let (saved, _, _) = self.read(work.kind, &work.temp, work.limit)?;
                validate(work.kind, &saved)?;
                if hash(&saved) != work.digest {
                    return Err(AttachmentError::Paused(PauseReason::IdentityChanged));
                }
                match fs::hard_link(&source, &destination) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(io_error(error)),
                }
            }
            let left = fs::symlink_metadata(&source).map_err(io_error)?;
            let right = fs::symlink_metadata(&destination).map_err(io_error)?;
            if same_file(&left, &right) {
                check_import_pair(&right)?;
            } else {
                let (saved, _, _) = self.read(work.kind, &work.name, work.limit)?;
                if hash(&saved) != work.digest {
                    return Err(AttachmentError::Paused(PauseReason::IdentityChanged));
                }
            }
            fs::remove_file(&source).map_err(io_error)?;
            File::open(self.root.join(work.kind.directory()))
                .and_then(|dir| dir.sync_all())
                .map_err(io_error)?;
        }
        let (saved, device, inode) = self.read(work.kind, &work.name, work.limit)?;
        validate(work.kind, &saved)?;
        if hash(&saved) != work.digest {
            return Err(AttachmentError::Paused(PauseReason::IdentityChanged));
        }
        Ok(PublishedImport {
            work,
            device,
            inode,
        })
    }
    pub(crate) fn commit_import(
        &self,
        conn: &mut Connection,
        published: PublishedImport,
    ) -> Result<AttachmentImport> {
        let PublishedImport {
            mut work,
            device,
            inode,
        } = published;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing:Option<(String,String,u64,u64)>=tx.query_row("SELECT id,state,device,inode FROM attachment_files WHERE kind=?1 AND name=?2 AND state!='removed'",params![work.kind.extension(),work.name],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        if let Some((id, state, old_device, old_inode)) = existing {
            if !matches!(state.as_str(), "ready" | "scheduled")
                || old_device != device
                || old_inode != inode
            {
                return Err(AttachmentError::Paused(PauseReason::IdentityChanged));
            }
            // 并行相同内容最终收敛同一物理代际；每个未完成导入仍独立持有 pin。
            work.id = id;
            tx.execute(
                "UPDATE attachment_files SET state='ready' WHERE id=?1",
                [&work.id],
            )?;
        } else {
            tx.execute("INSERT INTO attachment_files(id,kind,name,digest,size,device,inode,state) VALUES(?1,?2,?3,?4,?5,?6,?7,'ready')",params![work.id,work.kind.extension(),work.name,work.digest,work.size,device,inode])?;
        }
        tx.execute("UPDATE attachment_imports SET attachment_id=?2,state=CASE WHEN state='attached' THEN 'attached' ELSE 'published' END WHERE operation_id=?1",params![work.operation_id,work.id])?;
        tx.commit()?;
        Ok(AttachmentImport {
            attachment_id: work.id,
            file_name: work.name,
            operation_id: work.operation_id,
        })
    }

    fn recover_import(&self, conn: &mut Connection, operation_id: &str) -> Result<()> {
        let (id,digest,kind,temp,state):(String,String,String,String,String)=conn.query_row("SELECT attachment_id,digest,kind,temp_name,state FROM attachment_imports WHERE operation_id=?1",[operation_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
        let kind = AttachmentKind::parse(&kind)?;
        let name = format!("{digest}.{}", kind.extension());
        if state == "abandoned" {
            return Err(AttachmentError::Invalid("import abandoned"));
        }
        if state == "attached" {
            self.verify_registered(conn, &id)?;
            return Ok(());
        }
        let source = self.path(kind, &temp)?;
        let destination = self.path(kind, &name)?;
        if state == "prepared" && !source.exists() && destination.exists() {
            let (bytes, _, _) = self.read(kind, &name, self.budget(conn)?.max_file_bytes)?;
            validate(kind, &bytes)?;
            if hash(&bytes) != digest {
                return Err(AttachmentError::Paused(PauseReason::IdentityChanged));
            }
        } else if state == "prepared"
            && source.exists()
            && destination.exists()
            && same_file(
                &fs::symlink_metadata(&source).map_err(io_error)?,
                &fs::symlink_metadata(&destination).map_err(io_error)?,
            )
        {
            check_import_pair(&fs::symlink_metadata(&source).map_err(io_error)?)?;
        } else if state == "prepared" {
            let (bytes, _, _) = self.read(kind, &temp, self.budget(conn)?.max_file_bytes)?;
            validate(kind, &bytes)?;
            if hash(&bytes) != digest {
                return Err(AttachmentError::Paused(PauseReason::IdentityChanged));
            }
            conn.execute(
                "UPDATE attachment_imports SET state='verified' WHERE operation_id=?1",
                [operation_id],
            )?;
        }
        // hard_link 提供同卷、原子、禁止覆盖的发布；随后删除已知 staging 链接。
        // 只允许本日志记录的同 inode 双链接中间态，其他硬链接一律拒绝。
        if source.exists() {
            match fs::hard_link(&source, &destination) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(io_error(error)),
            }
            let before = fs::symlink_metadata(&source).map_err(io_error)?;
            let final_meta = fs::symlink_metadata(&destination).map_err(io_error)?;
            if !same_file(&before, &final_meta) {
                self.read(kind, &name, self.budget(conn)?.max_file_bytes)
                    .and_then(|(bytes, _, _)| {
                        if hash(&bytes) == digest {
                            Ok(())
                        } else {
                            Err(AttachmentError::Paused(PauseReason::IdentityChanged))
                        }
                    })?;
            } else {
                check_import_pair(&before)?;
            }
            fs::remove_file(&source).map_err(io_error)?;
            File::open(self.root.join(kind.directory()))
                .and_then(|dir| dir.sync_all())
                .map_err(io_error)?;
        }
        let (bytes, device, inode) = self.read(kind, &name, self.budget(conn)?.max_file_bytes)?;
        validate(kind, &bytes)?;
        if hash(&bytes) != digest {
            return Err(AttachmentError::Paused(PauseReason::IdentityChanged));
        }
        self.commit_import(
            conn,
            PublishedImport {
                work: ImportWork {
                    kind,
                    id,
                    digest,
                    name,
                    temp,
                    operation_id: operation_id.into(),
                    size: bytes.len() as u64,
                    limit: self.budget(conn)?.max_file_bytes,
                    state,
                },
                device,
                inode,
            },
        )?;
        Ok(())
    }

    /// 只登记数据库明确引用的文件；绝对外部路径只登记一个不透明外部身份。
    fn adopt(&self, conn: &Connection, reference: &AttachmentReference) -> Result<Option<String>> {
        let Some(path) = reference.path.as_deref().filter(|path| !path.is_empty()) else {
            return Ok(None);
        };
        let raw = Path::new(path);
        let (kind, name) = if reference.kind == AttachmentKind::External {
            (AttachmentKind::External, hash(path.as_bytes()))
        } else if raw.is_absolute() {
            match raw.strip_prefix(self.root.join(reference.kind.directory())) {
                Ok(name) if name.to_str().is_some_and(plain_name) => {
                    (reference.kind, name.to_string_lossy().into_owned())
                }
                _ => (AttachmentKind::External, hash(path.as_bytes())),
            }
        } else {
            (reference.kind, path.to_owned())
        };
        if kind == AttachmentKind::External {
            let id = format!("external-{name}");
            conn.execute("INSERT OR IGNORE INTO attachment_files(id,kind,name,digest,size,device,inode,state) VALUES(?1,'external',?2,?2,0,0,0,'ready')",params![id,name])?;
            return Ok(Some(id));
        }
        if let Some(id) = conn
            .query_row(
                "SELECT id FROM attachment_files WHERE kind=?1 AND name=?2 AND state IN('ready','scheduled')",
                params![kind.extension(), name],
                |r| r.get::<_, String>(0),
            )
            .optional()?
        {
            self.verify_registered(conn, &id)?;
            conn.execute("UPDATE attachment_files SET state='ready' WHERE id=?1 AND state='scheduled'",[&id])?;
            return Ok(Some(id));
        }
        let (bytes, device, inode) = self.read(kind, &name, self.budget(conn)?.max_file_bytes)?;
        validate(kind, &bytes)?;
        let digest = hash(&bytes);
        // 旧文件名可能不含摘要；身份也绑定路径，避免把不同 legacy 文件误算作同一物理文件。
        let id = if name == format!("{digest}.{}", kind.extension()) {
            format!("{}-{digest}-{}", kind.extension(), token()?)
        } else {
            format!(
                "legacy-{}-{}",
                hash(format!("{}:{name}:{digest}", kind.extension()).as_bytes()),
                token()?
            )
        };
        if let Some(existing) = conn
            .query_row(
                "SELECT id FROM attachment_files WHERE kind=?1 AND name=?2 AND state!='removed'",
                params![kind.extension(), name],
                |r| r.get::<_, String>(0),
            )
            .optional()?
        {
            if existing != id {
                return Err(AttachmentError::Paused(PauseReason::IdentityChanged));
            }
            self.verify_registered(conn, &id)?;
        } else {
            conn.execute("INSERT INTO attachment_files(id,kind,name,digest,size,device,inode,state) VALUES(?1,?2,?3,?4,?5,?6,?7,'ready')",params![id,kind.extension(),name,digest,bytes.len() as u64,device,inode])?;
        }
        Ok(Some(id))
    }

    fn verify_registered(&self, conn: &Connection, id: &str) -> Result<PathBuf> {
        let (kind, name, digest, size, device, inode, state): (
            String,
            String,
            String,
            u64,
            u64,
            u64,
            String,
        ) = conn.query_row(
            "SELECT kind,name,digest,size,device,inode,state FROM attachment_files WHERE id=?1",
            [id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            },
        )?;
        let kind = AttachmentKind::parse(&kind)?;
        if kind == AttachmentKind::External {
            return Err(AttachmentError::Paused(PauseReason::UnsafePath));
        }
        if !matches!(state.as_str(), "ready" | "scheduled") {
            return Err(AttachmentError::Paused(PauseReason::BusyPin));
        }
        let path = self.path(kind, &name)?;
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW);
        }
        let file = options.open(&path).map_err(io_error)?;
        let meta = file.metadata().map_err(io_error)?;
        let (current_device, current_inode) = secure_file(&meta)?;
        let _ = digest;
        if current_device != device || current_inode != inode || meta.len() != size {
            return Err(AttachmentError::Paused(PauseReason::IdentityChanged));
        }
        self.path(kind, &name)
    }

    /// 仅在全部源追平后提交完整引用快照。错误 owner 保留旧引用，不能以审计失败当作零引用。
    pub fn reconcile(
        &self,
        conn: &mut Connection,
        references: &[AttachmentReference],
    ) -> Result<()> {
        let mut good = Vec::new();
        let mut bad = std::collections::HashSet::new();
        for reference in references {
            identifier(&reference.store_id)?;
            identifier(&reference.record_id)?;
            match self.adopt(conn, reference) {
                Ok(id) => {
                    if let Some(id) = id {
                        good.push((reference, id));
                    }
                }
                Err(error) => {
                    bad.insert((reference.store_id.clone(), reference.record_id.clone()));
                    self.owner_failure(
                        conn,
                        &reference.store_id,
                        &reference.record_id,
                        reference.revision,
                        Some(error.reason()),
                    )?;
                }
            }
        }
        let pending = {
            let mut q=conn.prepare("SELECT operation_id,kind,temp_name,digest FROM attachment_imports WHERE state IN('prepared','verified') AND (instance<>?1 OR released=1)")?;
            let rows = q
                .query_map([&self.instance], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        for (operation, kind, temp, digest) in pending {
            let kind = AttachmentKind::parse(&kind)?;
            let name = format!("{digest}.{}", kind.extension());
            let final_path = self.path(kind, &name)?;
            let stage = self.path(kind, &temp)?;
            let referenced = references.iter().any(|r| {
                r.kind == kind
                    && r.path
                        .as_deref()
                        .is_some_and(|p| p == name || Path::new(p) == final_path)
            });
            if !referenced
                && matches!(fs::symlink_metadata(&stage),Err(e) if e.kind()==std::io::ErrorKind::NotFound)
                && matches!(fs::symlink_metadata(&final_path),Err(e) if e.kind()==std::io::ErrorKind::NotFound)
            {
                conn.execute("UPDATE attachment_imports SET state='abandoned',reason=NULL WHERE operation_id=?1",[operation])?;
            }
        }
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch("CREATE TEMP TABLE IF NOT EXISTS attachment_bad_owners(store_id TEXT,record_id TEXT);DELETE FROM attachment_bad_owners;")?;
        for (store, record) in &bad {
            tx.execute(
                "INSERT INTO attachment_bad_owners VALUES(?1,?2)",
                params![store, record],
            )?;
        }
        tx.execute("DELETE FROM attachment_refs WHERE NOT EXISTS(SELECT 1 FROM attachment_bad_owners bad WHERE bad.store_id=attachment_refs.store_id AND bad.record_id=attachment_refs.record_id)",[])?;
        for (reference, id) in good {
            tx.execute(
                "INSERT OR IGNORE INTO attachment_refs VALUES(?1,?2,?3,?4,?5)",
                params![
                    reference.store_id,
                    reference.record_id,
                    reference.revision,
                    if reference.retained {
                        "revision"
                    } else {
                        "current"
                    },
                    id
                ],
            )?;
        }
        for reference in references {
            if reference.path.is_some()
                && !bad.contains(&(reference.store_id.clone(), reference.record_id.clone()))
            {
                self.owner_failure(
                    &tx,
                    &reference.store_id,
                    &reference.record_id,
                    reference.revision,
                    None,
                )?;
            }
        }
        tx.execute("UPDATE attachment_imports SET state='attached' WHERE state='published' AND (released=1 OR instance<>?1) AND EXISTS(SELECT 1 FROM attachment_refs r WHERE r.attachment_id=attachment_imports.attachment_id)",[&self.instance])?;
        // 新进程只有完成全部源审计，才可判定旧进程的未关联导入没有提交源记录。
        tx.execute("UPDATE attachment_imports SET state='abandoned' WHERE state='published' AND (instance<>?1 OR released=1) AND NOT EXISTS(SELECT 1 FROM attachment_refs r WHERE r.attachment_id=attachment_imports.attachment_id)",[&self.instance])?;
        tx.execute(
            "UPDATE attachment_pins SET active=0 WHERE instance<>?1 AND purpose='active'",
            [&self.instance],
        )?;
        tx.execute("UPDATE attachment_files SET state='scheduled' WHERE state='ready' AND EXISTS(SELECT 1 FROM attachment_imports i WHERE i.attachment_id=attachment_files.id AND i.state='abandoned') AND NOT EXISTS(SELECT 1 FROM attachment_refs r WHERE r.attachment_id=attachment_files.id)",[])?;
        tx.commit()?;
        if !bad.is_empty() {
            return Err(AttachmentError::Paused(PauseReason::SourceAuditPending));
        }
        Ok(())
    }

    pub fn reconcile_owner(
        &self,
        conn: &mut Connection,
        references: &[AttachmentReference],
    ) -> Result<()> {
        let mut adopted = Vec::new();
        for reference in references {
            adopted.push((reference, self.adopt(conn, reference)?));
        }
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        for (reference, id) in adopted {
            tx.execute(
                "DELETE FROM attachment_refs WHERE store_id=?1 AND record_id=?2 AND role='current'",
                params![reference.store_id, reference.record_id],
            )?;
            if let Some(id) = id {
                tx.execute(
                    "INSERT OR IGNORE INTO attachment_refs VALUES(?1,?2,?3,'current',?4)",
                    params![
                        reference.store_id,
                        reference.record_id,
                        reference.revision,
                        id
                    ],
                )?;
            }
        }
        tx.execute("UPDATE attachment_imports SET state='attached' WHERE state='published' AND (released=1 OR instance<>?1) AND EXISTS(SELECT 1 FROM attachment_refs r WHERE r.attachment_id=attachment_imports.attachment_id)",[&self.instance])?;
        tx.commit()?;
        Ok(())
    }
    pub fn recover_imports(&self, conn: &mut Connection) -> Result<()> {
        let ids = {
            let mut q=conn.prepare("SELECT operation_id FROM attachment_imports WHERE state IN('prepared','verified','published')")?;
            let rows = q
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        for id in ids {
            if let Err(error) = self.recover_import(conn, &id) {
                conn.execute(
                    "UPDATE attachment_imports SET reason=?2 WHERE operation_id=?1",
                    params![id, serde_json::to_string(&error.reason())?],
                )?;
            } else {
                conn.execute(
                    "UPDATE attachment_imports SET reason=NULL WHERE operation_id=?1",
                    [id],
                )?;
            }
        }
        Ok(())
    }
    pub fn import_status(
        &self,
        conn: &Connection,
        operation: &str,
    ) -> Result<Option<AttachmentImportStatus>> {
        let row:Option<(String,String,Option<String>,bool)>=conn.query_row("SELECT attachment_id,state,reason,released FROM attachment_imports WHERE operation_id=?1",[operation],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        row.map(|(attachment_id, state, reason, released)| {
            Ok(AttachmentImportStatus {
                operation_id: operation.into(),
                attachment_id,
                state,
                reason: reason
                    .map(|reason| serde_json::from_str(&reason))
                    .transpose()?,
                released,
            })
        })
        .transpose()
    }
    pub fn finish_import(&self, conn: &Connection, operation: &str) -> Result<()> {
        identifier(operation)?;
        conn.execute(
            "UPDATE attachment_imports SET released=1 WHERE operation_id=?1",
            [operation],
        )?;
        Ok(())
    }
    pub fn owner_status(
        &self,
        conn: &Connection,
        store: &str,
        record: &str,
    ) -> Result<Option<PauseReason>> {
        let raw: Option<String> = conn
            .query_row(
                "SELECT reason FROM attachment_owner_status WHERE store_id=?1 AND record_id=?2",
                params![store, record],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        Ok(raw.map(|raw| serde_json::from_str(&raw)).transpose()?)
    }
    pub fn owner_failure(
        &self,
        conn: &Connection,
        store: &str,
        record: &str,
        revision: u64,
        reason: Option<PauseReason>,
    ) -> Result<()> {
        conn.execute("INSERT INTO attachment_owner_status VALUES(?1,?2,?3,?4) ON CONFLICT(store_id,record_id) DO UPDATE SET revision=excluded.revision,reason=excluded.reason",params![store,record,revision,reason.map(|r|serde_json::to_string(&r)).transpose()?])?;
        Ok(())
    }
    pub fn acquire(
        &self,
        conn: &Connection,
        attachment_id: Option<&str>,
        purpose: PinPurpose,
        lease_id: &str,
    ) -> Result<AttachmentLease> {
        identifier(lease_id)?;
        if conn.query_row("SELECT EXISTS(SELECT 1 FROM attachment_acquire_cancellations WHERE instance=?1 AND operation_id=?2)",params![self.instance,lease_id],|r|r.get::<_,bool>(0))? {return Err(AttachmentError::Invalid("attachment acquire cancelled"));}
        if attachment_id.is_none() && purpose != PinPurpose::Update {
            return Err(AttachmentError::Invalid(
                "only maintenance may pin all attachments",
            ));
        }
        if let Some(id) = attachment_id {
            self.verify_registered(conn, id)?;
        }
        let purpose_name = match purpose {
            PinPurpose::Active => "active",
            PinPurpose::Export => "export",
            PinPurpose::Update => "update",
        };
        if let Some((instance,asset,old_purpose,active))=conn.query_row("SELECT instance,attachment_id,purpose,active FROM attachment_pins WHERE lease_id=?1",[lease_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,String>(2)?,r.get::<_,bool>(3)?))).optional()? {
            if instance!=self.instance || asset.as_deref()!=attachment_id || old_purpose!=purpose_name || !active {return Err(AttachmentError::Invalid("lease identity conflict"));}
        } else {conn.execute("INSERT INTO attachment_pins VALUES(?1,?2,?3,?4,1)",params![lease_id,self.instance,attachment_id,purpose_name])?;}
        Ok(AttachmentLease {
            lease_id: lease_id.into(),
            instance_id: self.instance.clone(),
            attachment_id: attachment_id.map(str::to_owned),
            purpose,
        })
    }
    pub fn cancel_acquire(&self, conn: &Connection, operation_id: &str) -> Result<()> {
        identifier(operation_id)?;
        conn.execute(
            "INSERT OR IGNORE INTO attachment_acquire_cancellations VALUES(?1,?2)",
            params![self.instance, operation_id],
        )?;
        conn.execute("UPDATE attachment_pins SET active=0 WHERE lease_id=?1 AND instance=?2 AND purpose='active'",params![operation_id,self.instance])?;
        Ok(())
    }
    pub fn release(&self, conn: &Connection, lease: &AttachmentLease) -> Result<()> {
        let expected = match lease.purpose {
            PinPurpose::Active => "active",
            PinPurpose::Export => "export",
            PinPurpose::Update => "update",
        };
        let row: (String, Option<String>, String) = conn.query_row(
            "SELECT instance,attachment_id,purpose FROM attachment_pins WHERE lease_id=?1",
            [&lease.lease_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        if row.0 != lease.instance_id || row.1 != lease.attachment_id || row.2 != expected {
            return Err(AttachmentError::Invalid("lease owner mismatch"));
        }
        conn.execute(
            "UPDATE attachment_pins SET active=0 WHERE lease_id=?1",
            [&lease.lease_id],
        )?;
        Ok(())
    }
    pub fn acquire_owner(
        &self,
        conn: &Connection,
        store: &str,
        record: &str,
        revision: u64,
        purpose: PinPurpose,
        lease_id: &str,
    ) -> Result<(AttachmentLease, PathBuf)> {
        let id:String=conn.query_row("SELECT attachment_id FROM attachment_refs WHERE store_id=?1 AND record_id=?2 AND revision=?3 AND role='current'",params![store,record,revision],|r|r.get(0))?;
        let lease = self.acquire(conn, Some(&id), purpose, lease_id)?;
        let path = self.verify_registered(conn, &id)?;
        Ok((lease, path))
    }
    pub(crate) fn lease_read_plan(
        &self,
        conn: &Connection,
        lease: &AttachmentLease,
    ) -> Result<AttachmentRead> {
        let active:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM attachment_pins WHERE lease_id=?1 AND instance=?2 AND attachment_id=?3 AND active=1)",params![lease.lease_id,lease.instance_id,lease.attachment_id],|r|r.get(0))?;
        if !active {
            return Err(AttachmentError::Invalid("lease inactive"));
        }
        let id = lease
            .attachment_id
            .as_deref()
            .ok_or(AttachmentError::Invalid("global lease has no file"))?;
        let (kind, name, digest, size, device, inode): (String, String, String, u64, u64, u64) =
            conn.query_row(
                "SELECT kind,name,digest,size,device,inode FROM attachment_files WHERE id=?1",
                [id],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                },
            )?;
        Ok(AttachmentRead {
            kind: AttachmentKind::parse(&kind)?,
            name,
            digest,
            size,
            device,
            inode,
        })
    }
    pub(crate) fn read_plan(&self, plan: AttachmentRead) -> Result<Vec<u8>> {
        let (bytes, device, inode) = self.read(plan.kind, &plan.name, plan.size)?;
        if device != plan.device || inode != plan.inode || hash(&bytes) != plan.digest {
            return Err(AttachmentError::Paused(PauseReason::IdentityChanged));
        }
        Ok(bytes)
    }
    pub fn read_leased(&self, conn: &Connection, lease: &AttachmentLease) -> Result<Vec<u8>> {
        self.read_plan(self.lease_read_plan(conn, lease)?)
    }

    pub fn record_manifest(
        &self,
        conn: &mut Connection,
        request: &crate::deletion_lifecycle::DeleteRequest,
        source: Option<&[crate::source::DeletedAttachment]>,
        retained: &[AttachmentReference],
    ) -> Result<()> {
        #[derive(Serialize, Deserialize)]
        struct Manifest {
            source: Option<Vec<crate::source::DeletedAttachment>>,
            retained: Vec<AttachmentReference>,
            ids: std::collections::BTreeSet<String>,
        }
        let request_digest = hash(&serde_json::to_vec(request)?);
        let prior:Option<(String,String,String,String,String)>=conn.query_row("SELECT request_digest,ids,state,manifest_json,manifest_digest FROM attachment_deletions WHERE operation_id=?1",[&request.operation_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
        let mut evidence = if let Some((digest, ids, state, raw, manifest_digest)) = &prior {
            if digest != &request_digest {
                return Err(AttachmentError::Invalid(
                    "attachment deletion identity conflict",
                ));
            }
            if hash(raw.as_bytes()) != *manifest_digest {
                return self.block_manifest(
                    conn,
                    &request.operation_id,
                    PauseReason::IdentityChanged,
                );
            }
            let manifest: Manifest = serde_json::from_str(raw)?;
            if manifest.source.as_deref() != source
                || serde_json::from_str::<std::collections::BTreeSet<String>>(ids)? != manifest.ids
            {
                return self.block_manifest(
                    conn,
                    &request.operation_id,
                    PauseReason::IdentityChanged,
                );
            }
            if state == "completed" && !self.ids_settled(conn, &manifest.ids)? {
                return self.block_manifest(
                    conn,
                    &request.operation_id,
                    PauseReason::IdentityChanged,
                );
            }
            manifest
        } else {
            let ids = {
                let mut query = conn.prepare(
                    "SELECT attachment_id FROM attachment_refs WHERE store_id=?1 AND record_id=?2",
                )?;
                let ids = query
                    .query_map(params![request.store_id, request.record_id], |r| {
                        r.get::<_, String>(0)
                    })?
                    .collect::<rusqlite::Result<std::collections::BTreeSet<_>>>()?;
                ids
            };
            Manifest {
                source: source.map(<[_]>::to_vec),
                retained: retained.to_vec(),
                ids,
            }
        };
        let mut reason = if source.is_none() {
            Some(PauseReason::LegacyManifestUnknown)
        } else {
            None
        };
        let mut references = evidence.retained.clone();
        for attachment in evidence.source.as_deref().unwrap_or_default() {
            references.push(AttachmentReference {
                store_id: request.store_id.clone(),
                record_id: request.record_id.clone(),
                revision: request.expected_revision,
                kind: attachment.kind,
                path: Some(attachment.path.clone()),
                retained: false,
            });
        }
        for reference in &references {
            // 已完成/隔离的文件不重读已消失路径；仍必须证明每个清单项都映射到绑定的物理代际。
            let identity = reference
                .path
                .as_deref()
                .and_then(|path| self.reference_name(reference.kind, path).ok());
            let mut known = false;
            if let Some((kind, name)) = identity {
                for id in &evidence.ids {
                    let matches:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM attachment_files WHERE id=?1 AND kind=?2 AND name=?3)",params![id,kind.extension(),name],|r|r.get(0))?;
                    known |= matches;
                }
            }
            if !known {
                match self.adopt(conn, reference) {
                    Ok(Some(id)) => {
                        evidence.ids.insert(id);
                    }
                    Ok(None) => {}
                    Err(error) => reason = Some(error.reason()),
                }
            }
        }
        if prior.as_ref().is_some_and(|p| p.2 == "completed")
            && !self.ids_settled(conn, &evidence.ids)?
        {
            return self.block_manifest(conn, &request.operation_id, PauseReason::IdentityChanged);
        }
        let raw = serde_json::to_string(&evidence)?;
        let manifest_digest = hash(raw.as_bytes());
        let state = if reason.is_some() {
            "blocked"
        } else {
            prior
                .as_ref()
                .map(|p| p.2.as_str())
                .filter(|s| matches!(*s, "scheduled" | "completed"))
                .unwrap_or("recorded")
        };
        conn.execute("INSERT INTO attachment_deletions(operation_id,request_digest,ids,state,reason,manifest_json,manifest_digest) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(operation_id) DO UPDATE SET ids=excluded.ids,state=excluded.state,reason=excluded.reason,manifest_json=excluded.manifest_json,manifest_digest=excluded.manifest_digest",params![request.operation_id,request_digest,serde_json::to_string(&evidence.ids)?,state,reason.map(|r|serde_json::to_string(&r)).transpose()?,raw,manifest_digest])?;
        Ok(())
    }
    fn block_manifest(
        &self,
        conn: &Connection,
        operation: &str,
        reason: PauseReason,
    ) -> Result<()> {
        conn.execute(
            "UPDATE attachment_deletions SET state='blocked',reason=?2 WHERE operation_id=?1",
            params![operation, serde_json::to_string(&reason)?],
        )?;
        Ok(())
    }
    fn reference_name(&self, kind: AttachmentKind, path: &str) -> Result<(AttachmentKind, String)> {
        if kind == AttachmentKind::External {
            return Ok((kind, hash(path.as_bytes())));
        }
        let raw = Path::new(path);
        if raw.is_absolute() {
            return match raw.strip_prefix(self.root.join(kind.directory())) {
                Ok(name) if name.to_str().is_some_and(plain_name) => {
                    Ok((kind, name.to_string_lossy().into_owned()))
                }
                _ => Ok((AttachmentKind::External, hash(path.as_bytes()))),
            };
        }
        if !plain_name(path) {
            return Err(AttachmentError::Paused(PauseReason::UnsafePath));
        }
        Ok((kind, path.into()))
    }
    fn ids_settled(
        &self,
        conn: &Connection,
        ids: &std::collections::BTreeSet<String>,
    ) -> Result<bool> {
        for id in ids {
            let (state,kind,name,device,inode,quarantine,shared):(String,String,String,u64,u64,Option<String>,bool)=conn.query_row("SELECT state,kind,name,device,inode,quarantine,EXISTS(SELECT 1 FROM attachment_refs WHERE attachment_id=?1) FROM attachment_files WHERE id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?)))?;
            if kind == "external" || shared {
                continue;
            }
            if state != "removed" {
                return Ok(false);
            }
            let kind = AttachmentKind::parse(&kind)?;
            // removed 标签也不是物理证据；原 inode 不得仍留在原名或登记的隔离名。
            for name in std::iter::once(name).chain(quarantine) {
                match fs::symlink_metadata(self.path(kind, &name)?) {
                    Ok(meta) if matches_identity(&meta, device, inode) => return Ok(false),
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(io_error(error)),
                }
            }
        }
        Ok(true)
    }

    /// 与 source/projection 的删除结算分别记账；只撤销当前 owner，保留共享和活跃租约。
    pub fn schedule_deletion(
        &self,
        conn: &mut Connection,
        request: &crate::deletion_lifecycle::DeleteRequest,
    ) -> Result<()> {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (digest, ids, state): (String, String, String) = tx.query_row(
            "SELECT request_digest,ids,state FROM attachment_deletions WHERE operation_id=?1",
            [&request.operation_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        if digest != hash(&serde_json::to_vec(request)?) {
            return Err(AttachmentError::Invalid(
                "attachment deletion digest conflict",
            ));
        }
        tx.execute(
            "DELETE FROM attachment_refs WHERE store_id=?1 AND record_id=?2",
            params![request.store_id, request.record_id],
        )?;
        if state != "blocked" && state != "completed" {
            for id in serde_json::from_str::<Vec<String>>(&ids)? {
                tx.execute("UPDATE attachment_files SET state='scheduled' WHERE id=?1 AND state='ready' AND NOT EXISTS(SELECT 1 FROM attachment_refs WHERE attachment_id=?1)",[id])?;
            }
            tx.execute(
                "UPDATE attachment_deletions SET state='scheduled' WHERE operation_id=?1",
                [&request.operation_id],
            )?;
        }
        tx.commit()?;
        self.refresh_deletion_progress(conn)?;
        Ok(())
    }

    pub fn deletion_progress(
        &self,
        conn: &Connection,
        operation_id: &str,
    ) -> Result<(
        crate::deletion_lifecycle::AttachmentCleanup,
        Option<PauseReason>,
    )> {
        use crate::deletion_lifecycle::AttachmentCleanup;
        let row: Option<(String, Option<String>)> = conn
            .query_row(
                "SELECT state,reason FROM attachment_deletions WHERE operation_id=?1",
                [operation_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match row {
            None => Ok((AttachmentCleanup::NotStarted, None)),
            Some((state, reason)) => Ok((
                match state.as_str() {
                    "completed" => AttachmentCleanup::Completed,
                    "blocked" => AttachmentCleanup::Blocked,
                    _ => AttachmentCleanup::Scheduled,
                },
                reason.map(|r| serde_json::from_str(&r)).transpose()?,
            )),
        }
    }
    pub fn refresh_deletion_progress(&self, conn: &Connection) -> Result<()> {
        let mut stmt = conn
            .prepare("SELECT operation_id,ids FROM attachment_deletions WHERE state='scheduled'")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);
        for (operation, ids) in rows {
            let (manifest,digest):(String,String)=conn.query_row("SELECT manifest_json,manifest_digest FROM attachment_deletions WHERE operation_id=?1",[&operation],|r|Ok((r.get(0)?,r.get(1)?)))?;
            let parsed: serde_json::Value = serde_json::from_str(&manifest)?;
            if hash(manifest.as_bytes()) != digest
                || parsed.get("ids") != Some(&serde_json::from_str::<serde_json::Value>(&ids)?)
            {
                self.block_manifest(conn, &operation, PauseReason::IdentityChanged)?;
                continue;
            }
            let complete = self.ids_settled(conn, &serde_json::from_str(&ids)?)?;
            if complete {
                conn.execute("UPDATE attachment_deletions SET state='completed',reason=NULL WHERE operation_id=?1",[operation])?;
            }
        }
        conn.execute("UPDATE integration_deletion_operations SET attachment_cleanup=COALESCE((SELECT CASE state WHEN 'completed' THEN 'completed' WHEN 'blocked' THEN 'blocked' ELSE 'scheduled' END FROM attachment_deletions WHERE operation_id=integration_deletion_operations.operation_id),'not_started') WHERE state='projection_revoked'",[])?;
        Ok(())
    }

    /// 调用方已持真正排他源写门禁并完成引用审计；每次最多回收一个精确登记的文件。
    pub fn collect_one(
        &self,
        conn: &mut Connection,
        _exclusive: &AttachmentGcGuard,
        maintenance_user: &inputia_settings::maintenance::UserContext,
    ) -> Result<bool> {
        if maintenance_present_or_unsafe(maintenance_user) {
            return Err(AttachmentError::Paused(PauseReason::Maintenance));
        }
        if let Some(reason) = self.health(conn)?.paused {
            return Err(AttachmentError::Paused(reason));
        }
        let global: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM attachment_pins WHERE attachment_id IS NULL AND active=1)",
            [],
            |r| r.get(0),
        )?;
        if global {
            return Err(AttachmentError::Paused(PauseReason::BusyPin));
        }
        type Candidate = (
            String,
            String,
            String,
            String,
            u64,
            u64,
            u64,
            String,
            Option<String>,
        );
        let read_candidate = |after: Option<&str>| -> Result<Option<Candidate>> {
            Ok(conn.query_row("SELECT id,kind,name,digest,size,device,inode,state,quarantine FROM attachment_files f WHERE state IN('scheduled','quarantined') AND NOT EXISTS(SELECT 1 FROM attachment_refs WHERE attachment_id=f.id) AND NOT EXISTS(SELECT 1 FROM attachment_pins WHERE attachment_id=f.id AND active=1) AND NOT EXISTS(SELECT 1 FROM attachment_imports WHERE attachment_id=f.id AND state IN('prepared','verified','published')) AND (?1 IS NULL OR f.id>?1) AND NOT EXISTS(SELECT 1 FROM attachment_deletions d,json_each(d.manifest_json,'$.ids') held WHERE d.state='blocked' AND held.value=f.id) ORDER BY f.id LIMIT 1",[after],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?,r.get(8)?))).optional()?)
        };
        let cursor = self
            .gc_cursor
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let mut row = read_candidate(cursor.as_deref())?;
        if row.is_none() && cursor.is_some() {
            row = read_candidate(None)?;
        }
        *self.gc_cursor.lock().unwrap_or_else(|e| e.into_inner()) =
            row.as_ref().map(|row| row.0.clone());
        let Some((id, kind, name, digest, size, device, inode, state, quarantine)) = row else {
            self.refresh_deletion_progress(conn)?;
            return Ok(false);
        };
        let kind = AttachmentKind::parse(&kind)?;
        if kind == AttachmentKind::External {
            conn.execute(
                "UPDATE attachment_files SET state='removed' WHERE id=?1",
                [id],
            )?;
            self.refresh_deletion_progress(conn)?;
            return Ok(true);
        }
        let quarantine = quarantine.unwrap_or(format!(".inputia-gc-{}", token()?));
        let original = self.path(kind, &name)?;
        let trash = self.path(kind, &quarantine)?;
        let work = (|| {
            if state == "scheduled" {
                conn.execute(
                    "UPDATE attachment_files SET state='quarantined',quarantine=?2 WHERE id=?1",
                    params![id, quarantine],
                )?;
            }
            if !trash.exists() {
                if !original.exists() {
                    return Err(AttachmentError::Paused(PauseReason::MissingFile));
                }
                let (bytes, current_device, current_inode) = self.read(kind, &name, size)?;
                if hash(&bytes) != digest || device != current_device || inode != current_inode {
                    return Err(AttachmentError::Paused(PauseReason::IdentityChanged));
                }
                // 随机、日志绑定的 quarantine 名不可覆盖已有文件。
                fs::hard_link(&original, &trash).map_err(io_error)?;
                let left = fs::symlink_metadata(&original).map_err(io_error)?;
                let right = fs::symlink_metadata(&trash).map_err(io_error)?;
                if !same_file(&left, &right) || !matches_identity(&right, device, inode) {
                    return Err(AttachmentError::Paused(PauseReason::IdentityChanged));
                }
                check_import_pair(&right)?;
                if maintenance_present_or_unsafe(maintenance_user) {
                    return Err(AttachmentError::Paused(PauseReason::Maintenance));
                }
                fs::remove_file(&original).map_err(io_error)?;
                File::open(self.root.join(kind.directory()))
                    .and_then(|dir| dir.sync_all())
                    .map_err(io_error)?;
            } else if original.exists() {
                let left = fs::symlink_metadata(&original).map_err(io_error)?;
                let right = fs::symlink_metadata(&trash).map_err(io_error)?;
                if !same_file(&left, &right) || !matches_identity(&right, device, inode) {
                    return Err(AttachmentError::Paused(PauseReason::IdentityChanged));
                }
                check_import_pair(&right)?;
                if maintenance_present_or_unsafe(maintenance_user) {
                    return Err(AttachmentError::Paused(PauseReason::Maintenance));
                }
                fs::remove_file(&original).map_err(io_error)?;
            }
            let (bytes, current_device, current_inode) = self.read(kind, &quarantine, size)?;
            if hash(&bytes) != digest || device != current_device || inode != current_inode {
                return Err(AttachmentError::Paused(PauseReason::IdentityChanged));
            }
            if maintenance_present_or_unsafe(maintenance_user) {
                return Err(AttachmentError::Paused(PauseReason::Maintenance));
            }
            // 墓碑先于最终 unlink；崩溃后只有原路径、隔离路径均缺失才可确认完成。
            conn.execute(
                "UPDATE attachment_files SET reason='unlink_committed' WHERE id=?1",
                [&id],
            )?;
            fs::remove_file(&trash).map_err(io_error)?;
            File::open(self.root.join(kind.directory()))
                .and_then(|dir| dir.sync_all())
                .map_err(io_error)?;
            Ok(())
        })();
        if let Err(error) = work {
            let receipt: Option<String> = conn.query_row(
                "SELECT reason FROM attachment_files WHERE id=?1",
                [&id],
                |r| r.get(0),
            )?;
            if receipt.as_deref() != Some("unlink_committed") || original.exists() || trash.exists()
            {
                conn.execute(
                    "UPDATE attachment_files SET reason=?2 WHERE id=?1",
                    params![id, serde_json::to_string(&error.reason())?],
                )?;
                return Err(error);
            }
            File::open(self.root.join(kind.directory()))
                .and_then(|dir| dir.sync_all())
                .map_err(io_error)?;
        }
        conn.execute(
            "UPDATE attachment_files SET state='removed',reason=NULL WHERE id=?1",
            [id],
        )?;
        self.refresh_deletion_progress(conn)?;
        Ok(true)
    }
}

fn maintenance_present_or_unsafe(user: &inputia_settings::maintenance::UserContext) -> bool {
    !matches!(
        inputia_settings::maintenance::inspect(&user.home, user.uid),
        Ok(None)
    )
}

fn secure_directory(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path).map_err(io_error)?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err(AttachmentError::Paused(PauseReason::UnsafePath));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o022 != 0 {
            return Err(AttachmentError::Paused(PauseReason::UnsafePath));
        }
    }
    Ok(())
}
fn secure_file(meta: &fs::Metadata) -> Result<(u64, u64)> {
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(AttachmentError::Paused(PauseReason::UnsafePath));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.nlink() != 1 || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o022 != 0
        {
            return Err(AttachmentError::Paused(PauseReason::UnsafePath));
        }
        Ok((meta.dev(), meta.ino()))
    }
    #[cfg(not(unix))]
    {
        Err(AttachmentError::Paused(PauseReason::UnsafePath))
    }
}
fn matches_identity(meta: &fs::Metadata, device: u64, inode: u64) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        meta.dev() == device && meta.ino() == inode
    }
    #[cfg(not(unix))]
    {
        let _ = (meta, device, inode);
        false
    }
}
fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        left.dev() == right.dev() && left.ino() == right.ino()
    }
    #[cfg(not(unix))]
    {
        let _ = (left, right);
        false
    }
}
fn check_import_pair(meta: &fs::Metadata) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.is_file() && meta.nlink() == 2 && meta.uid() == unsafe { libc::geteuid() } {
            return Ok(());
        }
    }
    Err(AttachmentError::Paused(PauseReason::UnsafePath))
}
fn available_space(path: &Path) -> Result<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let path = std::ffi::CString::new(path.as_os_str().as_bytes())
            .map_err(|_| AttachmentError::Paused(PauseReason::UnsafePath))?;
        let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
        if unsafe { libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) } != 0 {
            return Err(AttachmentError::Paused(PauseReason::SpaceUnavailable));
        }
        let stat = unsafe { stat.assume_init() };
        // fsblkcnt_t/c_ulong 的宽度在 macOS 和 Linux、32/64 位目标之间不同。
        #[allow(clippy::unnecessary_cast)]
        let bytes = (stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64);
        Ok(bytes)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(AttachmentError::Paused(PauseReason::SpaceUnavailable))
    }
}
fn validate(kind: AttachmentKind, bytes: &[u8]) -> Result<()> {
    let invalid = || AttachmentError::Paused(PauseReason::InvalidFormat);
    match kind {
        AttachmentKind::Recording => {
            let mut reader = hound::WavReader::new(Cursor::new(bytes)).map_err(|_| invalid())?;
            let spec = reader.spec();
            if spec.channels == 0 || spec.sample_rate == 0 || reader.duration() == 0 {
                return Err(invalid());
            }
            match spec.sample_format {
                hound::SampleFormat::Float => {
                    for sample in reader.samples::<f32>() {
                        if !sample.map_err(|_| invalid())?.is_finite() {
                            return Err(invalid());
                        }
                    }
                }
                hound::SampleFormat::Int => {
                    for sample in reader.samples::<i32>() {
                        sample.map_err(|_| invalid())?;
                    }
                }
            };
            Ok(())
        }
        AttachmentKind::Image => {
            let mut decoder = png::Decoder::new(Cursor::new(bytes));
            decoder.set_limits(png::Limits {
                bytes: 256 * 1024 * 1024,
            });
            let mut reader = decoder.read_info().map_err(|_| invalid())?;
            let info = reader.info();
            if info.width == 0
                || info.height == 0
                || u64::from(info.width) * u64::from(info.height) > 64 * 1024 * 1024
            {
                return Err(invalid());
            }
            let mut buffer = vec![0; reader.output_buffer_size()];
            reader.next_frame(&mut buffer).map_err(|_| invalid())?;
            Ok(())
        }
        AttachmentKind::External => Err(invalid()),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::deletion_lifecycle::{AttachmentCleanup, DeleteRequest, DELETE_SCHEMA_VERSION};
    struct Fixture {
        root: tempfile::TempDir,
        conn: Connection,
        store: AttachmentStore,
    }
    fn wav() -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        {
            let mut writer = hound::WavWriter::new(
                &mut bytes,
                hound::WavSpec {
                    channels: 1,
                    sample_rate: 16000,
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                },
            )
            .unwrap();
            writer.write_sample(123i16).unwrap();
            writer.finalize().unwrap();
        }
        bytes.into_inner()
    }
    impl Fixture {
        fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            let conn = Connection::open_in_memory().unwrap();
            AttachmentStore::initialize(&conn).unwrap();
            conn.execute_batch("CREATE TABLE integration_deletion_operations(operation_id TEXT PRIMARY KEY,state TEXT,attachment_cleanup TEXT);").unwrap();
            let store = AttachmentStore::new(root.path()).unwrap();
            Self { root, conn, store }
        }
        fn import(&mut self, operation: &str) -> AttachmentImport {
            let asset = self
                .store
                .import(&mut self.conn, AttachmentKind::Recording, operation, &wav())
                .unwrap();
            self.store.finish_import(&self.conn, operation).unwrap();
            asset
        }
        fn reference(&self, asset: &AttachmentImport, owner: &str) -> AttachmentReference {
            AttachmentReference {
                store_id: "source".into(),
                record_id: owner.into(),
                revision: 1,
                kind: AttachmentKind::Recording,
                path: Some(asset.file_name.clone()),
                retained: false,
            }
        }
        fn delete(&mut self, reference: &AttachmentReference, operation: &str) {
            let request = DeleteRequest {
                schema_version: DELETE_SCHEMA_VERSION,
                operation_id: operation.into(),
                item_id: crate::store::item_id(&reference.store_id, &reference.record_id),
                store_id: reference.store_id.clone(),
                logical_name: "history".into(),
                record_id: reference.record_id.clone(),
                expected_revision: reference.revision,
            };
            self.conn.execute("INSERT INTO integration_deletion_operations VALUES(?1,'projection_revoked','not_started')",[operation]).unwrap();
            let manifest = reference
                .path
                .as_ref()
                .map(|path| {
                    vec![crate::source::DeletedAttachment {
                        kind: reference.kind,
                        path: path.clone(),
                    }]
                })
                .unwrap_or_default();
            self.store
                .record_manifest(&mut self.conn, &request, Some(&manifest), &[])
                .unwrap();
            self.store
                .schedule_deletion(&mut self.conn, &request)
                .unwrap();
        }
        fn gc(&mut self) -> Result<bool> {
            let gate = Arc::new(AttachmentMutationGate::default());
            let guard = gate.try_gc().unwrap();
            self.store.collect_one(
                &mut self.conn,
                &guard,
                &inputia_settings::maintenance::UserContext {
                    home: self.store.root.clone(),
                    uid: unsafe { libc::geteuid() },
                },
            )
        }
    }
    #[test]
    fn shared_owner_and_active_lease_prevent_collection() {
        let mut f = Fixture::new();
        let asset = f.import("import-1");
        let one = f.reference(&asset, "1");
        let two = f.reference(&asset, "2");
        f.store
            .reconcile(&mut f.conn, &[one.clone(), two.clone()])
            .unwrap();
        f.delete(&one, "delete-1");
        assert_eq!(
            f.store.deletion_progress(&f.conn, "delete-1").unwrap().0,
            AttachmentCleanup::Completed
        );
        assert!(!f.gc().unwrap());
        let lease = f
            .store
            .acquire(
                &f.conn,
                Some(&asset.attachment_id),
                PinPurpose::Active,
                "listen",
            )
            .unwrap();
        f.delete(&two, "delete-2");
        assert!(!f.gc().unwrap());
        assert_eq!(f.store.read_leased(&f.conn, &lease).unwrap(), wav());
        f.store.release(&f.conn, &lease).unwrap();
        f.store.release(&f.conn, &lease).unwrap();
        assert!(f.gc().unwrap());
        assert!(!f.gc().unwrap());
        assert!(!f
            .root
            .path()
            .join("recordings")
            .join(asset.file_name)
            .exists());
        assert_eq!(
            f.store.deletion_progress(&f.conn, "delete-2").unwrap().0,
            AttachmentCleanup::Completed
        );
    }
    #[test]
    fn external_reference_is_removed_without_touching_external_file() {
        let mut f = Fixture::new();
        let external = tempfile::NamedTempFile::new().unwrap();
        fs::write(external.path(), b"external data").unwrap();
        let reference = AttachmentReference {
            store_id: "source".into(),
            record_id: "1".into(),
            revision: 1,
            kind: AttachmentKind::Recording,
            path: Some(external.path().to_string_lossy().into_owned()),
            retained: false,
        };
        f.store
            .reconcile(&mut f.conn, std::slice::from_ref(&reference))
            .unwrap();
        f.delete(&reference, "delete-external");
        let _ = f.gc().unwrap();
        assert_eq!(fs::read(external.path()).unwrap(), b"external data");
    }
    #[cfg(unix)]
    #[test]
    fn symlink_hardlink_and_content_replacement_are_not_adopted_or_collected() {
        use std::os::unix::fs::symlink;
        let mut f = Fixture::new();
        let asset = f.import("safe");
        let reference = f.reference(&asset, "1");
        let path = f.root.path().join("recordings").join(&asset.file_name);
        let evil = f.root.path().join("recordings/evil.wav");
        symlink(&path, &evil).unwrap();
        let mut bad = reference.clone();
        bad.path = Some("evil.wav".into());
        assert!(f.store.reconcile(&mut f.conn, &[bad]).is_err());
        fs::remove_file(&evil).unwrap();
        fs::hard_link(&path, &evil).unwrap();
        assert!(f
            .store
            .reconcile(&mut f.conn, std::slice::from_ref(&reference))
            .is_err());
        fs::remove_file(&evil).unwrap();
        f.store
            .reconcile(&mut f.conn, std::slice::from_ref(&reference))
            .unwrap();
        f.delete(&reference, "delete-safe");
        fs::write(&path, b"foreign replacement").unwrap();
        assert!(f.gc().is_err());
        assert_eq!(fs::read(&path).unwrap(), b"foreign replacement");
    }
    #[test]
    fn retained_revision_and_transaction_pins_survive_process_restart() {
        let mut f = Fixture::new();
        let asset = f.import("keep");
        let mut reference = f.reference(&asset, "1");
        reference.retained = true;
        f.store
            .reconcile(&mut f.conn, std::slice::from_ref(&reference))
            .unwrap();
        let export = f
            .store
            .acquire(
                &f.conn,
                Some(&asset.attachment_id),
                PinPurpose::Export,
                "export",
            )
            .unwrap();
        let active = f
            .store
            .acquire(
                &f.conn,
                Some(&asset.attachment_id),
                PinPurpose::Active,
                "play",
            )
            .unwrap();
        f.store = AttachmentStore::new(f.root.path()).unwrap();
        f.store
            .reconcile(&mut f.conn, std::slice::from_ref(&reference))
            .unwrap();
        assert!(f.store.read_leased(&f.conn, &active).is_err());
        assert_eq!(f.store.read_leased(&f.conn, &export).unwrap(), wav());
        f.delete(&reference, "delete");
        assert!(!f.gc().unwrap());
        f.store.release(&f.conn, &export).unwrap();
        assert!(f.gc().unwrap());
    }
    #[test]
    fn import_recovers_staged_publish_and_registry_commit_crashes() {
        for phase in 0..3 {
            let mut f = Fixture::new();
            let bytes = wav();
            let work = f
                .store
                .prepare_import(
                    &f.conn,
                    AttachmentKind::Recording,
                    "crash",
                    &hash(&bytes),
                    bytes.len() as u64,
                )
                .unwrap();
            let path = f.store.path(work.kind, &work.temp).unwrap();
            fs::write(&path, &bytes).unwrap();
            if phase >= 1 {
                fs::hard_link(&path, f.store.path(work.kind, &work.name).unwrap()).unwrap();
            }
            if phase == 2 {
                fs::remove_file(&path).unwrap();
            }
            f.store.recover_import(&mut f.conn, "crash").unwrap();
            let retry = f.import("crash");
            assert_eq!(
                fs::read(
                    f.store
                        .path(AttachmentKind::Recording, &retry.file_name)
                        .unwrap()
                )
                .unwrap(),
                bytes
            );
            assert!(f
                .store
                .import(
                    &mut f.conn,
                    AttachmentKind::Recording,
                    "crash",
                    b"different"
                )
                .is_err());
        }
    }
    #[test]
    fn source_reference_commit_is_discovered_and_orphan_is_scheduled_after_restart() {
        let mut f = Fixture::new();
        let asset = f.import("survives");
        let reference = f.reference(&asset, "1");
        f.store = AttachmentStore::new(f.root.path()).unwrap();
        f.store.reconcile(&mut f.conn, &[reference]).unwrap();
        assert!(!f.gc().unwrap());
        let mut orphan = Fixture::new();
        let asset = orphan.import("orphan");
        orphan.store = AttachmentStore::new(orphan.root.path()).unwrap();
        orphan.store.reconcile(&mut orphan.conn, &[]).unwrap();
        assert!(orphan.gc().unwrap());
        assert!(!orphan
            .root
            .path()
            .join("recordings")
            .join(asset.file_name)
            .exists());
    }
    #[test]
    fn gc_recovers_quarantine_and_post_unlink_commit_crashes() {
        for phase in 0..3 {
            let mut f = Fixture::new();
            let asset = f.import("record");
            let reference = f.reference(&asset, "1");
            f.store
                .reconcile(&mut f.conn, std::slice::from_ref(&reference))
                .unwrap();
            f.delete(&reference, "delete");
            let original = f
                .store
                .path(AttachmentKind::Recording, &asset.file_name)
                .unwrap();
            let trash = f
                .store
                .path(AttachmentKind::Recording, ".inputia-gc-test")
                .unwrap();
            f.conn
                .execute(
                    "UPDATE attachment_files SET state='quarantined',quarantine='.inputia-gc-test'",
                    [],
                )
                .unwrap();
            fs::hard_link(&original, &trash).unwrap();
            if phase >= 1 {
                fs::remove_file(&original).unwrap();
            }
            if phase == 2 {
                f.conn
                    .execute("UPDATE attachment_files SET reason='unlink_committed'", [])
                    .unwrap();
                fs::remove_file(&trash).unwrap();
            }
            assert!(f.gc().unwrap());
            assert!(!f.gc().unwrap());
            assert_eq!(
                f.store.deletion_progress(&f.conn, "delete").unwrap().0,
                AttachmentCleanup::Completed
            );
        }
    }
    #[test]
    fn missing_file_without_unlink_receipt_does_not_claim_complete() {
        let mut f = Fixture::new();
        let asset = f.import("record");
        let reference = f.reference(&asset, "1");
        f.store
            .reconcile(&mut f.conn, std::slice::from_ref(&reference))
            .unwrap();
        f.delete(&reference, "delete");
        fs::remove_file(
            f.store
                .path(AttachmentKind::Recording, &asset.file_name)
                .unwrap(),
        )
        .unwrap();
        assert!(f.gc().is_err());
        assert_eq!(
            f.store.deletion_progress(&f.conn, "delete").unwrap().0,
            AttachmentCleanup::Scheduled
        );
    }
    #[test]
    fn maintenance_and_global_update_pin_stop_physical_gc() {
        let mut f = Fixture::new();
        let asset = f.import("record");
        let reference = f.reference(&asset, "1");
        f.store
            .reconcile(&mut f.conn, std::slice::from_ref(&reference))
            .unwrap();
        f.delete(&reference, "delete");
        let pin = f
            .store
            .acquire(&f.conn, None, PinPurpose::Update, "update")
            .unwrap();
        assert!(matches!(
            f.gc(),
            Err(AttachmentError::Paused(PauseReason::BusyPin))
        ));
        f.store.release(&f.conn, &pin).unwrap();
        let marker = inputia_settings::maintenance::marker_path(&f.store.root);
        fs::create_dir_all(marker.parent().unwrap()).unwrap();
        fs::write(&marker, b"invalid marker still pauses").unwrap();
        let gate = Arc::new(AttachmentMutationGate::default());
        assert!(matches!(
            f.store.collect_one(
                &mut f.conn,
                &gate.try_gc().unwrap(),
                &inputia_settings::maintenance::UserContext {
                    home: f.store.root.clone(),
                    uid: unsafe { libc::geteuid() }
                }
            ),
            Err(AttachmentError::Paused(PauseReason::Maintenance))
        ));
        assert!(f
            .store
            .path(AttachmentKind::Recording, &asset.file_name)
            .unwrap()
            .exists());
    }
    #[test]
    fn capacity_free_space_and_invalid_format_fail_with_typed_reason() {
        let mut f = Fixture::new();
        f.store
            .configure(
                &f.conn,
                &AttachmentBudget {
                    capacity_bytes: 100,
                    max_file_bytes: 100,
                    minimum_free_bytes: 50,
                },
            )
            .unwrap();
        assert!(matches!(
            f.store.preflight(&f.conn, 60, 100),
            Err(AttachmentError::Paused(PauseReason::InsufficientSpace))
        ));
        assert!(matches!(
            f.store.preflight(&f.conn, 101, 1000),
            Err(AttachmentError::Paused(PauseReason::FileTooLarge))
        ));
        let _ = f.import("one");
        assert!(matches!(
            f.store.preflight(&f.conn, 60, 1000),
            Err(AttachmentError::Paused(PauseReason::Capacity))
        ));
        assert!(matches!(
            validate(AttachmentKind::Recording, b"invalid"),
            Err(AttachmentError::Paused(PauseReason::InvalidFormat))
        ));
        assert_eq!(
            io_error(std::io::Error::from_raw_os_error(28)).reason(),
            PauseReason::InsufficientSpace
        );
    }

    #[test]
    fn impossible_minimum_free_budget_is_rejected_before_persisting() {
        let f = Fixture::new();
        let result = f.store.configure(
            &f.conn,
            &AttachmentBudget {
                capacity_bytes: 100,
                max_file_bytes: 50,
                minimum_free_bytes: 101,
            },
        );
        assert!(matches!(
            result,
            Err(AttachmentError::Invalid("storage budget"))
        ));
    }
    #[test]
    fn cancelled_acquire_cannot_late_create_pin_and_release_identity_is_bound() {
        let mut f = Fixture::new();
        let asset = f.import("record");
        f.store.cancel_acquire(&f.conn, "late").unwrap();
        assert!(f
            .store
            .acquire(
                &f.conn,
                Some(&asset.attachment_id),
                PinPurpose::Active,
                "late"
            )
            .is_err());
        let lease = f
            .store
            .acquire(
                &f.conn,
                Some(&asset.attachment_id),
                PinPurpose::Active,
                "active",
            )
            .unwrap();
        let mut forged = lease.clone();
        forged.instance_id = "other".into();
        assert!(f.store.release(&f.conn, &forged).is_err());
        f.store.cancel_acquire(&f.conn, "active").unwrap();
        assert!(f.store.read_leased(&f.conn, &lease).is_err());
        f.store.release(&f.conn, &lease).unwrap();
    }
    #[test]
    fn exclusive_gc_cannot_overlap_new_or_existing_source_writer() {
        let gate = Arc::new(AttachmentMutationGate::default());
        let writer = gate.enter();
        assert!(gate.try_gc().is_none());
        let nested = gate.enter();
        drop(nested);
        drop(writer);
        let gc = gate.try_gc().unwrap();
        let other = gate.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let task = std::thread::spawn(move || {
            let _writer = other.enter();
            sender.send(()).unwrap();
        });
        assert!(receiver
            .recv_timeout(std::time::Duration::from_millis(20))
            .is_err());
        drop(gc);
        receiver
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        task.join().unwrap();
    }
    #[test]
    fn same_content_after_collection_gets_new_physical_generation() {
        let mut f = Fixture::new();
        let first = f.import("first");
        let reference = f.reference(&first, "1");
        f.store
            .reconcile(&mut f.conn, std::slice::from_ref(&reference))
            .unwrap();
        f.delete(&reference, "delete");
        assert!(f.gc().unwrap());
        let second = f.import("second");
        assert_eq!(first.file_name, second.file_name);
        assert_ne!(first.attachment_id, second.attachment_id);
        assert!(f
            .store
            .acquire(
                &f.conn,
                Some(&first.attachment_id),
                PinPurpose::Active,
                "old"
            )
            .is_err());
        assert!(f
            .store
            .acquire(
                &f.conn,
                Some(&second.attachment_id),
                PinPurpose::Active,
                "new"
            )
            .is_ok());
    }
    #[test]
    fn empty_old_reservation_refunds_capacity_but_corrupt_stage_stays_visible() {
        let mut f = Fixture::new();
        let bytes = wav();
        let empty = f
            .store
            .prepare_import(
                &f.conn,
                AttachmentKind::Recording,
                "empty",
                &hash(&bytes),
                bytes.len() as u64,
            )
            .unwrap();
        assert!(f.store.health(&f.conn).unwrap().used_bytes > 0);
        f.store = AttachmentStore::new(f.root.path()).unwrap();
        f.store.recover_imports(&mut f.conn).unwrap();
        f.store.reconcile(&mut f.conn, &[]).unwrap();
        assert_eq!(f.store.health(&f.conn).unwrap().used_bytes, 0);
        assert!(!f.store.path(empty.kind, &empty.temp).unwrap().exists());
        let corrupt = f
            .store
            .prepare_import(
                &f.conn,
                AttachmentKind::Recording,
                "corrupt",
                &hash(&bytes),
                bytes.len() as u64,
            )
            .unwrap();
        fs::write(
            f.store.path(corrupt.kind, &corrupt.temp).unwrap(),
            b"partial",
        )
        .unwrap();
        f.store = AttachmentStore::new(f.root.path()).unwrap();
        f.store.recover_imports(&mut f.conn).unwrap();
        f.store.reconcile(&mut f.conn, &[]).unwrap();
        assert_eq!(f.store.health(&f.conn).unwrap().blocked_imports, 1);
        assert_eq!(
            f.store.health(&f.conn).unwrap().used_bytes,
            bytes.len() as u64
        );
    }
    #[test]
    fn missing_retained_manifest_is_blocked_and_survives_projection_loss() {
        let mut f = Fixture::new();
        let asset = f.import("current");
        let current = f.reference(&asset, "1");
        let mut old = current.clone();
        old.path = Some("missing-old.wav".into());
        old.retained = true;
        let request = DeleteRequest {
            schema_version: DELETE_SCHEMA_VERSION,
            operation_id: "delete".into(),
            item_id: crate::store::item_id("source", "1"),
            store_id: "source".into(),
            logical_name: "history".into(),
            record_id: "1".into(),
            expected_revision: 1,
        };
        let manifest = vec![crate::source::DeletedAttachment {
            kind: AttachmentKind::Recording,
            path: asset.file_name.clone(),
        }];
        f.store
            .record_manifest(&mut f.conn, &request, Some(&manifest), &[old])
            .unwrap();
        f.store.schedule_deletion(&mut f.conn, &request).unwrap();
        assert_eq!(
            f.store.deletion_progress(&f.conn, "delete").unwrap().0,
            AttachmentCleanup::Blocked
        );
        f.store
            .record_manifest(&mut f.conn, &request, Some(&manifest), &[])
            .unwrap();
        assert_eq!(
            f.store.deletion_progress(&f.conn, "delete").unwrap().0,
            AttachmentCleanup::Blocked
        );
        let raw: String = f
            .conn
            .query_row("SELECT manifest_json FROM attachment_deletions", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(raw.contains("missing-old.wav"));
    }
    #[test]
    fn altered_cleanup_ids_or_phase_cannot_self_certify_completion() {
        for mutation in [
            "UPDATE attachment_deletions SET ids='[]'",
            "UPDATE attachment_deletions SET state='completed'",
        ] {
            let mut f = Fixture::new();
            let asset = f.import("asset");
            let reference = f.reference(&asset, "1");
            f.store
                .reconcile(&mut f.conn, std::slice::from_ref(&reference))
                .unwrap();
            f.delete(&reference, "delete");
            f.conn.execute_batch(mutation).unwrap();
            let request = DeleteRequest {
                schema_version: DELETE_SCHEMA_VERSION,
                operation_id: "delete".into(),
                item_id: crate::store::item_id("source", "1"),
                store_id: "source".into(),
                logical_name: "history".into(),
                record_id: "1".into(),
                expected_revision: 1,
            };
            let manifest = vec![crate::source::DeletedAttachment {
                kind: AttachmentKind::Recording,
                path: asset.file_name.clone(),
            }];
            f.store
                .record_manifest(&mut f.conn, &request, Some(&manifest), &[])
                .unwrap();
            f.store.refresh_deletion_progress(&f.conn).unwrap();
            assert_eq!(
                f.store.deletion_progress(&f.conn, "delete").unwrap().0,
                AttachmentCleanup::Blocked
            );
            assert!(!f.gc().unwrap());
            assert!(f
                .store
                .path(AttachmentKind::Recording, &asset.file_name)
                .unwrap()
                .exists());
        }
    }
    #[test]
    fn failed_gc_candidate_does_not_starve_later_healthy_file() {
        let mut f = Fixture::new();
        let one = f.import("one");
        let reference = f.reference(&one, "1");
        f.store
            .reconcile(&mut f.conn, std::slice::from_ref(&reference))
            .unwrap();
        f.delete(&reference, "delete-one");
        fs::write(
            f.store
                .path(AttachmentKind::Recording, &one.file_name)
                .unwrap(),
            b"corrupt",
        )
        .unwrap();
        assert!(f.gc().is_err());
        let mut bytes = wav();
        let last = bytes.len() - 1;
        bytes[last] = 1;
        let two = f
            .store
            .import(&mut f.conn, AttachmentKind::Recording, "two", &bytes)
            .unwrap();
        f.store.finish_import(&f.conn, "two").unwrap();
        let reference = f.reference(&two, "2");
        f.store
            .reconcile_owner(&mut f.conn, std::slice::from_ref(&reference))
            .unwrap();
        f.delete(&reference, "delete-two");
        let mut progressed = false;
        for _ in 0..2 {
            progressed |= f.gc().unwrap_or(false);
        }
        assert!(progressed);
        assert!(!f
            .store
            .path(AttachmentKind::Recording, &two.file_name)
            .unwrap()
            .exists());
        assert!(f
            .store
            .path(AttachmentKind::Recording, &one.file_name)
            .unwrap()
            .exists());
    }
    #[test]
    fn concurrent_equal_imports_share_generation_but_keep_each_pending_pin() {
        let mut f = Fixture::new();
        let bytes = wav();
        let a = f
            .store
            .prepare_import(
                &f.conn,
                AttachmentKind::Recording,
                "a",
                &hash(&bytes),
                bytes.len() as u64,
            )
            .unwrap();
        let b = f
            .store
            .prepare_import(
                &f.conn,
                AttachmentKind::Recording,
                "b",
                &hash(&bytes),
                bytes.len() as u64,
            )
            .unwrap();
        let published = f.store.publish_import(a, &bytes).unwrap();
        let a = f.store.commit_import(&mut f.conn, published).unwrap();
        let published = f.store.publish_import(b, &bytes).unwrap();
        let b = f.store.commit_import(&mut f.conn, published).unwrap();
        assert_eq!(a.attachment_id, b.attachment_id);
        let reference = f.reference(&a, "1");
        f.store.finish_import(&f.conn, "a").unwrap();
        f.store
            .reconcile(&mut f.conn, std::slice::from_ref(&reference))
            .unwrap();
        f.delete(&reference, "delete");
        assert!(!f.gc().unwrap());
        let still_pending: String = f
            .conn
            .query_row(
                "SELECT state FROM attachment_imports WHERE operation_id='b'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(still_pending, "published");
        f.store.finish_import(&f.conn, "b").unwrap();
        f.store.reconcile(&mut f.conn, &[]).unwrap();
        assert!(f.gc().unwrap());
    }
    #[test]
    fn self_consistent_empty_ids_and_completed_phase_still_need_final_file_evidence() {
        let mut f = Fixture::new();
        let asset = f.import("asset");
        let reference = f.reference(&asset, "1");
        f.store
            .reconcile(&mut f.conn, std::slice::from_ref(&reference))
            .unwrap();
        f.delete(&reference, "delete");
        let raw: String = f
            .conn
            .query_row("SELECT manifest_json FROM attachment_deletions", [], |r| {
                r.get(0)
            })
            .unwrap();
        let mut forged: serde_json::Value = serde_json::from_str(&raw).unwrap();
        forged["ids"] = serde_json::json!([]);
        let raw = serde_json::to_string(&forged).unwrap();
        f.conn.execute("UPDATE attachment_deletions SET ids='[]',state='completed',manifest_json=?1,manifest_digest=?2",params![raw,hash(raw.as_bytes())]).unwrap();
        let request = DeleteRequest {
            schema_version: DELETE_SCHEMA_VERSION,
            operation_id: "delete".into(),
            item_id: crate::store::item_id("source", "1"),
            store_id: "source".into(),
            logical_name: "history".into(),
            record_id: "1".into(),
            expected_revision: 1,
        };
        let manifest = vec![crate::source::DeletedAttachment {
            kind: AttachmentKind::Recording,
            path: asset.file_name.clone(),
        }];
        f.store
            .record_manifest(&mut f.conn, &request, Some(&manifest), &[])
            .unwrap();
        assert_eq!(
            f.store.deletion_progress(&f.conn, "delete").unwrap().0,
            AttachmentCleanup::Blocked
        );
    }
}
