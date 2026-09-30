use super::*;
use inputia_settings::memory_domain::{
    FileIdentity, MemoryFileBinding, DATABASE_NAME, DOMAIN_DIRECTORY, FENCE_RECORD,
    OLD_DATABASE_NAME,
};
use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    binding: MemoryFileBinding,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap().join("fixture-profile");
        for path in [
            &root,
            &root.join(DOMAIN_DIRECTORY),
            &root.join(OLD_DATABASE_NAME),
        ] {
            fs::create_dir(path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let path = root.join(DOMAIN_DIRECTORY).join(DATABASE_NAME);
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE fixture(value INTEGER); INSERT INTO fixture VALUES(7);")
            .unwrap();
        db.close().unwrap();
        private(&path);
        for path in [
            root.join(DOMAIN_DIRECTORY).join("service.lock"),
            root.join(OLD_DATABASE_NAME).join(FENCE_RECORD),
        ] {
            fs::write(&path, b"fixture-only").unwrap();
            private(&path);
        }
        let identity = |path: PathBuf| FileIdentity::of(&File::open(path).unwrap()).unwrap();
        let binding = MemoryFileBinding {
            database: identity(path),
            service_lock: identity(root.join(DOMAIN_DIRECTORY).join("service.lock")),
            fence: identity(root.join(OLD_DATABASE_NAME)),
            fence_record: identity(root.join(OLD_DATABASE_NAME).join(FENCE_RECORD)),
        };
        // 仅自建子进程测试使用，序列化文件事实不产生生产origin授权。
        fs::write(
            root.join("fixture-binding.json"),
            serde_json::to_vec(&binding).unwrap(),
        )
        .unwrap();
        Self {
            _temp: temp,
            root,
            binding,
        }
    }
    fn lease(&self) -> OwnedMemoryDomainLease {
        OwnedMemoryDomainLease::acquire(&self.root, unsafe { libc::geteuid() }, &self.binding)
            .unwrap()
    }
    fn path(&self) -> PathBuf {
        self.root.join(DOMAIN_DIRECTORY).join(DATABASE_NAME)
    }
}
fn private(path: &Path) {
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}
fn child(root: &Path, mode: &str) -> std::process::Child {
    Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "owned_memory_connection::tests::child_fixture",
            "--nocapture",
        ])
        .env("INPUTIA_OWNED_SQLITE_FIXTURE", root)
        .env("INPUTIA_OWNED_SQLITE_MODE", mode)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap()
}
fn probe(root: &Path, mode: &str) {
    assert!(child(root, mode).wait().unwrap().success(), "{mode}");
}
fn ready(child: &mut std::process::Child) {
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    loop {
        line.clear();
        assert!(
            reader.read_line(&mut line).unwrap() > 0,
            "child exited before ready"
        );
        if line.contains("OWNED_SQLITE_READY") {
            break;
        }
    }
    child.stdout = Some(reader.into_inner());
}
fn leaked_statement(owner: &OwnedSqliteConnection) -> *mut rusqlite::ffi::sqlite3_stmt {
    owner
        .with_connection(|db| -> rusqlite::Result<_> {
            let mut statement = std::ptr::null_mut();
            // 仅夹具刻意保留未finalize语句，制造真实sqlite3_close(BUSY)。
            let code = unsafe {
                rusqlite::ffi::sqlite3_prepare_v2(
                    db.handle(),
                    c"SELECT value FROM fixture".as_ptr(),
                    -1,
                    &mut statement,
                    std::ptr::null_mut(),
                )
            };
            assert_eq!(code, rusqlite::ffi::SQLITE_OK);
            Ok(statement)
        })
        .unwrap()
}

#[test]
fn child_fixture() {
    let Ok(root) = std::env::var("INPUTIA_OWNED_SQLITE_FIXTURE") else {
        return;
    };
    let root = PathBuf::from(root);
    let mode = std::env::var("INPUTIA_OWNED_SQLITE_MODE").unwrap();
    let binding: MemoryFileBinding =
        serde_json::from_slice(&fs::read(root.join("fixture-binding.json")).unwrap()).unwrap();
    if mode == "sqlite_busy" {
        let db = Connection::open(root.join(DOMAIN_DIRECTORY).join(DATABASE_NAME)).unwrap();
        db.busy_timeout(std::time::Duration::ZERO).unwrap();
        assert_eq!(
            db.execute_batch("BEGIN IMMEDIATE;")
                .unwrap_err()
                .sqlite_error_code(),
            Some(rusqlite::ErrorCode::DatabaseBusy)
        );
        return;
    }
    let lease = OwnedMemoryDomainLease::acquire(&root, unsafe { libc::geteuid() }, &binding);
    if mode == "lock_busy" {
        assert!(matches!(lease, Err(LeaseError::Busy)));
        return;
    }
    let owner = OwnedSqliteConnection::open(lease.unwrap()).unwrap();
    if mode == "drop_busy" {
        let _statement = leaked_statement(&owner);
        drop(owner);
        println!("OWNED_SQLITE_READY");
        std::io::stdout().flush().unwrap();
        let mut byte = [0];
        std::io::stdin().read_exact(&mut byte).unwrap();
        // 故意不finalize，只有子进程真正退出才回收兜底保留的Connection和租约。
        return;
    }
    assert_eq!(mode, "available");
    owner
        .with_connection(|db| -> rusqlite::Result<()> {
            db.busy_timeout(std::time::Duration::ZERO)?;
            db.execute_batch("BEGIN IMMEDIATE; ROLLBACK;")?;
            let value: i64 = db.query_row("SELECT value FROM fixture", [], |r| r.get(0))?;
            assert_eq!(value, 7);
            Ok(())
        })
        .unwrap();
}

