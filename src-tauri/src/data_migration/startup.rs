//! 启动备份的写入许可与恢复日志。Prepared/Recovered 均不授权覆盖源数据。
use super::*;
use inputia_settings::store::InitializationIntent;
use std::collections::BTreeSet;
use tauri::Manager;

const MIGRATION_ID: &str = "control-settings-document-v2";
const JOURNAL: &str = "active-startup.json";
const JOURNAL_LIMIT: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupFailureKind {
    CleanPreflight,
    LegacyRecoveryRequired,
    RepairRequired,
    PendingRecovery,
}

#[derive(Debug)]
pub struct StartupFailure {
    pub kind: StartupFailureKind,
}
impl std::fmt::Display for StartupFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "startup migration {:?}", self.kind)
    }
}
impl std::error::Error for StartupFailure {}
impl StartupFailure {
    /// 只有明确没有待恢复写入时，恢复 UI 才能允许用户修配置。
    pub fn allows_configuration_repair(&self) -> bool {
        self.kind == StartupFailureKind::CleanPreflight
    }
}
fn classified(kind: StartupFailureKind, error: anyhow::Error) -> anyhow::Error {
    // 下层 context 供本地诊断；UI 只使用固定分类，不渲染路径/设置正文。
    error.context(StartupFailure { kind })
}
fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn hash_valid(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Prepared,
    Mutating,
    Restoring,
    Recovered,
    Completed,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
enum Original {
    Absent,
    Present {
        sha256: String,
        size: u64,
        identity: FileIdentity,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileIdentity {
    device: u64,
    inode: u64,
    modified_ns: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Pair {
    root_label: String,
    domain: String,
    document_name: String,
    marker_name: String,
    document: Original,
    marker: Original,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Creation {
    domain: String,
    file_name: String,
    marker_name: String,
    store_id: String,
    original_document_sha256: Option<String>,
    document_sha256: String,
    marker_sha256: String,
    will_write_document: bool,
    will_create_marker: bool,
}
impl From<&InitializationIntent> for Creation {
    fn from(value: &InitializationIntent) -> Self {
        Self {
            domain: value.domain.clone(),
            file_name: value.file_name.clone(),
            marker_name: value.marker_name.clone(),
            store_id: value.store_id.clone(),
            original_document_sha256: value.original_document_sha256.clone(),
            document_sha256: value.document_sha256.clone(),
            marker_sha256: value.marker_sha256.clone(),
            will_write_document: value.will_write_document,
            will_create_marker: value.will_create_marker,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema_version: u32,
    migration_id: String,
    attempt_id: String,
    roots_sha256: String,
    manifest_relative: PathBuf,
    manifest_sha256: String,
    phase: Phase,
    pairs: Vec<Pair>,
    creations: Vec<Creation>,
}

struct StartupLock {
    #[cfg(unix)]
    file: fs::File,
    #[cfg(not(unix))]
    _legacy: MigrationLock,
}
impl StartupLock {
    fn acquire(root: &Path) -> Result<Self> {
        fs::create_dir_all(root)?;
        #[cfg(unix)]
        {
            use std::os::{
                fd::AsRawFd,
                unix::fs::{MetadataExt, OpenOptionsExt},
            };
            let path = checked_path(root, Path::new("control-settings-startup.lock"))?;
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&path)?;
            let metadata = file.metadata()?;
            anyhow::ensure!(
                metadata.is_file()
                    && metadata.nlink() == 1
                    && metadata.uid() == unsafe { libc::geteuid() },
                "unsafe startup lock"
            );
            anyhow::ensure!(
                unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
                "startup migration lock busy"
            );
            Ok(Self { file })
        }
        #[cfg(not(unix))]
        {
            Ok(Self {
                _legacy: MigrationLock::acquire(root, MIGRATION_ID)?,
            })
        }
    }
}
impl Drop for StartupLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            // 显式解锁，fork/exec 继承不应延长生命周期；固定锁 inode 永不 unlink。
            unsafe {
                libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
            }
        }
    }
}

pub struct StartupMigration {
    _lock: Option<StartupLock>,
    pub(super) outcome: MigrationBackupOutcome,
    source_roots: Vec<MigrationSourceRoot>,
    backup_root: PathBuf,
    journal: Journal,
}
impl StartupMigration {
    /// 在打开设置协调器或任何业务 manager 前调用。失败不授权源写入。
    pub fn begin_mutations(&mut self) -> Result<()> {
        self.begin_inner()
            .map_err(|e| classified(StartupFailureKind::PendingRecovery, e))
    }
    fn begin_inner(&mut self) -> Result<()> {
        anyhow::ensure!(
            self.journal.phase == Phase::Prepared,
            "startup is not prepared"
        );
        self.revalidate()?;
        verify_backup(&self.outcome)?;
        for pair in &self.journal.pairs {
            let root = self.root(&pair.root_label)?;
            anyhow::ensure!(
                observe(&checked_path(root, Path::new(&pair.document_name))?)? == pair.document
                    && observe(&checked_path(root, Path::new(&pair.marker_name))?)? == pair.marker,
                "settings changed after startup backup"
            );
        }
        self.journal.phase = Phase::Mutating;
        self.persist()?;
        fault("armed")
    }
    /// 同一设置锁内的写前回调。仅接受固定 pair 与备份原始摘要，不接收设置正文。
    pub fn record_settings_initialization(&mut self, intent: &InitializationIntent) -> Result<()> {
        self.record_inner(intent)
            .map_err(|e| classified(StartupFailureKind::PendingRecovery, e))
    }
    fn record_inner(&mut self, intent: &InitializationIntent) -> Result<()> {
        anyhow::ensure!(
            self.journal.phase == Phase::Mutating,
            "initialization is not armed"
        );
        let creation = Creation::from(intent);
        validate_creation(&self.journal, &creation)?;
        let pair = self
            .journal
            .pairs
            .iter()
            .find(|p| p.domain == creation.domain)
            .context("unknown settings domain")?;
        let root = self.root(&pair.root_label)?;
        anyhow::ensure!(
            observe(&checked_path(root, Path::new(&pair.document_name))?)? == pair.document
                && observe(&checked_path(root, Path::new(&pair.marker_name))?)? == pair.marker,
            "settings changed before initialization authorization"
        );
        if let Some(existing) = self
            .journal
            .creations
            .iter()
            .find(|v| v.domain == creation.domain)
        {
            anyhow::ensure!(
                existing == &creation,
                "initialization identity already reserved"
            );
        } else {
            self.journal.creations.push(creation);
        }
        self.persist()?;
        fault("creation_authorized")
    }
    /// 仅全部 manager 成功初始化后调用；失败时仍由下一启动处理耐久日志。
    pub fn complete(&mut self) -> Result<()> {
        (|| {
            anyhow::ensure!(
                self.journal.phase == Phase::Mutating,
                "startup never authorized mutation"
            );
            self.revalidate()?;
            self.journal.phase = Phase::Completed;
            self.persist()?;
            fault("completed")
        })()
        .map_err(|e| classified(StartupFailureKind::PendingRecovery, e))
    }
    fn root(&self, label: &str) -> Result<&Path> {
        Ok(&self
            .source_roots
            .iter()
            .find(|r| r.label == label)
            .context("source root unavailable")?
            .root)
    }
    fn persist(&self) -> Result<()> {
        write_json_atomically(&self.backup_root.join(JOURNAL), &self.journal)
    }
    fn revalidate(&self) -> Result<()> {
        validate_journal(&self.journal, &self.backup_root, &self.source_roots).map(|_| ())
    }
    #[cfg(test)]
    pub(super) fn backup_dir(&self) -> &Path {
        &self.outcome.backup_dir
    }
}
impl Drop for StartupMigration {
    fn drop(&mut self) {
        if matches!(self.journal.phase, Phase::Mutating | Phase::Restoring) {
            log::warn!("Startup requires recovery before settings or managers may open");
        }
    }
}

/// 兼容旧调用入口；新的启动编排必须使用带纯预检闭包的入口。
pub fn prepare_startup_backup(app: &tauri::AppHandle) -> Result<Option<StartupMigration>> {
    prepare_startup_backup_with_preflight(app, || Ok(()))
}
pub fn prepare_startup_backup_with_preflight<F>(
    app: &tauri::AppHandle,
    preflight: F,
) -> Result<Option<StartupMigration>>
where
    F: FnOnce() -> Result<()>,
{
    let handy_root = crate::portable::app_data_dir(app)?;
    let migration_root = handy_root.join("migration_backups");
    let legacy = migration_root.join(STARTUP_MIGRATION_ID);
    #[cfg(target_os = "macos")]
    let inputia_root = crate::candidate_profile::current()
        .map(|p| p.inputia_root.clone())
        .or_else(|| {
            app.path()
                .home_dir()
                .ok()
                .map(|h| h.join("Library/Application Support/Inputia"))
        });
    #[cfg(not(target_os = "macos"))]
    let inputia_root: Option<PathBuf> = None;
    prepare_paths(
        &handy_root,
        inputia_root.as_deref(),
        &migration_root.join(MIGRATION_ID),
        &handy_root.join("migration_locks"),
        Some(&legacy),
        preflight,
    )
}
#[cfg(test)]
pub(super) fn prepare_startup_backup_for_paths(
    handy: &Path,
    inputia: Option<&Path>,
    backup: &Path,
    lock: &Path,
) -> Result<Option<StartupMigration>> {
    prepare_paths(handy, inputia, backup, lock, None, || Ok(()))
}
fn prepare_paths<F>(
    handy: &Path,
    inputia: Option<&Path>,
    backup: &Path,
    lock: &Path,
    legacy: Option<&Path>,
    preflight: F,
) -> Result<Option<StartupMigration>>
where
    F: FnOnce() -> Result<()>,
{
    let lock = StartupLock::acquire(lock)
        .map_err(|e| classified(StartupFailureKind::PendingRecovery, e))?;
    let mut roots = vec![MigrationSourceRoot {
        label: "handy".into(),
        root: handy.into(),
    }];
    if let Some(root) = inputia {
        roots.push(MigrationSourceRoot {
            label: "inputia".into(),
            root: root.into(),
        });
    }
    let journal_path = checked_path(backup, Path::new(JOURNAL))
        .map_err(|e| classified(StartupFailureKind::RepairRequired, e))?;
    let journal = match fs::symlink_metadata(&journal_path) {
        Ok(_) => Some(
            read_json::<Journal>(&journal_path, JOURNAL_LIMIT)
                .map_err(|e| classified(StartupFailureKind::RepairRequired, e))?,
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(classified(StartupFailureKind::RepairRequired, e.into())),
    };
    if let Some(ref journal) = journal {
        let outcome = validate_journal(journal, backup, &roots)
            .map_err(|e| classified(StartupFailureKind::RepairRequired, e))?;
        let mut guard = StartupMigration {
            _lock: Some(lock),
            outcome,
            source_roots: roots,
            backup_root: backup.into(),
            journal: journal.clone(),
        };
        if matches!(guard.journal.phase, Phase::Mutating | Phase::Restoring) {
            recover(&mut guard).map_err(|e| classified(StartupFailureKind::PendingRecovery, e))?;
        }
        // 读取到 terminal 记录也须重做同步，不能把上次目录 fsync 失败误作已确认耐久。
        fs::File::open(&journal_path)?.sync_all()?;
        sync_dir(backup)?;
        preflight().map_err(|e| classified(StartupFailureKind::CleanPreflight, e))?;
        if guard.journal.phase == Phase::Completed {
            return Ok(None);
        }
        return new_attempt(
            guard._lock.take().context("startup lock missing")?,
            guard.source_roots.clone(),
            backup,
        );
    }
    if has_backup(backup)? {
        return Err(StartupFailure {
            kind: StartupFailureKind::LegacyRecoveryRequired,
        }
        .into());
    }
    if let Some(legacy) = legacy {
        if has_backup(legacy)? {
            let marker = legacy.join("complete.json");
            verify_startup_marker(&marker, &roots)
                .map_err(|e| classified(StartupFailureKind::LegacyRecoveryRequired, e))?;
        }
    }
    preflight().map_err(|e| classified(StartupFailureKind::CleanPreflight, e))?;
    new_attempt(lock, roots, backup)
}
fn new_attempt(
    lock: StartupLock,
    roots: Vec<MigrationSourceRoot>,
    backup: &Path,
) -> Result<Option<StartupMigration>> {
    (|| {
        let pairs = observe_pairs(&roots)?;
        let candidates = roots
            .iter()
            .flat_map(|root| {
                if root.label == "handy" {
                    handy_data_candidates("handy")
                } else {
                    inputia_data_candidates("inputia")
                }
            })
            .collect::<Vec<_>>();
        fault("before_backup")?;
        create_dirs_durable(backup)?;
        let outcome = prepare_backup_for_migration(&roots, &candidates, backup, MIGRATION_ID)?;
        // 此后 manifest 永不修改，恢复结果在独立日志内。
        durable_tree(&outcome.backup_dir)?;
        sync_dir(backup)?;
        fault("backup_created")?;
        anyhow::ensure!(
            observe_pairs(&roots)? == pairs,
            "settings changed during backup"
        );
        let journal = Journal {
            schema_version: 2,
            migration_id: MIGRATION_ID.into(),
            attempt_id: uuid::Uuid::new_v4().to_string(),
            roots_sha256: roots_digest(&roots)?,
            manifest_relative: outcome.manifest_path.strip_prefix(backup)?.into(),
            manifest_sha256: file_fingerprint(&outcome.manifest_path)?.1,
            phase: Phase::Prepared,
            pairs,
            creations: vec![],
        };
        validate_journal(&journal, backup, &roots)?;
        let guard = StartupMigration {
            _lock: Some(lock),
            outcome,
            source_roots: roots,
            backup_root: backup.into(),
            journal,
        };
        guard.persist()?;
        fault("prepared")?;
        Ok(Some(guard))
    })()
    .map_err(|e| classified(StartupFailureKind::RepairRequired, e))
}
fn has_backup(root: &Path) -> Result<bool> {
    let entries = match fs::read_dir(root) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    for entry in entries {
        if entry?
            .file_name()
            .to_string_lossy()
            .starts_with("handy-data-")
        {
            return Ok(true);
        }
    }
    Ok(false)
}
fn roots_digest(roots: &[MigrationSourceRoot]) -> Result<String> {
    Ok(sha(&serde_json::to_vec(
        &roots
            .iter()
            .map(|r| (&r.label, &r.root))
            .collect::<Vec<_>>(),
    )?))
}
fn pair_spec(label: &str) -> Result<(&'static str, &'static str, &'static str)> {
    match label {
        "handy" => Ok((
            "inputia.control-settings",
            "settings_store.json",
            ".inputia-control-settings-initialized.json",
        )),
        "inputia" => Ok((
            "inputia.basic-input",
            "settings.json",
            ".inputia-settings-initialized.json",
        )),
        _ => anyhow::bail!("unknown startup root"),
    }
}
fn observe_pairs(roots: &[MigrationSourceRoot]) -> Result<Vec<Pair>> {
    roots
        .iter()
        .map(|root| {
            let (domain, document, marker) = pair_spec(&root.label)?;
            Ok(Pair {
                root_label: root.label.clone(),
                domain: domain.into(),
                document_name: document.into(),
                marker_name: marker.into(),
                document: observe(&checked_path(&root.root, Path::new(document))?)?,
                marker: observe(&checked_path(&root.root, Path::new(marker))?)?,
            })
        })
        .collect()
}
fn metadata_identity(m: &fs::Metadata) -> Result<FileIdentity> {
    #[cfg(unix)]
    let (device, inode) = {
        use std::os::unix::fs::MetadataExt;
        (m.dev(), m.ino())
    };
    #[cfg(not(unix))]
    let (device, inode) = (0, 0);
    Ok(FileIdentity {
        device,
        inode,
        modified_ns: m
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
            .try_into()
            .context("file timestamp out of range")?,
    })
}
fn open_read(path: &Path) -> Result<fs::File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    anyhow::ensure!(metadata.is_file(), "startup data is not regular file");
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        anyhow::ensure!(metadata.nlink() == 1, "startup file is hard-linked");
    }
    Ok(file)
}
fn observe(path: &Path) -> Result<Original> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Original::Absent),
        Err(e) => return Err(e.into()),
        Ok(_) => {}
    }
    let mut file = open_read(path)?;
    let metadata = file.metadata()?;
    let identity = metadata_identity(&metadata)?;
    let mut hasher = Sha256::new();
    let read = std::io::copy(
        &mut (&mut file).take(metadata.len().saturating_add(1)),
        &mut hasher,
    )?;
    anyhow::ensure!(
        read == metadata.len()
            && metadata_identity(&file.metadata()?)? == identity
            && metadata_identity(&fs::symlink_metadata(path)?)? == identity,
        "startup file changed while reading"
    );
    Ok(Original::Present {
        sha256: format!("{:x}", hasher.finalize()),
        size: read,
        identity,
    })
}
fn read_bytes(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let mut file = open_read(path)?;
    let metadata = file.metadata()?;
    let identity = metadata_identity(&metadata)?;
    anyhow::ensure!(metadata.len() <= limit, "startup JSON exceeds bound");
    let mut bytes = vec![];
    (&mut file).take(limit + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 == metadata.len()
            && metadata_identity(&file.metadata()?)? == identity
            && metadata_identity(&fs::symlink_metadata(path)?)? == identity,
        "startup JSON changed while reading"
    );
    Ok(bytes)
}
fn read_json<T: serde::de::DeserializeOwned>(path: &Path, limit: u64) -> Result<T> {
    Ok(serde_json::from_slice(&read_bytes(path, limit)?)?)
}

