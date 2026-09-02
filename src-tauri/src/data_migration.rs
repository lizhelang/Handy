use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tauri::Manager;

const STARTUP_MIGRATION_ID: &str = "upstream-first-fbd4e15-v1";

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
    pub entries: Vec<MigrationBackupEntry>,
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

pub struct StartupMigration {
    _lock: MigrationLock,
    outcome: MigrationBackupOutcome,
    source_roots: Vec<MigrationSourceRoot>,
    marker_path: PathBuf,
    completed: bool,
}

#[derive(Debug)]
pub struct MigrationLock {
    path: PathBuf,
}

impl MigrationLock {
    pub fn acquire(lock_dir: &Path, name: &str) -> Result<Self> {
        fs::create_dir_all(lock_dir).with_context(|| {
            format!("failed to create migration lock dir {}", lock_dir.display())
        })?;
        let path = lock_dir.join(format!("{name}.lock"));
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

impl StartupMigration {
    pub fn complete(&mut self) -> Result<()> {
        verify_backup(&self.outcome)?;
        let (_, manifest_sha256) = file_fingerprint(&self.outcome.manifest_path)?;
        let marker = StartupMigrationMarker {
            migration_id: STARTUP_MIGRATION_ID.to_string(),
            completed_at: Utc::now().to_rfc3339(),
            manifest_path: self.outcome.manifest_path.clone(),
            manifest_sha256,
        };
        write_json_atomically(&self.marker_path, &marker)?;
        self.completed = true;
        Ok(())
    }

    #[cfg(test)]
    fn backup_dir(&self) -> &Path {
        &self.outcome.backup_dir
    }
}

impl Drop for StartupMigration {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        if let Err(err) = restore_backup(&self.outcome, &self.source_roots) {
            log::error!("Failed to restore incomplete startup migration: {err:#}");
        }
    }
}

pub fn prepare_startup_backup(app: &tauri::AppHandle) -> Result<Option<StartupMigration>> {
    let handy_root = crate::portable::app_data_dir(app)?;
    let backup_root = handy_root
        .join("migration_backups")
        .join(STARTUP_MIGRATION_ID);
    let lock_root = handy_root.join("migration_locks");

    #[cfg(target_os = "macos")]
    let inputia_root = app
        .path()
        .home_dir()
        .ok()
        .map(|home| home.join("Library/Application Support/Inputia"));
    #[cfg(not(target_os = "macos"))]
    let inputia_root: Option<PathBuf> = None;

    prepare_startup_backup_for_paths(
        &handy_root,
        inputia_root.as_deref(),
        &backup_root,
        &lock_root,
    )
}

fn prepare_startup_backup_for_paths(
    handy_root: &Path,
    inputia_root: Option<&Path>,
    backup_root: &Path,
    lock_root: &Path,
) -> Result<Option<StartupMigration>> {
    let marker_path = backup_root.join("complete.json");
    if marker_path.exists() {
        verify_startup_marker(&marker_path)?;
        return Ok(None);
    }

    let lock = MigrationLock::acquire(lock_root, STARTUP_MIGRATION_ID)?;
    let mut source_roots = vec![MigrationSourceRoot {
        label: "handy".to_string(),
        root: handy_root.to_path_buf(),
    }];
    let mut candidates = handy_data_candidates("handy");

    if let Some(inputia_root) = inputia_root.filter(|path| path.exists()) {
        source_roots.push(MigrationSourceRoot {
            label: "inputia".to_string(),
            root: inputia_root.to_path_buf(),
        });
        candidates.extend(inputia_data_candidates("inputia"));
    }

    recover_incomplete_startup_backup(backup_root, &source_roots)?;
    let outcome = prepare_backup_for_migration(
        &source_roots,
        &candidates,
        backup_root,
        STARTUP_MIGRATION_ID,
    )?;

    Ok(Some(StartupMigration {
        _lock: lock,
        outcome,
        source_roots,
        marker_path,
        completed: false,
    }))
}

fn recover_incomplete_startup_backup(
    backup_root: &Path,
    source_roots: &[MigrationSourceRoot],
) -> Result<()> {
    let Ok(entries) = fs::read_dir(backup_root) else {
        return Ok(());
    };
    let mut backup_dirs = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_dir()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("handy-data-"))
        })
        .collect::<Vec<_>>();
    backup_dirs.sort();

    let Some(backup_dir) = backup_dirs.pop() else {
        return Ok(());
    };
    let manifest_path = backup_dir.join("manifest.json");
    if !manifest_path.exists() {
        return Ok(());
    }
    let manifest: MigrationBackupManifest = serde_json::from_slice(
        &fs::read(&manifest_path)
            .with_context(|| format!("failed to read {}", manifest_path.display()))?,
    )?;
    anyhow::ensure!(
        manifest.migration_id == STARTUP_MIGRATION_ID,
        "unexpected incomplete migration id {}",
        manifest.migration_id
    );
    let outcome = MigrationBackupOutcome {
        backup_dir,
        manifest,
        manifest_path,
    };
    restore_backup(&outcome, source_roots)
        .context("failed to restore interrupted startup migration")
}