#[test]
fn open_does_not_initialize_schema_or_change_journal_mode() {
    let fixture = Fixture::new();
    let before = fs::read(fixture.path()).unwrap();
    let mut owner = OwnedSqliteConnection::open(fixture.lease()).unwrap();
    owner
        .with_connection(|db| -> rusqlite::Result<()> {
            assert_eq!(
                db.query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0))?,
                "delete"
            );
            assert_eq!(
                db.query_row("SELECT count(*) FROM sqlite_schema", [], |r| r
                    .get::<_, i64>(0))?,
                1
            );
            Ok(())
        })
        .unwrap();
    probe(&fixture.root, "lock_busy");
    owner.try_close().unwrap();
    owner.try_close().unwrap();
    assert_eq!(before, fs::read(fixture.path()).unwrap());
    probe(&fixture.root, "available");
    assert!(matches!(
        owner.with_connection(|_| Ok::<_, ()>(())),
        Err(AccessError::BeforeOperation(VerificationError::Closed))
    ));
}

#[test]
fn invalid_or_missing_target_is_rejected_without_creation_or_sql_effects() {
    for mode in ["missing", "replacement", "symlink"] {
        let fixture = Fixture::new();
        let lease = fixture.lease();
        let preserved = fixture.root.join("preserved.sqlite");
        fs::rename(fixture.path(), &preserved).unwrap();
        match mode {
            "replacement" => {
                fs::write(fixture.path(), b"not-authorized-sqlite").unwrap();
                private(&fixture.path());
            }
            "symlink" => std::os::unix::fs::symlink(&preserved, fixture.path()).unwrap(),
            _ => {}
        }
        let before = fs::read(&preserved).unwrap();
        assert!(OwnedSqliteConnection::open(lease).is_err(), "{mode}");
        assert_eq!(before, fs::read(&preserved).unwrap());
        if mode == "missing" {
            assert!(!fixture.path().exists());
        }
        assert!(!fixture.path().with_extension("sqlite-wal").exists());
    }
}

#[test]
fn checks_preserve_transaction_lock_and_close_rolls_back_before_releasing_lease() {
    let fixture = Fixture::new();
    let mut owner = OwnedSqliteConnection::open(fixture.lease()).unwrap();
    owner
        .with_connection(|db| db.execute_batch("BEGIN IMMEDIATE; UPDATE fixture SET value=99;"))
        .unwrap();
    probe(&fixture.root, "sqlite_busy");
    owner
        .with_connection(|db| db.query_row("SELECT value FROM fixture", [], |r| r.get::<_, i64>(0)))
        .unwrap();
    probe(&fixture.root, "sqlite_busy");
    probe(&fixture.root, "lock_busy");
    owner.try_close().unwrap();
    probe(&fixture.root, "available");
}

#[test]
fn close_busy_retains_both_connection_and_lock_until_successful_retry() {
    let fixture = Fixture::new();
    let mut owner = OwnedSqliteConnection::open(fixture.lease()).unwrap();
    let statement = leaked_statement(&owner);
    assert_eq!(
        owner.try_close().unwrap_err().sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseBusy)
    );
    probe(&fixture.root, "lock_busy");
    owner
        .with_connection(|db| db.query_row("SELECT value FROM fixture", [], |r| r.get::<_, i64>(0)))
        .unwrap();
    assert_eq!(
        unsafe { rusqlite::ffi::sqlite3_finalize(statement) },
        rusqlite::ffi::SQLITE_OK
    );
    owner.try_close().unwrap();
    probe(&fixture.root, "available");
}

#[test]
fn drop_busy_keeps_lock_in_own_child_until_actual_process_exit() {
    let fixture = Fixture::new();
    let mut holder = child(&fixture.root, "drop_busy");
    ready(&mut holder);
    probe(&fixture.root, "lock_busy");
    holder.stdin.as_mut().unwrap().write_all(b"x").unwrap();
    assert!(holder.wait().unwrap().success());
    probe(&fixture.root, "available");
}

#[test]
fn successful_normal_drop_closes_connection_before_other_process_acquires() {
    let fixture = Fixture::new();
    let owner = OwnedSqliteConnection::open(fixture.lease()).unwrap();
    owner
        .with_connection(|db| db.execute_batch("BEGIN IMMEDIATE; UPDATE fixture SET value=99;"))
        .unwrap();
    probe(&fixture.root, "lock_busy");
    drop(owner);
    probe(&fixture.root, "available");
}

#[test]
fn changed_binding_after_commit_is_uncertain_and_never_replays() {
    let fixture = Fixture::new();
    let mut owner = OwnedSqliteConnection::open(fixture.lease()).unwrap();
    let preserved = fixture.root.join("committed.sqlite");
    let result = owner.with_connection(|db| -> rusqlite::Result<()> {
        db.execute("UPDATE fixture SET value=value+1", [])?;
        fs::rename(fixture.path(), &preserved).unwrap();
        fs::write(fixture.path(), b"replacement-preserved").unwrap();
        private(&fixture.path());
        Ok(())
    });
    assert!(matches!(
        result,
        Err(AccessError::Uncertain {
            operation_error: None,
            ..
        })
    ));
    owner.try_close().unwrap();
    let db = Connection::open_with_flags(&preserved, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert_eq!(
        db.query_row("SELECT value FROM fixture", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        8
    );
    assert_eq!(fs::read(fixture.path()).unwrap(), b"replacement-preserved");
}

#[test]
fn operation_failure_preserves_its_error_without_reclassifying_as_uncertain() {
    let fixture = Fixture::new();
    let owner = OwnedSqliteConnection::open(fixture.lease()).unwrap();
    assert!(matches!(
        owner.with_connection(|db| db.execute_batch("not valid SQL")),
        Err(AccessError::Operation(_))
    ));
}