fn validate_journal(
    j: &Journal,
    backup: &Path,
    roots: &[MigrationSourceRoot],
) -> Result<MigrationBackupOutcome> {
    anyhow::ensure!(
        j.schema_version == 2
            && j.migration_id == MIGRATION_ID
            && uuid::Uuid::parse_str(&j.attempt_id).is_ok()
            && j.roots_sha256 == roots_digest(roots)?
            && hash_valid(&j.manifest_sha256),
        "startup journal identity mismatch"
    );
    validate_relative(&j.manifest_relative)?;
    anyhow::ensure!(
        j.manifest_relative.components().count() == 2
            && j.manifest_relative.file_name() == Some(std::ffi::OsStr::new("manifest.json")),
        "unexpected startup manifest layout"
    );
    let path = checked_path(backup, &j.manifest_relative)?;
    let bytes = read_bytes(&path, 16 * 1024 * 1024)?;
    anyhow::ensure!(sha(&bytes) == j.manifest_sha256, "startup manifest changed");
    let manifest: MigrationBackupManifest = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(
        manifest.migration_id == MIGRATION_ID
            && manifest.status == MigrationBackupStatus::Verified
            && manifest.sqlite_snapshot_format == Some(SqliteSnapshotFormat::VacuumIntoV1),
        "startup manifest not verified immutable snapshot"
    );
    let outcome = MigrationBackupOutcome {
        backup_dir: path.parent().context("missing manifest parent")?.into(),
        manifest_path: path,
        manifest,
    };
    validate_outcome_paths(&outcome, Some(roots))?;
    anyhow::ensure!(j.pairs.len() == roots.len(), "settings pair count mismatch");
    let mut labels = BTreeSet::new();
    for pair in &j.pairs {
        let (domain, document, marker) = pair_spec(&pair.root_label)?;
        anyhow::ensure!(
            labels.insert(&pair.root_label)
                && roots.iter().any(|r| r.label == pair.root_label)
                && pair.domain == domain
                && pair.document_name == document
                && pair.marker_name == marker,
            "settings pair binding mismatch"
        );
        for (name, original) in [
            (&pair.document_name, &pair.document),
            (&pair.marker_name, &pair.marker),
        ] {
            let entries = outcome
                .manifest
                .entries
                .iter()
                .filter(|e| {
                    e.source_root_label == pair.root_label
                        && e.source_relative_path == Path::new(name)
                })
                .collect::<Vec<_>>();
            match original {
                Original::Absent => {
                    anyhow::ensure!(entries.is_empty(), "absent setting has backup entry")
                }
                Original::Present { sha256, size, .. } => anyhow::ensure!(
                    entries.len() == 1
                        && entries[0].sha256 == *sha256
                        && entries[0].byte_len == *size
                        && entries[0].kind == MigrationPathKind::File,
                    "settings baseline differs from manifest"
                ),
            }
        }
    }
    anyhow::ensure!(
        j.creations.len() <= j.pairs.len(),
        "too many settings creation records"
    );
    let mut domains = BTreeSet::new();
    for creation in &j.creations {
        anyhow::ensure!(
            domains.insert(&creation.domain),
            "duplicate settings creation"
        );
        validate_creation(j, creation)?;
    }
    anyhow::ensure!(
        j.phase != Phase::Prepared || j.creations.is_empty(),
        "unarmed creation intent"
    );
    Ok(outcome)
}
fn validate_creation(j: &Journal, c: &Creation) -> Result<()> {
    let p = j
        .pairs
        .iter()
        .find(|p| p.domain == c.domain)
        .context("unauthorized settings domain")?;
    anyhow::ensure!(
        c.file_name == p.document_name
            && c.marker_name == p.marker_name
            && uuid::Uuid::parse_str(&c.store_id).is_ok()
            && hash_valid(&c.document_sha256)
            && hash_valid(&c.marker_sha256)
            && c.will_create_marker
            && p.marker == Original::Absent,
        "invalid settings initialization binding"
    );
    match &p.document {
        Original::Absent => anyhow::ensure!(
            c.original_document_sha256.is_none() && c.will_write_document,
            "new document not authorized"
        ),
        Original::Present { sha256, .. } => {
            anyhow::ensure!(
                c.original_document_sha256.as_ref() == Some(sha256),
                "original document mismatch"
            );
            anyhow::ensure!(
                c.will_write_document || c.document_sha256 == *sha256,
                "unchanged document digest mismatch"
            );
        }
    }
    Ok(())
}
fn durable_tree(root: &Path) -> Result<()> {
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        let metadata = fs::symlink_metadata(&path)?;
        anyhow::ensure!(
            !metadata.file_type().is_symlink(),
            "backup contains symlink"
        );
        if metadata.is_dir() {
            durable_tree(&path)?;
        } else {
            open_read(&path)?.sync_all()?;
        }
    }
    sync_dir(root)
}
fn recover(guard: &mut StartupMigration) -> Result<()> {
    guard.revalidate()?;
    verify_backup(&guard.outcome)?;
    // 源与固定隔离目标共同判定，rename 后 sync 失败时不可因源已不存在而略过。
    let mut new_files = Vec::new();
    for pair in &guard.journal.pairs {
        let root = guard.root(&pair.root_label)?;
        let creation = guard
            .journal
            .creations
            .iter()
            .find(|v| v.domain == pair.domain);
        for (name, original, is_marker) in [
            (&pair.document_name, &pair.document, false),
            (&pair.marker_name, &pair.marker, true),
        ] {
            if original != &Original::Absent {
                continue;
            }
            let source = checked_path(root, Path::new(name))?;
            let destination = checked_path(
                root,
                &PathBuf::from("migration_restore_quarantine")
                    .join(format!("startup-{}", guard.journal.attempt_id))
                    .join(name),
            )?;
            let source_state = observe(&source)?;
            let target_state = observe(&destination)?;
            let (state, must_move) = match (&source_state, &target_state) {
                (Original::Absent, Original::Absent) => continue,
                (Original::Present { .. }, Original::Absent) => (&source_state, true),
                (Original::Absent, Original::Present { .. }) => (&target_state, false),
                _ => anyhow::bail!("quarantine exists and source reappeared; preserve both"),
            };
            let c = creation.context("unknown new settings file requires explicit repair")?;
            let expected = if is_marker {
                &c.marker_sha256
            } else {
                &c.document_sha256
            };
            anyhow::ensure!(
                matches!(state, Original::Present {sha256,..} if sha256 == expected)
                    && (is_marker || c.will_write_document),
                "new settings or quarantine changed; preserve for repair"
            );
            new_files.push((source, destination, state.clone(), must_move));
        }
    }
    guard.journal.phase = Phase::Restoring;
    guard.persist()?;
    fault("restoring")?;
    for (source, destination, expected, must_move) in new_files {
        let parent = source.parent().context("new settings missing parent")?;
        let quarantine = destination
            .parent()
            .context("quarantine missing directory")?;
        create_dirs_durable(quarantine)?;
        if must_move {
            anyhow::ensure!(
                observe(&source)? == expected && observe(&destination)? == Original::Absent,
                "new settings changed before quarantine"
            );
            fs::rename(&source, &destination)?;
            fault("quarantine_after_rename")?;
        } else {
            anyhow::ensure!(
                observe(&source)? == Original::Absent && observe(&destination)? == expected,
                "quarantine changed before durability confirmation"
            );
        }
        // 每次重入均同步现存精确隔离文件与完整目录链，再准许写 Recovered。
        fault("quarantine_before_file_sync")?;
        open_read(&destination)?.sync_all()?;
        fault("quarantine_after_file_sync")?;
        for (directory, before, after) in [
            (
                parent,
                "quarantine_before_source_sync",
                "quarantine_after_source_sync",
            ),
            (
                quarantine,
                "quarantine_before_target_sync",
                "quarantine_after_target_sync",
            ),
            (
                quarantine.parent().context("quarantine missing parent")?,
                "quarantine_before_ancestor_sync",
                "quarantine_after_ancestor_sync",
            ),
        ] {
            fault(before)?;
            sync_dir(directory)?;
            fault(after)?;
        }
        fault("quarantined")?;
    }
    restore_backup_inner(
        &guard.outcome,
        &guard.source_roots,
        SqliteRestorePolicy::ConsistentSnapshot,
        false,
    )?;
    fault("restored")?;
    guard.journal.phase = Phase::Recovered;
    guard.persist()?;
    fault("recovered")
}
#[cfg(test)]
thread_local! {static FAIL:std::cell::RefCell<Option<&'static str>>=const {std::cell::RefCell::new(None)};}
fn fault(_point: &str) -> Result<()> {
    #[cfg(test)]
    FAIL.with(|v| {
        if v.borrow().as_ref().is_some_and(|p| *p == _point) {
            anyhow::bail!("injected startup crash");
        }
        Ok(())
    })?;
    Ok(())
}

pub(super) fn create_dirs_durable(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => anyhow::ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "unsafe directory"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = path.parent().context("directory has no parent")?;
            create_dirs_durable(parent)?;
            fs::create_dir(path)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
            }
        }
        Err(error) => return Err(error.into()),
    }
    // mkdir 已成功、父目录同步失败后重入时，也必须重新确认这一目录项。
    sync_dir(path)?;
    if let Some(parent) = path.parent() {
        sync_dir(parent)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;

pub(super) fn journal_fault(path: &Path, point: &str) -> Result<()> {
    if path.file_name() == Some(std::ffi::OsStr::new(JOURNAL)) {
        fault(point)?;
    }
    Ok(())
}