fn verify_startup_marker(marker_path: &Path) -> Result<()> {
    let marker: StartupMigrationMarker = serde_json::from_slice(
        &fs::read(marker_path)
            .with_context(|| format!("failed to read {}", marker_path.display()))?,
    )?;
    anyhow::ensure!(
        marker.migration_id == STARTUP_MIGRATION_ID,
        "startup migration marker has unexpected id {}",
        marker.migration_id
    );
    let (_, actual_sha256) = file_fingerprint(&marker.manifest_path)?;
    anyhow::ensure!(
        actual_sha256 == marker.manifest_sha256,
        "startup migration manifest checksum changed"
    );
    Ok(())
}

pub fn handy_data_candidates(label: &str) -> Vec<MigrationPathCandidate> {
    candidates_for_root(
        label,
        &[
            ("history.db", MigrationPathKind::Sqlite),
            ("history.db-wal", MigrationPathKind::File),
            ("history.db-shm", MigrationPathKind::File),
            ("clipboard.db", MigrationPathKind::Sqlite),
            ("clipboard.db-wal", MigrationPathKind::File),
            ("clipboard.db-shm", MigrationPathKind::File),
            ("settings_store.json", MigrationPathKind::File),
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
            ("inputia_memory.db", MigrationPathKind::Sqlite),
            ("inputia_memory.db-wal", MigrationPathKind::File),
            ("inputia_memory.db-shm", MigrationPathKind::File),
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
    fs::create_dir_all(backup_root)
        .with_context(|| format!("failed to create backup root {}", backup_root.display()))?;

    let backup_dir = backup_root.join(format!(
        "handy-data-{}",
        Utc::now().format("%Y%m%dT%H%M%S%3fZ")
    ));
    fs::create_dir_all(&backup_dir)
        .with_context(|| format!("failed to create backup dir {}", backup_dir.display()))?;

    let mut entries = Vec::new();

    for candidate in candidates {
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

pub fn restore_backup(
    outcome: &MigrationBackupOutcome,
    source_roots: &[MigrationSourceRoot],
) -> Result<()> {
    verify_backup(outcome)?;

    for entry in &outcome.manifest.entries {
        let Some(root) = source_roots
            .iter()
            .find(|root| root.label == entry.source_root_label)
        else {
            continue;
        };
        let source = root.root.join(&entry.source_relative_path);
        let backup = outcome.backup_dir.join(&entry.backup_relative_path);
        if let Some(parent) = source.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create restore dir {}", parent.display()))?;
        }
        fs::copy(&backup, &source).with_context(|| {
            format!(
                "failed to restore {} from {}",
                source.display(),
                backup.display()
            )
        })?;
    }

    let mut restored = outcome.clone();
    restored.manifest.status = MigrationBackupStatus::Restored;
    for entry in &mut restored.manifest.entries {
        entry.status = MigrationBackupStatus::Restored;
    }
    write_manifest(&restored)?;
    Ok(())
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
        let mut conn = Connection::open(db_path)
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
    for entry in fs::read_dir(current)
        .with_context(|| format!("failed to read snapshot dir {}", current.display()))?
    {
        let path = entry?.path();
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
    let backup_relative = PathBuf::from(&root.label).join(source_relative);
    let backup = backup_dir.join(&backup_relative);
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
    let source_conn = Connection::open(source)
        .with_context(|| format!("failed to open source database {}", source.display()))?;
    let backup_sql = format!("VACUUM main INTO '{}'", escape_sql_path(backup));
    source_conn
        .execute_batch(&backup_sql)
        .with_context(|| format!("failed to backup sqlite database {}", source.display()))?;
    verify_sqlite_database(backup)
}

fn verify_sqlite_database(path: &Path) -> Result<()> {
    let conn = Connection::open(path)
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
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let temp_path = path.with_extension(format!("tmp-{}", std::process::id()));
    let bytes = serde_json::to_vec_pretty(value)?;
    {
        let mut file = fs::File::create(&temp_path)
            .with_context(|| format!("failed to create {}", temp_path.display()))?;
        file.write_all(&bytes)
            .with_context(|| format!("failed to write {}", temp_path.display()))?;
        file.sync_all()
            .with_context(|| format!("failed to sync {}", temp_path.display()))?;
    }
    fs::rename(&temp_path, path).with_context(|| {
        format!(
            "failed to move startup migration marker {} to {}",
            temp_path.display(),
            path.display()
        )
    })?;
    Ok(())
}

fn write_manifest(outcome: &MigrationBackupOutcome) -> Result<()> {
    let json = serde_json::to_vec_pretty(&outcome.manifest)?;
    let mut file = fs::File::create(&outcome.manifest_path).with_context(|| {
        format!(
            "failed to create migration manifest {}",
            outcome.manifest_path.display()
        )
    })?;
    file.write_all(&json).with_context(|| {
        format!(
            "failed to write migration manifest {}",
            outcome.manifest_path.display()
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

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
    fn dropping_incomplete_startup_migration_restores_source_files() {
        let temp = tempfile::tempdir().unwrap();
        let handy_root = temp.path().join("handy");
        let backup_root = temp.path().join("backups");
        let lock_root = temp.path().join("locks");
        fs::create_dir_all(&handy_root).unwrap();
        fs::write(handy_root.join("settings_store.json"), b"kept").unwrap();

        let migration =
            prepare_startup_backup_for_paths(&handy_root, None, &backup_root, &lock_root)
                .unwrap()
                .unwrap();
        fs::write(handy_root.join("settings_store.json"), b"changed").unwrap();
        drop(migration);

        assert_eq!(
            fs::read(handy_root.join("settings_store.json")).unwrap(),
            b"kept"
        );
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
