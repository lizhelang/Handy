use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

const STARTUP_MIGRATION_ID: &str = "upstream-first-fbd4e15-v1";
const SQLITE_SIDECARS: [&str; 3] = ["-wal", "-shm", "-journal"];

fn validate_relative(path: &Path) -> Result<()> {
    anyhow::ensure!(
        !path.as_os_str().is_empty()
            && path
                .components()
                .all(|part| matches!(part, Component::Normal(_))),
        "migration reference must be a non-empty relative path without traversal"
    );
    Ok(())
}

fn validate_root(root: &Path) -> Result<()> {
    anyhow::ensure!(
        root.is_absolute()
            && !root
                .components()
                .any(|part| matches!(part, Component::ParentDir | Component::CurDir)),
        "migration root must be an absolute path without traversal"
    );
    Ok(())
}

/// root 是调用者授权的域，relative 来自待验证数据。先做词法约束，再查域内元数据。
/// 不对未经授权的引用执行 canonicalize/stat，也不跟随域内链接。
fn checked_path(root: &Path, relative: &Path) -> Result<PathBuf> {
    validate_root(root)?;
    validate_relative(relative)?;
    let mut current = root.to_path_buf();
    let mut paths = vec![current.clone()];
    for part in relative.components() {
        current.push(part);
        paths.push(current.clone());
    }
    for (index, path) in paths.iter().enumerate() {
        match fs::symlink_metadata(path) {
            Ok(metadata) => {
                anyhow::ensure!(
                    !metadata.file_type().is_symlink(),
                    "migration reference contains a symbolic link"
                );
                anyhow::ensure!(
                    metadata.is_dir() || metadata.is_file(),
                    "migration reference contains a special file"
                );
                anyhow::ensure!(
                    index == paths.len() - 1 || metadata.is_dir(),
                    "migration reference has a non-directory parent"
                );
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    anyhow::ensure!(
                        !metadata.is_file() || metadata.nlink() == 1,
                        "migration reference contains a hard link"
                    );
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("migration reference metadata unavailable"),
        }
    }
    Ok(current)
}

fn checked_database(root: &Path, relative: &Path) -> Result<PathBuf> {
    let path = checked_path(root, relative)?;
    for suffix in SQLITE_SIDECARS {
        let mut sidecar = relative.as_os_str().to_owned();
        sidecar.push(suffix);
        checked_path(root, Path::new(&sidecar))?;
    }
    Ok(path)
}

fn validate_outcome_paths(
    outcome: &MigrationBackupOutcome,
    roots: Option<&[MigrationSourceRoot]>,
) -> Result<()> {
    let parent = outcome
        .backup_dir
        .parent()
        .context("backup domain has no parent")?;
    let name = outcome
        .backup_dir
        .file_name()
        .context("backup domain has no name")?;
    anyhow::ensure!(
        name.to_string_lossy().starts_with("handy-data-"),
        "unexpected backup domain name"
    );
    checked_path(parent, Path::new(name))?;
    anyhow::ensure!(
        outcome.manifest_path == outcome.backup_dir.join("manifest.json"),
        "manifest path is outside its backup domain"
    );
    checked_path(&outcome.backup_dir, Path::new("manifest.json"))?;
    // 整批检查必须完成后才能 fingerprint 或写入第一条，不能产生部分越界恢复。
    for entry in &outcome.manifest.entries {
        let label = Path::new(&entry.source_root_label);
        validate_relative(label)?;
        anyhow::ensure!(
            label.components().count() == 1,
            "migration source label must be one component"
        );
        validate_root(&entry.source_root)?;
        validate_relative(&entry.source_relative_path)?;
        validate_relative(&entry.backup_relative_path)?;
        anyhow::ensure!(
            entry.backup_relative_path == label.join(&entry.source_relative_path),
            "backup reference does not match its source mapping"
        );
        if let Some(roots) = roots {
            let root = roots
                .iter()
                .find(|root| root.label == entry.source_root_label)
                .context("manifest source root is not authorized")?;
            anyhow::ensure!(
                root.root == entry.source_root,
                "copied migration source root requires explicit relocation"
            );
            checked_database(&root.root, &entry.source_relative_path)?;
        }
        checked_database(&outcome.backup_dir, &entry.backup_relative_path)?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MigrationPathKind {
    File,
    Directory,
    Sqlite,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MigrationBackupStatus {
    Prepared,
    Verified,
    Restored,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationSourceRoot {
    pub label: String,
    pub root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationPathCandidate {
    pub source_root_label: String,
    pub relative_path: PathBuf,
    pub kind: MigrationPathKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationBackupEntry {
    pub source_root_label: String,
    pub source_root: PathBuf,
    pub source_relative_path: PathBuf,
    pub backup_relative_path: PathBuf,
    pub kind: MigrationPathKind,
    pub byte_len: u64,
    pub mtime_unix_ms: Option<i64>,
    pub sha256: String,
    pub status: MigrationBackupStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationBackupManifest {
    pub migration_id: String,
    pub application_version: String,
    pub created_at: String,
    pub status: MigrationBackupStatus,
    /// 缺失表示旧格式；新格式 SQLite 条目均为包含已提交 WAL 的自包含快照。
    #[serde(default)]
    pub sqlite_snapshot_format: Option<SqliteSnapshotFormat>,
    pub entries: Vec<MigrationBackupEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SqliteSnapshotFormat {
    VacuumIntoV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SqliteRestorePolicy {
    ConsistentSnapshot,
    /// 明确恢复哈希验证过的主快照时点，绝不声称合并旧 WAL 中未知的后续提交。
    VerifiedSnapshotPoint,
}

#[derive(Debug, Serialize)]
pub struct MigrationRestoreReport {
    pub policy: SqliteRestorePolicy,
    pub selected_manifest_created_at: String,
    pub sqlite_snapshot_hashes: Vec<(PathBuf, String)>,
    pub preserved_legacy_sidecars: Vec<PathBuf>,
    pub target_quarantines: Vec<PathBuf>,
    pub lossless_legacy_wal_merge: bool,
}

fn is_sqlite_sidecar(path: &Path, database: &Path) -> bool {
    SQLITE_SIDECARS.iter().any(|suffix| {
        let mut expected = database.as_os_str().to_owned();
        expected.push(suffix);
        path == Path::new(&expected)
    })
}

#[derive(Debug, Clone)]
pub struct MigrationBackupOutcome {
    pub backup_dir: PathBuf,
    pub manifest: MigrationBackupManifest,
    pub manifest_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StartupMigrationMarker {
    migration_id: String,
    completed_at: String,
    manifest_path: PathBuf,
    manifest_sha256: String,
}

#[derive(Debug)]
pub struct MigrationLock {
    path: PathBuf,
}

impl MigrationLock {
    pub fn acquire(lock_dir: &Path, name: &str) -> Result<Self> {
        let path = checked_path(lock_dir, Path::new(&format!("{name}.lock")))?;
        fs::create_dir_all(lock_dir).with_context(|| {
            format!("failed to create migration lock dir {}", lock_dir.display())
        })?;
        if path.exists() {
            let owner_pid = read_lock_pid(&path);
            if owner_pid.is_some_and(process_is_alive) {
                anyhow::bail!("migration lock is already held: {}", path.display());
            }
            fs::remove_file(&path)
                .with_context(|| format!("failed to remove stale lock {}", path.display()))?;
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .with_context(|| {
                format!(
                    "migration lock is already held or unavailable: {}",
                    path.display()
                )
            })?;
        writeln!(file, "pid={}", std::process::id())
            .with_context(|| format!("failed to write migration lock {}", path.display()))?;
        Ok(Self { path })
    }

    #[cfg(test)]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn read_lock_pid(path: &Path) -> Option<u32> {
    fs::read_to_string(path)
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("pid="))?
        .trim()
        .parse()
        .ok()
}

#[cfg(unix)]
fn process_is_alive(pid: u32) -> bool {
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    i32::try_from(pid)
        .ok()
        .is_some_and(|pid| unsafe { kill(pid, 0) == 0 })
}

#[cfg(windows)]
fn process_is_alive(pid: u32) -> bool {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    let Ok(handle) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else {
        return false;
    };
    unsafe {
        let _ = CloseHandle(handle);
    }
    true
}

#[cfg(not(any(unix, windows)))]
fn process_is_alive(_pid: u32) -> bool {
    false
}

impl Drop for MigrationLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

mod startup;
#[cfg(test)]
use startup::prepare_startup_backup_for_paths;
pub use startup::{
    prepare_startup_backup, prepare_startup_backup_with_preflight, StartupFailure,
    StartupFailureKind, StartupMigration,
};

fn verify_startup_marker(marker_path: &Path, roots: &[MigrationSourceRoot]) -> Result<()> {
    let backup_root = marker_path
        .parent()
        .context("marker has no backup domain")?;
    anyhow::ensure!(
        marker_path.file_name() == Some(std::ffi::OsStr::new("complete.json")),
        "unexpected startup marker name"
    );
    checked_path(backup_root, Path::new("complete.json"))?;
    let marker: StartupMigrationMarker = serde_json::from_slice(
        &fs::read(marker_path)
            .with_context(|| format!("failed to read {}", marker_path.display()))?,
    )?;
    anyhow::ensure!(
        marker.migration_id == STARTUP_MIGRATION_ID,
        "startup migration marker has unexpected id {}",
        marker.migration_id
    );
    let relative = marker.manifest_path.strip_prefix(backup_root).context(
        "copied marker manifest is outside current backup domain; explicit relocation required",
    )?;
    validate_relative(relative)?;
    anyhow::ensure!(
        relative.components().count() == 2
            && relative.file_name() == Some(std::ffi::OsStr::new("manifest.json")),
        "unexpected marker manifest layout"
    );
    checked_path(backup_root, relative)?;
    let (_, actual_sha256) = file_fingerprint(&marker.manifest_path)?;
    anyhow::ensure!(
        actual_sha256 == marker.manifest_sha256,
        "startup migration manifest checksum changed"
    );
    let manifest: MigrationBackupManifest =
        serde_json::from_slice(&fs::read(&marker.manifest_path)?)?;
    anyhow::ensure!(
        manifest.migration_id == marker.migration_id,
        "marker and manifest migration identities differ"
    );
    validate_outcome_paths(
        &MigrationBackupOutcome {
            backup_dir: marker
                .manifest_path
                .parent()
                .context("manifest has no domain")?
                .to_path_buf(),
            manifest_path: marker.manifest_path,
            manifest,
        },
        Some(roots),
    )?;
    Ok(())
}

pub fn handy_data_candidates(label: &str) -> Vec<MigrationPathCandidate> {
    candidates_for_root(
        label,
        &[
            ("history.db", MigrationPathKind::Sqlite),
            ("clipboard.db", MigrationPathKind::Sqlite),
            ("integration.db", MigrationPathKind::Sqlite),
            ("integration-learning.key", MigrationPathKind::File),
            ("settings_store.json", MigrationPathKind::File),
            (
                ".inputia-control-settings-initialized.json",
                MigrationPathKind::File,
            ),
            ("recordings", MigrationPathKind::Directory),
            ("clipboard_images", MigrationPathKind::Directory),
            ("models", MigrationPathKind::Directory),
        ],
    )
}

pub fn inputia_data_candidates(label: &str) -> Vec<MigrationPathCandidate> {
    candidates_for_root(
        label,
        &[
            ("settings.json", MigrationPathKind::File),
            (
                ".inputia-settings-initialized.json",
                MigrationPathKind::File,
            ),
            ("inputia_memory.db", MigrationPathKind::Sqlite),
            ("outbox.db", MigrationPathKind::Sqlite),
            ("policy.db", MigrationPathKind::Sqlite),
            ("snapshots", MigrationPathKind::Directory),
            ("rime", MigrationPathKind::Directory),
        ],
    )
}

pub fn prepare_backup(
    source_roots: &[MigrationSourceRoot],
    candidates: &[MigrationPathCandidate],
    backup_root: &Path,
) -> Result<MigrationBackupOutcome> {
    prepare_backup_for_migration(source_roots, candidates, backup_root, "adhoc")
}

fn prepare_backup_for_migration(
    source_roots: &[MigrationSourceRoot],
    candidates: &[MigrationPathCandidate],
    backup_root: &Path,
    migration_id: &str,
) -> Result<MigrationBackupOutcome> {
    // 对全部声明先做约束；非法候选不应在读到一半后才失败。
    for root in source_roots {
        validate_root(&root.root)?;
        let label = Path::new(&root.label);
        validate_relative(label)?;
        anyhow::ensure!(
            label.components().count() == 1,
            "migration source label must be one component"
        );
    }
    for candidate in candidates {
        let root = source_roots
            .iter()
            .find(|root| root.label == candidate.source_root_label)
            .context("backup candidate source root is not authorized")?;
        checked_database(&root.root, &candidate.relative_path)?;
        if candidate.kind == MigrationPathKind::Sqlite
            && !root.root.join(&candidate.relative_path).exists()
        {
            for suffix in SQLITE_SIDECARS {
                let mut sidecar = candidate.relative_path.as_os_str().to_owned();
                sidecar.push(suffix);
                anyhow::ensure!(
                    !root.root.join(sidecar).exists(),
                    "orphan SQLite sidecar requires explicit recovery before backup"
                );
            }
        }
    }
    checked_path(backup_root, Path::new("manifest.json"))?;
    fs::create_dir_all(backup_root)
        .with_context(|| format!("failed to create backup root {}", backup_root.display()))?;

    let backup_dir = backup_root.join(format!(
        "handy-data-{}",
        Utc::now().format("%Y%m%dT%H%M%S%9fZ")
    ));
    fs::create_dir(&backup_dir)
        .with_context(|| format!("failed to create backup dir {}", backup_dir.display()))?;

    let mut entries = Vec::new();

    for candidate in candidates {
        // 即使旧调用方仍列出 sidecar，也不能与 VACUUM 产物混配。
        if candidates.iter().any(|database| {
            database.kind == MigrationPathKind::Sqlite
                && database.source_root_label == candidate.source_root_label
                && is_sqlite_sidecar(&candidate.relative_path, &database.relative_path)
        }) {
            continue;
        }
        let Some(root) = source_roots
            .iter()
            .find(|root| root.label == candidate.source_root_label)
        else {
            continue;
        };
        let source = root.root.join(&candidate.relative_path);
        if !source.exists() {
            continue;
        }

        snapshot_path(root, candidate, &source, &backup_dir, &mut entries)?;
    }

    let manifest_path = backup_dir.join("manifest.json");
    let outcome = MigrationBackupOutcome {
        backup_dir,
        manifest: MigrationBackupManifest {
            migration_id: migration_id.to_string(),
            application_version: env!("CARGO_PKG_VERSION").to_string(),
            created_at: Utc::now().to_rfc3339(),
            status: MigrationBackupStatus::Prepared,
            sqlite_snapshot_format: Some(SqliteSnapshotFormat::VacuumIntoV1),
            entries,
        },
        manifest_path,
    };
    write_manifest(&outcome)?;
    verify_backup(&outcome)?;

    let mut verified = outcome;
    verified.manifest.status = MigrationBackupStatus::Verified;
    for entry in &mut verified.manifest.entries {
        entry.status = MigrationBackupStatus::Verified;
    }
    write_manifest(&verified)?;
    Ok(verified)
}

/// 默认只恢复一致的主快照；调用方必须已停止全部来源写入并关闭目标连接。
/// 旧备份存在不明确的非空 WAL/journal 时拒绝自动恢复，保留数据等待显式恢复点选择。
pub fn restore_backup(
    outcome: &MigrationBackupOutcome,
    source_roots: &[MigrationSourceRoot],
) -> Result<()> {
    restore_backup_with_policy(
        outcome,
        source_roots,
        SqliteRestorePolicy::ConsistentSnapshot,
    )
    .map(|_| ())
}

/// 前置合同：调用方已停止所有来源写入，并关闭目标 SQLite/Rime 连接。
/// SQLite 排他事务探针尽力发现活跃写入，不能证明 WAL 读者、空闲或稍后重开的连接已关闭。
/// 恢复通过新文件替换，不对 live SQLite 文件原地覆盖；跨数据库不是全局原子事务。
pub fn restore_backup_with_policy(
    outcome: &MigrationBackupOutcome,
    source_roots: &[MigrationSourceRoot],
    policy: SqliteRestorePolicy,
) -> Result<MigrationRestoreReport> {
    restore_backup_inner(outcome, source_roots, policy, true)
}

fn restore_backup_inner(
    outcome: &MigrationBackupOutcome,
    source_roots: &[MigrationSourceRoot],
    policy: SqliteRestorePolicy,
    record_status: bool,
) -> Result<MigrationRestoreReport> {
    validate_outcome_paths(outcome, Some(source_roots))?;
    let legacy_sidecars: Vec<_> = outcome
        .manifest
        .entries
        .iter()
        .filter(|entry| {
            outcome.manifest.entries.iter().any(|database| {
                database.kind == MigrationPathKind::Sqlite
                    && database.source_root_label == entry.source_root_label
                    && is_sqlite_sidecar(
                        &entry.source_relative_path,
                        &database.source_relative_path,
                    )
            })
        })
        .collect();
    anyhow::ensure!(
        outcome.manifest.sqlite_snapshot_format.is_none() || legacy_sidecars.is_empty(),
        "self-contained SQLite snapshot manifest cannot contain source sidecars"
    );
    let has_unknown_commits = legacy_sidecars.iter().any(|entry| {
        entry.byte_len > 0
            && !entry
                .source_relative_path
                .to_string_lossy()
                .ends_with("-shm")
    });
    anyhow::ensure!(!has_unknown_commits || policy == SqliteRestorePolicy::VerifiedSnapshotPoint,
        "legacy WAL/journal may contain later commits; explicit VerifiedSnapshotPoint selection required; original sidecars retained, no lossless merge claimed");
    verify_backup(outcome)?;

    for entry in outcome
        .manifest
        .entries
        .iter()
        .filter(|entry| entry.kind == MigrationPathKind::Sqlite)
    {
        let root = source_roots
            .iter()
            .find(|root| root.label == entry.source_root_label)
            .context("restore source root unavailable")?;
        require_sqlite_idle(&checked_database(&root.root, &entry.source_relative_path)?)?;
    }
    let mut report = MigrationRestoreReport {
        policy,
        selected_manifest_created_at: outcome.manifest.created_at.clone(),
        sqlite_snapshot_hashes: outcome
            .manifest
            .entries
            .iter()
            .filter(|entry| entry.kind == MigrationPathKind::Sqlite)
            .map(|entry| (entry.source_relative_path.clone(), entry.sha256.clone()))
            .collect(),
        preserved_legacy_sidecars: legacy_sidecars
            .iter()
            .map(|entry| outcome.backup_dir.join(&entry.backup_relative_path))
            .collect(),
        target_quarantines: Vec::new(),
        lossless_legacy_wal_merge: false,
    };

    for entry in &outcome.manifest.entries {
        if legacy_sidecars
            .iter()
            .any(|sidecar| std::ptr::eq(*sidecar, entry))
        {
            continue;
        }
        let Some(root) = source_roots
            .iter()
            .find(|root| root.label == entry.source_root_label)
        else {
            continue;
        };
        let source = checked_database(&root.root, &entry.source_relative_path)?;
        let backup = checked_database(&outcome.backup_dir, &entry.backup_relative_path)?;
        if let Some(parent) = source.parent() {
            startup::create_dirs_durable(parent)
                .with_context(|| format!("failed to create restore dir {}", parent.display()))?;
        }
        if entry.kind == MigrationPathKind::Sqlite {
            report
                .target_quarantines
                .push(restore_sqlite_file(&backup, &source)?);
        } else {
            restore_regular_file(&backup, &source)?;
        }
    }

    let mut restored = outcome.clone();
    restored.manifest.status = MigrationBackupStatus::Restored;
    for entry in &mut restored.manifest.entries {
        entry.status = MigrationBackupStatus::Restored;
    }
    if record_status {
        write_manifest(&restored)?;
    }
    write_json_atomically(&outcome.backup_dir.join("last-restore.json"), &report)?;
    log::info!("Migration restored selected snapshot point; legacy sidecars retained={}, target recovery quarantines={}", report.preserved_legacy_sidecars.len(), report.target_quarantines.len());
    Ok(report)
}

fn require_sqlite_idle(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    conn.busy_timeout(std::time::Duration::ZERO)?;
    conn.pragma_update(None, "locking_mode", "EXCLUSIVE")?;
    conn.execute_batch("BEGIN EXCLUSIVE; ROLLBACK;")
        .context("restore requires closed SQLite connections; target has an active transaction")?;
    Ok(())
}

fn restore_sqlite_file(backup: &Path, target: &Path) -> Result<PathBuf> {
    let parent = target
        .parent()
        .context("SQLite restore target has no parent")?;
    let name = target
        .file_name()
        .context("SQLite restore target has no name")?;
    let recovery_relative = PathBuf::from("migration_restore_quarantine").join(format!(
        "restore-{}-{}",
        std::process::id(),
        Utc::now().format("%Y%m%dT%H%M%S%9fZ")
    ));
    let recovery = checked_path(parent, &recovery_relative)?;
    anyhow::ensure!(!recovery.exists(), "restore quarantine collision");
    startup::create_dirs_durable(&recovery)?;
    let stage = checked_path(&recovery, Path::new("verified-snapshot.db"))?;
    fs::copy(backup, &stage)?;
    verify_sqlite_database(&stage)?;
    fs::File::open(&stage)?.sync_all()?;
    // 只移动当前确切主库和三种 sidecar，保留旧状态而非删除。失败时按逆序恢复。
    let mut moved = Vec::new();
    let replace = (|| -> Result<()> {
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let mut component = name.to_owned();
            component.push(suffix);
            let source = checked_path(parent, Path::new(&component))?;
            if source.exists() {
                let destination = checked_path(&recovery, Path::new(&component))?;
                fs::rename(&source, &destination)?;
                moved.push((source, destination));
                sync_dir(parent)?;
                sync_dir(&recovery)?;
            }
        }
        fs::rename(&stage, target)?;
        sync_dir(&recovery)?;
        sync_dir(parent)?;
        Ok(())
    })();
    if let Err(error) = replace {
        for (source, destination) in moved.iter().rev() {
            fs::rename(destination, source)
                .context("SQLite restore failed and quarantine rollback requires recovery")?;
            sync_dir(parent)?;
            sync_dir(&recovery)?;
        }
        return Err(error).context("SQLite replacement failed; previous exact files restored");
    }
    Ok(recovery)
}

pub fn run_sqlite_migration_with_backup<F>(
    db_path: &Path,
    backup_root: &Path,
    migrate: F,
) -> Result<MigrationBackupOutcome>
where
    F: FnOnce(&mut Connection) -> Result<()>,
{
    let source_root = db_path
        .parent()
        .context("sqlite database path must have a parent directory")?;
    let file_name = db_path
        .file_name()
        .context("sqlite database path must include a file name")?;
    let roots = vec![MigrationSourceRoot {
        label: "sqlite".into(),
        root: source_root.to_path_buf(),
    }];
    let candidates = vec![MigrationPathCandidate {
        source_root_label: "sqlite".into(),
        relative_path: PathBuf::from(file_name),
        kind: MigrationPathKind::Sqlite,
    }];
    let outcome = prepare_backup(&roots, &candidates, backup_root)?;

    let migration_result = (|| {
        let mut conn =
            Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)
                .with_context(|| format!("failed to open database {}", db_path.display()))?;
        migrate(&mut conn)
    })();

    match migration_result {
        Ok(()) => Ok(outcome),
        Err(error) => {
            restore_backup(&outcome, &roots)?;
            Err(error)
                .context("sqlite migration failed after backup; original database was restored")
        }
    }
}

pub fn verify_backup(outcome: &MigrationBackupOutcome) -> Result<()> {
    validate_outcome_paths(outcome, None)?;
    for entry in &outcome.manifest.entries {
        let backup = outcome.backup_dir.join(&entry.backup_relative_path);
        if entry.kind == MigrationPathKind::Sqlite {
            verify_sqlite_database(&backup)?;
        }
        let (byte_len, sha256) = file_fingerprint(&backup)?;
        anyhow::ensure!(
            byte_len == entry.byte_len,
            "backup size changed for {}",
            entry.backup_relative_path.display()
        );
        anyhow::ensure!(
            sha256 == entry.sha256,
            "backup sha256 changed for {}",
            entry.backup_relative_path.display()
        );
    }
    Ok(())
}

fn candidates_for_root(
    label: &str,
    paths: &[(&str, MigrationPathKind)],
) -> Vec<MigrationPathCandidate> {
    paths
        .iter()
        .map(|(relative_path, kind)| MigrationPathCandidate {
            source_root_label: label.to_string(),
            relative_path: PathBuf::from(relative_path),
            kind: *kind,
        })
        .collect()
}

fn snapshot_path(
    root: &MigrationSourceRoot,
    candidate: &MigrationPathCandidate,
    source: &Path,
    backup_dir: &Path,
    entries: &mut Vec<MigrationBackupEntry>,
) -> Result<()> {
    checked_database(&root.root, &candidate.relative_path)?;
    if source.is_dir() {
        snapshot_dir(root, candidate, source, source, backup_dir, entries)
    } else {
        snapshot_file(
            root,
            candidate,
            source,
            &candidate.relative_path,
            backup_dir,
            entries,
        )
    }
}

fn snapshot_dir(
    root: &MigrationSourceRoot,
    candidate: &MigrationPathCandidate,
    dir_root: &Path,
    current: &Path,
    backup_dir: &Path,
    entries: &mut Vec<MigrationBackupEntry>,
) -> Result<()> {
    checked_path(&root.root, current.strip_prefix(&root.root)?)?;
    for entry in fs::read_dir(current)
        .with_context(|| format!("failed to read snapshot dir {}", current.display()))?
    {
        let path = entry?.path();
        checked_path(&root.root, path.strip_prefix(&root.root)?)?;
        if path.is_dir() {
            snapshot_dir(root, candidate, dir_root, &path, backup_dir, entries)?;
        } else if path.is_file() {
            let relative_inside_dir = path.strip_prefix(dir_root)?;
            let source_relative = candidate.relative_path.join(relative_inside_dir);
            snapshot_file(
                root,
                candidate,
                &path,
                &source_relative,
                backup_dir,
                entries,
            )?;
        }
    }
    Ok(())
}

fn snapshot_file(
    root: &MigrationSourceRoot,
    candidate: &MigrationPathCandidate,
    source: &Path,
    source_relative: &Path,
    backup_dir: &Path,
    entries: &mut Vec<MigrationBackupEntry>,
) -> Result<()> {
    anyhow::ensure!(
        source == checked_database(&root.root, source_relative)?,
        "snapshot source mapping mismatch"
    );
    let backup_relative = PathBuf::from(&root.label).join(source_relative);
    let backup = checked_database(backup_dir, &backup_relative)?;
    if let Some(parent) = backup.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create backup parent {}", parent.display()))?;
    }

    if candidate.kind == MigrationPathKind::Sqlite {
        sqlite_consistent_copy(source, &backup)?;
    } else {
        fs::copy(source, &backup).with_context(|| {
            format!(
                "failed to copy {} to {}",
                source.display(),
                backup.display()
            )
        })?;
    }

    let (byte_len, sha256) = file_fingerprint(&backup)?;
    entries.push(MigrationBackupEntry {
        source_root_label: root.label.clone(),
        source_root: root.root.clone(),
        source_relative_path: source_relative.to_path_buf(),
        backup_relative_path: backup_relative,
        kind: candidate.kind,
        byte_len,
        mtime_unix_ms: mtime_unix_ms(source).ok(),
        sha256,
        status: MigrationBackupStatus::Prepared,
    });
    Ok(())
}

fn sqlite_consistent_copy(source: &Path, backup: &Path) -> Result<()> {
    let source_conn =
        Connection::open_with_flags(source, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("failed to open source database {}", source.display()))?;
    let backup_sql = format!("VACUUM main INTO '{}'", escape_sql_path(backup));
    source_conn
        .execute_batch(&backup_sql)
        .with_context(|| format!("failed to backup sqlite database {}", source.display()))?;
    verify_sqlite_database(backup)
}

fn verify_sqlite_database(path: &Path) -> Result<()> {
    // VACUUM INTO 是自包含主快照；验证不能让旧备份目录中的 WAL 改变其读取时点。
    let mut uri = tauri::Url::from_file_path(path)
        .map_err(|_| anyhow::anyhow!("invalid SQLite backup path"))?;
    uri.query_pairs_mut().append_pair("immutable", "1");
    let conn = Connection::open_with_flags(
        uri.as_str(),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )
    .with_context(|| format!("failed to open sqlite backup {}", path.display()))?;
    let result: String = conn
        .pragma_query_value(None, "integrity_check", |row| row.get(0))
        .with_context(|| format!("failed to run integrity_check on {}", path.display()))?;
    anyhow::ensure!(
        result == "ok",
        "sqlite backup {} failed integrity_check: {}",
        path.display(),
        result
    );
    Ok(())
}

fn file_fingerprint(path: &Path) -> Result<(u64, String)> {
    let mut file = fs::File::open(path)
        .with_context(|| format!("failed to open file for hashing {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut len = 0_u64;
    let mut buf = [0_u8; 8192];

    loop {
        let read = file
            .read(&mut buf)
            .with_context(|| format!("failed to read file for hashing {}", path.display()))?;
        if read == 0 {
            break;
        }
        len += read as u64;
        hasher.update(&buf[..read]);
    }

    Ok((len, format!("{:x}", hasher.finalize())))
}

fn mtime_unix_ms(path: &Path) -> Result<i64> {
    let modified = fs::metadata(path)
        .with_context(|| format!("failed to stat {}", path.display()))?
        .modified()
        .with_context(|| format!("failed to read mtime for {}", path.display()))?;
    let duration = modified
        .duration_since(std::time::UNIX_EPOCH)
        .context("file mtime is before unix epoch")?;
    Ok(duration.as_millis() as i64)
}

fn escape_sql_path(path: &Path) -> String {
    path.to_string_lossy().replace('\'', "''")
}

fn write_json_atomically<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let root = path
        .parent()
        .context("migration JSON has no authorized parent")?;
    let name = path
        .file_name()
        .context("migration JSON has no file name")?;
    checked_path(root, Path::new(name))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let temp_path = root.join(format!(".migration-{}.tmp", uuid::Uuid::new_v4()));
    let bytes = serde_json::to_vec_pretty(value)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let mut file = options.open(&temp_path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    startup::journal_fault(path, "journal_before_rename")?;
    fs::rename(&temp_path, path)?;
    startup::journal_fault(path, "journal_after_rename")?;
    sync_dir(root)?;
    startup::journal_fault(path, "journal_after_sync")?;
    Ok(())
}

fn write_manifest(outcome: &MigrationBackupOutcome) -> Result<()> {
    validate_outcome_paths(outcome, None)?;
    write_json_atomically(&outcome.manifest_path, &outcome.manifest)
}

fn sync_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    fs::File::open(path)?.sync_all()?;
    // Windows 没有可移植目录 fsync；不能把该平台当作本轮 macOS 耐久验收。
    Ok(())
}

fn restore_regular_file(backup: &Path, target: &Path) -> Result<()> {
    let parent = target.parent().context("restore target has no parent")?;
    let stage = parent.join(format!(".migration-{}.restore", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let mut output = options.open(&stage)?;
    std::io::copy(&mut fs::File::open(backup)?, &mut output)?;
    output.set_permissions(fs::metadata(backup)?.permissions())?;
    output.sync_all()?;
    fs::rename(&stage, target)?;
    sync_dir(parent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    #[test]
    fn wal_snapshot_and_later_sidecars_are_different_recovery_points() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.db");
        let snapshot = temp.path().join("snapshot.db");
        let writer = Connection::open(&source).unwrap();
        writer.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE records(id INTEGER PRIMARY KEY, body TEXT); INSERT INTO records VALUES(1,'committed before snapshot');").unwrap();
        sqlite_consistent_copy(&source, &snapshot).unwrap();
        let count = |path: &Path| -> rusqlite::Result<i64> {
            Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?
                .query_row("SELECT COUNT(*) FROM records", [], |row| row.get(0))
        };
        assert_eq!(
            count(&snapshot).unwrap(),
            1,
            "snapshot contains committed WAL row"
        );
        writer
            .execute(
                "INSERT INTO records VALUES(2,'committed after snapshot')",
                [],
            )
            .unwrap();
        fs::copy(
            temp.path().join("source.db-wal"),
            temp.path().join("snapshot.db-wal"),
        )
        .unwrap();
        fs::copy(
            temp.path().join("source.db-shm"),
            temp.path().join("snapshot.db-shm"),
        )
        .unwrap();
        let mixed = count(&snapshot);
        eprintln!("syntheticWalMixedRecovery before_snapshot=1 after_snapshot=2 mixed={mixed:?}");
        assert_eq!(count(&source).unwrap(), 2);
        // 此实验只证明两文件集来自不同时间点；不能用其混合结果作为可靠恢复依据。
        drop(writer);
    }

    #[test]
    fn active_wal_backup_is_self_contained_and_legacy_restore_requires_selected_point() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.db");
        let writer = Connection::open(&source).unwrap();
        writer.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE records(id INTEGER PRIMARY KEY, body TEXT, favorite INTEGER); INSERT INTO records VALUES(1,'kept',1);").unwrap();
        let roots = [MigrationSourceRoot {
            label: "source".into(),
            root: temp.path().into(),
        }];
        let candidates = [
            MigrationPathCandidate {
                source_root_label: "source".into(),
                relative_path: "source.db".into(),
                kind: MigrationPathKind::Sqlite,
            },
            MigrationPathCandidate {
                source_root_label: "source".into(),
                relative_path: "source.db-wal".into(),
                kind: MigrationPathKind::File,
            },
            MigrationPathCandidate {
                source_root_label: "source".into(),
                relative_path: "source.db-shm".into(),
                kind: MigrationPathKind::File,
            },
        ];
        let outcome = prepare_backup(&roots, &candidates, &temp.path().join("backups")).unwrap();
        assert_eq!(outcome.manifest.entries.len(), 1);
        assert_eq!(
            outcome.manifest.sqlite_snapshot_format,
            Some(SqliteSnapshotFormat::VacuumIntoV1)
        );
        let snapshot = outcome.backup_dir.join("source/source.db");
        let read_state = |path: &Path| -> (i64, String, i64) {
            Connection::open(path)
                .unwrap()
                .query_row(
                    "SELECT COUNT(*),MIN(body),SUM(favorite) FROM records",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap()
        };
        assert_eq!(read_state(&snapshot), (1, "kept".into(), 1));
        writer
            .execute("INSERT INTO records VALUES(2,'later',0)", [])
            .unwrap();
        let mut legacy = outcome.clone();
        legacy.manifest.sqlite_snapshot_format = None;
        for suffix in ["-wal", "-shm"] {
            let relative = PathBuf::from(format!("source.db{suffix}"));
            let backup_relative = PathBuf::from("source").join(&relative);
            let backup = outcome.backup_dir.join(&backup_relative);
            fs::copy(temp.path().join(&relative), &backup).unwrap();
            let (byte_len, sha256) = file_fingerprint(&backup).unwrap();
            legacy.manifest.entries.push(MigrationBackupEntry {
                source_root_label: "source".into(),
                source_root: temp.path().into(),
                source_relative_path: relative,
                backup_relative_path: backup_relative,
                kind: MigrationPathKind::File,
                byte_len,
                mtime_unix_ms: None,
                sha256,
                status: MigrationBackupStatus::Verified,
            });
        }
        write_manifest(&legacy).unwrap();
        let forensic_before =
            file_fingerprint(&legacy.backup_dir.join("source/source.db-wal")).unwrap();
        assert!(restore_backup(&legacy, &roots)
            .unwrap_err()
            .to_string()
            .contains("VerifiedSnapshotPoint"));
        assert_eq!(read_state(&source).0, 2);
        // 调用方完成静默窗口：先关闭所有目标连接，再选择早期已验证主快照时点。
        drop(writer);
        for _ in 0..2 {
            let report = restore_backup_with_policy(
                &legacy,
                &roots,
                SqliteRestorePolicy::VerifiedSnapshotPoint,
            )
            .unwrap();
            assert!(!report.lossless_legacy_wal_merge);
            assert_eq!(report.preserved_legacy_sidecars.len(), 2);
            assert_eq!(
                report.sqlite_snapshot_hashes[0].1,
                outcome.manifest.entries[0].sha256
            );
            assert_eq!(read_state(&source), (1, "kept".into(), 1));
            assert_eq!(
                file_fingerprint(&legacy.backup_dir.join("source/source.db-wal")).unwrap(),
                forensic_before
            );
            assert!(!temp.path().join("source.db-wal").exists());
            assert!(!temp.path().join("source.db-shm").exists());
            assert!(report.target_quarantines[0].join("source.db").exists());
        }
    }

    #[test]
    fn restore_rejects_active_sqlite_writer_before_replacing_any_file() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.db");
        create_sample_database(&source);
        let roots = [MigrationSourceRoot {
            label: "source".into(),
            root: temp.path().into(),
        }];
        let outcome = prepare_backup(
            &roots,
            &[MigrationPathCandidate {
                source_root_label: "source".into(),
                relative_path: "source.db".into(),
                kind: MigrationPathKind::Sqlite,
            }],
            &temp.path().join("backups"),
        )
        .unwrap();
        let writer = Connection::open(&source).unwrap();
        writer
            .execute_batch(
                "PRAGMA journal_mode=WAL; BEGIN IMMEDIATE; UPDATE items SET name='live';",
            )
            .unwrap();
        assert!(restore_backup(&outcome, &roots).is_err());
        assert!(!temp.path().join("migration_restore_quarantine").exists());
        assert_eq!(
            writer
                .query_row("SELECT name FROM items", [], |row| row.get::<_, String>(0))
                .unwrap(),
            "live"
        );
        writer.execute_batch("ROLLBACK;").unwrap();
        drop(writer);
        restore_backup(&outcome, &roots).unwrap();
        assert_eq!(read_item_name(&source), "kept");
    }

    #[test]
    fn missing_source_database_is_not_created_by_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("missing.db");
        assert!(sqlite_consistent_copy(&missing, &temp.path().join("backup.db")).is_err());
        assert!(!missing.exists());
    }

    #[test]
    fn offline_sqlite_replacement_quarantines_only_exact_target_files() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target.db");
        let backup = temp.path().join("backup.db");
        create_sample_database(&target);
        sqlite_consistent_copy(&target, &backup).unwrap();
        fs::write(temp.path().join("other.db-wal"), b"unrelated untouched").unwrap();
        for suffix in SQLITE_SIDECARS {
            fs::write(
                temp.path().join(format!("target.db{suffix}")),
                suffix.as_bytes(),
            )
            .unwrap();
        }
        // 没有打开的数据库句柄。仅验证离线替换函数的精确文件边界。
        let quarantine = restore_sqlite_file(&backup, &target).unwrap();
        for suffix in SQLITE_SIDECARS {
            assert!(!temp.path().join(format!("target.db{suffix}")).exists());
            assert_eq!(
                fs::read(quarantine.join(format!("target.db{suffix}"))).unwrap(),
                suffix.as_bytes()
            );
        }
        assert_eq!(
            fs::read(temp.path().join("other.db-wal")).unwrap(),
            b"unrelated untouched"
        );
        assert_eq!(read_item_name(&target), "kept");
    }

    fn synthetic_file_backup() -> (
        tempfile::TempDir,
        Vec<MigrationSourceRoot>,
        MigrationBackupOutcome,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("source");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("value.txt"), b"kept").unwrap();
        let roots = vec![MigrationSourceRoot {
            label: "synthetic".into(),
            root,
        }];
        let outcome = prepare_backup(
            &roots,
            &[MigrationPathCandidate {
                source_root_label: "synthetic".into(),
                relative_path: "value.txt".into(),
                kind: MigrationPathKind::File,
            }],
            &temp.path().join("backups"),
        )
        .unwrap();
        (temp, roots, outcome)
    }

    #[test]
    fn rejects_absolute_and_parent_backup_references_before_fingerprinting() {
        let (temp, _, outcome) = synthetic_file_backup();
        let outside = temp.path().join("outside");
        fs::write(&outside, b"kept").unwrap();
        for reference in [outside, PathBuf::from("../../outside")] {
            let mut altered = outcome.clone();
            altered.manifest.entries[0].backup_relative_path = reference;
            assert!(verify_backup(&altered).is_err());
        }
    }

    #[test]
    fn rejects_restore_escape_before_any_source_write() {
        let (temp, roots, outcome) = synthetic_file_backup();
        let outside = temp.path().join("outside");
        fs::write(&outside, b"outside untouched").unwrap();
        fs::write(roots[0].root.join("value.txt"), b"current unchanged").unwrap();
        for reference in [outside.clone(), PathBuf::from("../outside")] {
            let mut altered = outcome.clone();
            let mut invalid = altered.manifest.entries[0].clone();
            invalid.source_relative_path = reference;
            altered.manifest.entries.push(invalid);
            assert!(restore_backup(&altered, &roots).is_err());
            assert_eq!(
                fs::read(roots[0].root.join("value.txt")).unwrap(),
                b"current unchanged"
            );
            assert_eq!(fs::read(&outside).unwrap(), b"outside untouched");
        }
    }

    #[test]
    fn copied_backup_requires_explicit_source_root_relocation() {
        let (_temp, roots, mut outcome) = synthetic_file_backup();
        outcome.manifest.entries[0].source_root = PathBuf::from("/synthetic-old-profile/source");
        assert!(restore_backup(&outcome, &roots).is_err());
    }

    #[test]
    fn copied_complete_marker_cannot_reference_original_backup_domain() {
        let (temp, roots, outcome) = synthetic_file_backup();
        let copied_root = temp.path().join("copied-backups");
        fs::create_dir(&copied_root).unwrap();
        let marker = StartupMigrationMarker {
            migration_id: STARTUP_MIGRATION_ID.into(),
            completed_at: "synthetic".into(),
            manifest_path: outcome.manifest_path.clone(),
            manifest_sha256: file_fingerprint(&outcome.manifest_path).unwrap().1,
        };
        let marker_path = copied_root.join("complete.json");
        write_json_atomically(&marker_path, &marker).unwrap();
        assert!(verify_startup_marker(&marker_path, &roots).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_linked_backup_content_without_reading_its_target() {
        use std::os::unix::fs::symlink;
        let (temp, _, outcome) = synthetic_file_backup();
        let outside = temp.path().join("outside");
        fs::write(&outside, b"kept").unwrap();
        let backup = outcome
            .backup_dir
            .join(&outcome.manifest.entries[0].backup_relative_path);
        fs::remove_file(&backup).unwrap();
        symlink(&outside, &backup).unwrap();
        assert!(verify_backup(&outcome).is_err());
        fs::remove_file(&backup).unwrap();
        fs::hard_link(&outside, &backup).unwrap();
        assert!(verify_backup(&outcome).is_err());
        assert_eq!(fs::read(&outside).unwrap(), b"kept");
    }

    #[test]
    fn rejects_traversal_candidates_before_creating_a_backup() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        fs::create_dir(&source).unwrap();
        let backup = temp.path().join("backups");
        let roots = [MigrationSourceRoot {
            label: "source".into(),
            root: source,
        }];
        for relative in [PathBuf::from("../outside"), temp.path().join("outside")] {
            assert!(prepare_backup(
                &roots,
                &[MigrationPathCandidate {
                    source_root_label: "source".into(),
                    relative_path: relative,
                    kind: MigrationPathKind::File,
                }],
                &backup
            )
            .is_err());
            assert!(!backup.exists());
        }
    }

    #[test]
    fn missing_sqlite_backup_is_not_created_by_verification() {
        let (_temp, _, mut outcome) = synthetic_file_backup();
        outcome.manifest.entries[0].kind = MigrationPathKind::Sqlite;
        let backup = outcome
            .backup_dir
            .join(&outcome.manifest.entries[0].backup_relative_path);
        fs::remove_file(&backup).unwrap();
        assert!(verify_backup(&outcome).is_err());
        assert!(!backup.exists());
    }

    #[test]
    fn marker_rejects_missing_foreign_reference_before_attempting_to_open_it() {
        let temp = tempfile::tempdir().unwrap();
        let marker_path = temp.path().join("complete.json");
        let marker = StartupMigrationMarker {
            migration_id: STARTUP_MIGRATION_ID.into(),
            completed_at: "synthetic".into(),
            manifest_path: PathBuf::from("/synthetic-forbidden-not-present/manifest.json"),
            manifest_sha256: "not-read".into(),
        };
        write_json_atomically(&marker_path, &marker).unwrap();
        let error = verify_startup_marker(&marker_path, &[])
            .unwrap_err()
            .to_string();
        assert!(error.contains("outside current backup domain"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn rejects_linked_source_and_sqlite_sidecar_before_backup_creation() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("source");
        fs::create_dir(&root).unwrap();
        let outside = temp.path().join("outside");
        fs::write(&outside, b"untouched").unwrap();
        let roots = [MigrationSourceRoot {
            label: "source".into(),
            root: root.clone(),
        }];
        let candidates = [MigrationPathCandidate {
            source_root_label: "source".into(),
            relative_path: "db.sqlite".into(),
            kind: MigrationPathKind::Sqlite,
        }];
        let backup = temp.path().join("backups");
        symlink(&outside, root.join("db.sqlite-wal")).unwrap();
        assert!(prepare_backup(&roots, &candidates, &backup).is_err());
        assert!(!backup.exists());
        assert_eq!(fs::read(outside).unwrap(), b"untouched");
    }

    #[test]
    fn lists_handy_and_inputia_data_candidates() {
        let handy = handy_data_candidates("handy");
        let inputia = inputia_data_candidates("inputia");

        assert!(handy
            .iter()
            .any(|candidate| candidate.relative_path == Path::new("history.db")));
        assert!(handy
            .iter()
            .all(|candidate| candidate.source_root_label == "handy"));
        assert!(inputia
            .iter()
            .all(|candidate| candidate.source_root_label == "inputia"));
        assert!(inputia
            .iter()
            .any(|candidate| candidate.relative_path == Path::new("rime")));
    }

    #[test]
    fn lock_prevents_concurrent_migration_in_same_dir() {
        let temp = tempfile::tempdir().unwrap();
        let first = MigrationLock::acquire(temp.path(), "handy").unwrap();

        assert!(MigrationLock::acquire(temp.path(), "handy").is_err());
        assert!(first.path().exists());
        drop(first);
        assert!(MigrationLock::acquire(temp.path(), "handy").is_ok());
    }

    #[test]
    fn stale_lock_from_dead_process_is_recovered() {
        let temp = tempfile::tempdir().unwrap();
        let lock_path = temp.path().join("handy.lock");
        fs::write(&lock_path, "pid=4294967295\n").unwrap();

        let lock = MigrationLock::acquire(temp.path(), "handy").unwrap();
        assert_eq!(read_lock_pid(lock.path()), Some(std::process::id()));
    }

    #[test]
    fn snapshots_sqlite_files_and_directories_with_manifest() {
        let temp = tempfile::tempdir().unwrap();
        let app_data = temp.path().join("app-data");
        let backup_root = temp.path().join("backups");
        fs::create_dir_all(app_data.join("recordings")).unwrap();
        create_sample_database(&app_data.join("history.db"));
        fs::write(app_data.join("settings_store.json"), br#"{"theme":"dark"}"#).unwrap();
        fs::write(app_data.join("recordings/one.wav"), b"audio").unwrap();

        let roots = vec![MigrationSourceRoot {
            label: "app-data".into(),
            root: app_data,
        }];
        let candidates = vec![
            MigrationPathCandidate {
                source_root_label: "app-data".into(),
                relative_path: "history.db".into(),
                kind: MigrationPathKind::Sqlite,
            },
            MigrationPathCandidate {
                source_root_label: "app-data".into(),
                relative_path: "settings_store.json".into(),
                kind: MigrationPathKind::File,
            },
            MigrationPathCandidate {
                source_root_label: "app-data".into(),
                relative_path: "recordings".into(),
                kind: MigrationPathKind::Directory,
            },
        ];

        let outcome = prepare_backup(&roots, &candidates, &backup_root).unwrap();

        assert_eq!(outcome.manifest.status, MigrationBackupStatus::Verified);
        assert_eq!(outcome.manifest.entries.len(), 3);
        assert!(outcome.backup_dir.join("app-data/history.db").exists());
        assert!(outcome
            .backup_dir
            .join("app-data/recordings/one.wav")
            .exists());
        verify_backup(&outcome).unwrap();
    }

    #[test]
    fn backs_up_sqlite_database_before_running_migration() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("history.db");
        let backup_root = temp.path().join("backups");
        create_sample_database(&db_path);

        let outcome = run_sqlite_migration_with_backup(&db_path, &backup_root, |conn| {
            conn.execute("ALTER TABLE items ADD COLUMN pinned INTEGER DEFAULT 0", [])?;
            Ok(())
        })
        .unwrap();

        assert!(outcome.manifest.entries[0]
            .backup_relative_path
            .ends_with("history.db"));
        assert_eq!(outcome.manifest.status, MigrationBackupStatus::Verified);
        assert_eq!(
            read_item_name(
                &outcome
                    .backup_dir
                    .join(&outcome.manifest.entries[0].backup_relative_path)
            ),
            "kept"
        );
    }

    #[test]
    fn restores_sqlite_database_when_migration_fails() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("clipboard.db");
        let backup_root = temp.path().join("backups");
        create_sample_database(&db_path);

        let result = run_sqlite_migration_with_backup(&db_path, &backup_root, |conn| {
            conn.execute("UPDATE items SET name = 'corrupted'", [])?;
            anyhow::bail!("forced migration failure")
        });

        assert!(result.is_err());
        assert_eq!(read_item_name(&db_path), "kept");
    }

    #[test]
    fn restore_is_idempotent_and_marks_manifest_restored() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("inputia_memory.db");
        let backup_root = temp.path().join("backups");
        create_sample_database(&db_path);

        let roots = vec![MigrationSourceRoot {
            label: "sqlite".into(),
            root: temp.path().to_path_buf(),
        }];
        let outcome = prepare_backup(
            &roots,
            &[MigrationPathCandidate {
                source_root_label: "sqlite".into(),
                relative_path: "inputia_memory.db".into(),
                kind: MigrationPathKind::Sqlite,
            }],
            &backup_root,
        )
        .unwrap();

        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute("UPDATE items SET name = 'changed'", [])
                .unwrap();
        }

        restore_backup(&outcome, &roots).unwrap();
        restore_backup(&outcome, &roots).unwrap();

        assert_eq!(read_item_name(&db_path), "kept");
        let manifest: MigrationBackupManifest =
            serde_json::from_slice(&fs::read(outcome.manifest_path).unwrap()).unwrap();
        assert_eq!(manifest.status, MigrationBackupStatus::Restored);
    }

    #[test]
    fn failed_backup_does_not_modify_source_database() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("clipboard.db");
        create_sample_database(&db_path);

        let result = prepare_backup(
            &[MigrationSourceRoot {
                label: "app-data".into(),
                root: temp.path().to_path_buf(),
            }],
            &[MigrationPathCandidate {
                source_root_label: "app-data".into(),
                relative_path: "clipboard.db".into(),
                kind: MigrationPathKind::Sqlite,
            }],
            &db_path,
        );

        assert!(result.is_err());
        assert_eq!(read_item_name(&db_path), "kept");
    }

    #[test]
    fn startup_backup_covers_handy_and_inputia_and_completes_once() {
        let temp = tempfile::tempdir().unwrap();
        let handy_root = temp.path().join("handy");
        let inputia_root = temp.path().join("Inputia");
        let backup_root = temp.path().join("backups");
        let lock_root = temp.path().join("locks");
        fs::create_dir_all(handy_root.join("recordings")).unwrap();
        fs::create_dir_all(inputia_root.join("rime")).unwrap();
        create_sample_database(&handy_root.join("history.db"));
        create_sample_database(&inputia_root.join("inputia_memory.db"));
        fs::write(handy_root.join("recordings/one.wav"), b"audio").unwrap();
        fs::write(inputia_root.join("settings.json"), b"{}").unwrap();
        fs::write(inputia_root.join("rime/user.yaml"), b"state").unwrap();

        let mut migration = prepare_startup_backup_for_paths(
            &handy_root,
            Some(&inputia_root),
            &backup_root,
            &lock_root,
        )
        .unwrap()
        .expect("first startup should create a backup");
        assert!(migration.backup_dir().join("handy/history.db").exists());
        assert!(migration
            .backup_dir()
            .join("inputia/inputia_memory.db")
            .exists());
        assert!(migration
            .backup_dir()
            .join("inputia/rime/user.yaml")
            .exists());
        migration.begin_mutations().unwrap();
        migration.complete().unwrap();
        drop(migration);

        assert!(prepare_startup_backup_for_paths(
            &handy_root,
            Some(&inputia_root),
            &backup_root,
            &lock_root,
        )
        .unwrap()
        .is_none());
    }

    #[test]
    fn failed_setup_defers_restore_until_next_startup_after_connections_close() {
        let temp = tempfile::tempdir().unwrap();
        let handy_root = temp.path().join("handy");
        let backup_root = temp.path().join("backups");
        let lock_root = temp.path().join("locks");
        fs::create_dir_all(&handy_root).unwrap();
        fs::write(handy_root.join("settings_store.json"), b"kept").unwrap();
        create_sample_database(&handy_root.join("history.db"));

        let mut migration =
            prepare_startup_backup_for_paths(&handy_root, None, &backup_root, &lock_root)
                .unwrap()
                .unwrap();
        migration.begin_mutations().unwrap();
        let writer = Connection::open(handy_root.join("history.db")).unwrap();
        writer
            .execute_batch("PRAGMA journal_mode=WAL; UPDATE items SET name='changed';")
            .unwrap();
        drop(migration);
        assert_eq!(
            fs::read(handy_root.join("settings_store.json")).unwrap(),
            b"kept"
        );
        assert_eq!(
            writer
                .query_row("SELECT name FROM items", [], |row| row.get::<_, String>(0))
                .unwrap(),
            "changed"
        );
        // 模拟失败进程已退出/关闭连接，下一次启动仍在任何 manager 打开前。
        drop(writer);
        let mut resumed =
            prepare_startup_backup_for_paths(&handy_root, None, &backup_root, &lock_root)
                .unwrap()
                .unwrap();
        assert_eq!(
            fs::read(handy_root.join("settings_store.json")).unwrap(),
            b"kept"
        );
        assert_eq!(read_item_name(&handy_root.join("history.db")), "kept");
        resumed.begin_mutations().unwrap();
        resumed.complete().unwrap();
    }

    #[test]
    fn formal_backup_restores_records_favorites_terms_key_and_host_state_twice() {
        let temp = tempfile::tempdir().unwrap();
        let handy = temp.path().join("Handy");
        let inputia = temp.path().join("Inputia");
        fs::create_dir_all(&handy).unwrap();
        fs::create_dir_all(inputia.join("snapshots")).unwrap();
        let key = [7_u8; 32];
        fs::write(handy.join("integration-learning.key"), key).unwrap();
        fs::write(
            inputia.join("snapshots/terms.json"),
            b"synthetic confirmed terms",
        )
        .unwrap();
        let databases = [
            handy.join("history.db"),
            handy.join("clipboard.db"),
            handy.join("integration.db"),
            inputia.join("inputia_memory.db"),
            inputia.join("outbox.db"),
            inputia.join("policy.db"),
        ];
        for path in &databases {
            let conn = Connection::open(path).unwrap();
            conn.execute_batch("CREATE TABLE records(body TEXT,favorite INTEGER); INSERT INTO records VALUES('synthetic record',1); CREATE TABLE terms(term TEXT); INSERT INTO terms VALUES('Inputia');").unwrap();
        }
        let roots = [
            MigrationSourceRoot {
                label: "handy".into(),
                root: handy.clone(),
            },
            MigrationSourceRoot {
                label: "inputia".into(),
                root: inputia.clone(),
            },
        ];
        let candidates: Vec<_> = handy_data_candidates("handy")
            .into_iter()
            .chain(inputia_data_candidates("inputia"))
            .collect();
        let outcome = prepare_backup(&roots, &candidates, &temp.path().join("backups")).unwrap();
        assert_eq!(
            outcome
                .manifest
                .entries
                .iter()
                .filter(|entry| entry.kind == MigrationPathKind::Sqlite)
                .count(),
            6
        );
        assert!(outcome
            .manifest
            .entries
            .iter()
            .any(|entry| entry.source_relative_path == Path::new("integration-learning.key")));
        for _ in 0..2 {
            for path in &databases {
                let conn = Connection::open(path).unwrap();
                conn.execute_batch("DELETE FROM records; DELETE FROM terms;")
                    .unwrap();
            }
            fs::write(handy.join("integration-learning.key"), b"changed").unwrap();
            fs::write(inputia.join("snapshots/terms.json"), b"changed").unwrap();
            // 上面每个连接已经离开作用域，所有源写入者在恢复期间保持关闭。
            restore_backup(&outcome, &roots).unwrap();
            for path in &databases {
                let conn = Connection::open(path).unwrap();
                assert_eq!(
                    conn.query_row("SELECT body,favorite FROM records", [], |row| Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?
                    )))
                    .unwrap(),
                    ("synthetic record".into(), 1)
                );
                assert_eq!(
                    conn.query_row("SELECT term FROM terms", [], |row| row.get::<_, String>(0))
                        .unwrap(),
                    "Inputia"
                );
            }
            assert_eq!(
                fs::read(handy.join("integration-learning.key")).unwrap(),
                key
            );
            assert_eq!(
                fs::read(inputia.join("snapshots/terms.json")).unwrap(),
                b"synthetic confirmed terms"
            );
        }
    }

    #[test]
    #[ignore = "requires explicit real Handy data paths"]
    fn real_data_startup_backup_is_verified_before_cutover() {
        let handy_root = PathBuf::from(
            std::env::var_os("HANDY_REAL_DATA_ROOT")
                .expect("HANDY_REAL_DATA_ROOT must point at the existing app data"),
        );
        let inputia_root = std::env::var_os("HANDY_REAL_INPUTIA_ROOT").map(PathBuf::from);
        let backup_root = handy_root
            .join("migration_backups")
            .join(STARTUP_MIGRATION_ID);
        let lock_root = handy_root.join("migration_locks");

        let migration = prepare_startup_backup_for_paths(
            &handy_root,
            inputia_root.as_deref(),
            &backup_root,
            &lock_root,
        )
        .unwrap();

        if let Some(mut migration) = migration {
            assert!(!migration.outcome.manifest.entries.is_empty());
            eprintln!("verifiedBackupDir={}", migration.backup_dir().display());
            migration.begin_mutations().unwrap();
            migration.complete().unwrap();
        } else {
            // prepare_startup_backup_for_paths 已在返回 None 前校验 marker 与授权根。
            eprintln!("verifiedBackupMarker={}", backup_root.display());
        }
    }

    fn create_sample_database(db_path: &Path) {
        let conn = Connection::open(db_path).unwrap();
        conn.execute(
            "CREATE TABLE items (id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO items (name) VALUES (?1)", params!["kept"])
            .unwrap();
    }

    fn read_item_name(db_path: &Path) -> String {
        let conn = Connection::open(db_path).unwrap();
        conn.query_row("SELECT name FROM items WHERE id = 1", [], |row| row.get(0))
            .unwrap()
    }
}
