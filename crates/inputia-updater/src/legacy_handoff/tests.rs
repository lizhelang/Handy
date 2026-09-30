use super::*;
use crate::MaintenanceMarker;
use inputia_settings::installation::{
    ComponentPaths, DataLocation, InstallationScope, UpdateChannel,
};
use std::{
    cell::Cell,
    fs::File,
    io::{BufRead, BufReader, Write},
    os::unix::fs::PermissionsExt,
    process::{Command, Stdio},
    rc::Rc,
};
fn private(path: &Path) {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}
struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap().join("profile");
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = root.join(OLD_DATABASE_NAME);
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE terms(text TEXT PRIMARY KEY, count INTEGER); INSERT INTO terms VALUES('fixture-private-term', 7);").unwrap();
        db.close().unwrap();
        private(&path);
        Self { _temp: temp, root }
    }
}
fn handoff(root: &Path) -> LegacyMemoryHandoff {
    let uid = unsafe { libc::geteuid() };
    let subject = Subject {
        transaction_id: "11111111-1111-4111-8111-111111111111".into(),
        installation_id: "22222222-2222-4222-8222-222222222222".into(),
        new_release_id: "inputia-new".into(),
        plan_sha256: "a".repeat(64),
    };
    LegacyMemoryHandoff {
        authority: GuardianTransactionAuthority {
            lock: fs::lock(&root.join("fixture-update.lock"), uid).unwrap(),
            marker: MaintenanceMarker {
                schema_version: 1,
                transaction_id: subject.transaction_id.clone(),
                installation_id: subject.installation_id.clone(),
                old_release_id: Some("inputia-old".into()),
                new_release_id: subject.new_release_id.clone(),
                epoch: "33333333-3333-4333-8333-333333333333".into(),
                plan_sha256: subject.plan_sha256.clone(),
            },
            subject,
        },
        location: LocatedInstallation {
            receipt: InstallationReceipt {
                schema_version: 1,
                product_id: "com.inputia".into(),
                installation_id: "22222222-2222-4222-8222-222222222222".into(),
                profile_id: "44444444-4444-4444-8444-444444444444".into(),
                uid,
                scope: InstallationScope::User,
                data: DataLocation::Managed,
                components: ComponentPaths {
                    control: root.join("control"),
                    ime: root.join("ime"),
                    settings: root.join("settings"),
                },
                release_id: "inputia-old".into(),
                channel: UpdateChannel::Candidate,
            },
            inputia_root: root.into(),
            handy_root: root.join("Handy"),
            pair_manifest: root.join("pair"),
        },
        fixture: true,
    }
}
fn origin(h: &LegacyMemoryHandoff) -> VerifiedLegacyOrigin {
    VerifiedLegacyOrigin {
        subject: h.authority.subject.clone(),
        epoch: h.authority.marker.epoch.clone(),
        check: Box::new(|_, _| Ok(())),
    }
}
fn count(path: &Path) -> i64 {
    let db =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    db.query_row(
        "SELECT count FROM terms WHERE text='fixture-private-term'",
        [],
        |row| row.get(0),
    )
    .unwrap()
}
#[test]
fn fence_snapshot_and_exclusive_instances_are_real_but_not_origin_authority() {
    let fixture = Fixture::new();
    let h = handoff(&fixture.root);
    assert!(matches!(
        h.origin_authority(),
        Err(HandoffError::OriginProofRequired)
    ));
    let mut proof = origin(&h);
    let lease = h.advance(&mut proof).unwrap();
    assert!(fixture.root.join(OLD_DATABASE_NAME).is_dir());
    assert!(rusqlite::Connection::open(fixture.root.join(OLD_DATABASE_NAME)).is_err());
    assert_eq!(count(&lease.database_path()), 7);
    assert!(lease.requires_origin_and_connection_validation());
    let journal = h.load().unwrap().unwrap();
    assert!(matches!(
        OwnedMemoryDomainLease::acquire(&fixture.root, h.uid(), &binding(&journal).unwrap()),
        Err(inputia_settings::memory_domain::LeaseError::Busy)
    ));
    let status = h.status().unwrap();
    assert!(
        status.path_fenced
            && status.snapshot_published
            && status.origin_proof_required
            && !status.production_connected
    );
    assert!(
        !String::from_utf8(std::fs::read(fixture.root.join(JOURNAL)).unwrap())
            .unwrap()
            .contains("fixture-private-term")
    );
    drop(lease);
    let lease = h.advance(&mut proof).unwrap();
    lease.assert_current().unwrap();
    let target = lease.database_path();
    std::fs::rename(&target, target.with_extension("preserved")).unwrap();
    std::fs::write(&target, b"replacement").unwrap();
    private(&target);
    assert!(lease.assert_current().is_err());
}
#[test]
fn source_absence_unknown_target_links_and_unknown_archives_never_create_a_fresh_domain() {
    for mode in ["missing", "symlink", "hardlink", "unknown_target"] {
        let fixture = Fixture::new();
        let old = fixture.root.join(OLD_DATABASE_NAME);
        match mode {
            "missing" => std::fs::remove_file(&old).unwrap(),
            "symlink" => {
                std::fs::rename(&old, fixture.root.join("preserved")).unwrap();
                std::os::unix::fs::symlink("preserved", &old).unwrap();
            }
            "hardlink" => std::fs::hard_link(&old, fixture.root.join("alias")).unwrap(),
            _ => std::fs::create_dir(fixture.root.join(DOMAIN_DIRECTORY)).unwrap(),
        }
        let h = handoff(&fixture.root);
        assert!(h.advance(&mut origin(&h)).is_err(), "{mode}");
        assert!(!h.managed().join(DATABASE_NAME).exists());
    }
}
#[test]
fn post_fence_origin_failure_never_publishes_and_remains_recoverable() {
    let fixture = Fixture::new();
    let h = handoff(&fixture.root);
    let post = Rc::new(Cell::new(false));
    let flag = post.clone();
    let mut proof = origin(&h);
    proof.check = Box::new(move |_, fenced| {
        if fenced && !flag.get() {
            Err(HandoffError::OriginProofRequired)
        } else {
            Ok(())
        }
    });
    assert!(matches!(
        h.advance(&mut proof),
        Err(HandoffError::OriginProofRequired)
    ));
    assert!(fixture.root.join(OLD_DATABASE_NAME).is_dir());
    assert!(!h.managed().join(DATABASE_NAME).exists());
    post.set(true);
    assert_eq!(count(&h.advance(&mut proof).unwrap().database_path()), 7);
}
#[test]
fn forged_ready_does_not_bypass_fence_origin_content_or_instance_evidence() {
    for mode in [
        "ready",
        "target_inode",
        "target_content",
        "archive_inode",
        "missing_fence",
    ] {
        let fixture = Fixture::new();
        let h = handoff(&fixture.root);
        let mut proof = origin(&h);
        if mode == "ready" {
            let _ = h.drive(&mut proof, &mut |point| {
                if point == "journal" {
                    Err(HandoffError::Busy)
                } else {
                    Ok(())
                }
            });
            let mut j = h.load().unwrap().unwrap();
            j.ready = true;
            h.save(&j).unwrap();
            proof.check = Box::new(|_, _| Err(HandoffError::OriginProofRequired));
            assert!(matches!(
                h.advance(&mut proof),
                Err(HandoffError::OriginProofRequired)
            ));
            assert!(!h.managed().join(DATABASE_NAME).exists());
            continue;
        }
        drop(h.advance(&mut proof).unwrap());
        let target = h.managed().join(DATABASE_NAME);
        match mode {
            "target_inode" => {
                std::fs::rename(&target, h.managed().join("target-preserved")).unwrap();
                std::fs::copy(h.managed().join("target-preserved"), &target).unwrap();
            }
            "target_content" => {
                let db = rusqlite::Connection::open(&target).unwrap();
                db.execute("UPDATE terms SET count=1", []).unwrap();
            }
            "archive_inode" => {
                let old = h.archive().join(OLD_DATABASE_NAME);
                std::fs::rename(&old, h.archive().join("preserved")).unwrap();
                std::fs::copy(h.archive().join("preserved"), old).unwrap();
            }
            _ => std::fs::rename(
                fixture.root.join(OLD_DATABASE_NAME),
                fixture.root.join("fence-preserved"),
            )
            .unwrap(),
        }
        assert!(h.advance(&mut proof).is_err(), "{mode}");
    }
}
#[test]
fn source_wal_is_copied_with_original_basename_and_recovered() {
    let fixture = Fixture::new();
    let source_temp = tempfile::tempdir().unwrap();
    let source = source_temp.path().join(OLD_DATABASE_NAME);
    let live = rusqlite::Connection::open(&source).unwrap();
    live.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE terms(text TEXT PRIMARY KEY,count INTEGER); INSERT INTO terms VALUES('fixture-private-term', 17);").unwrap();
    for name in SOURCE_NAMES {
        if source_temp.path().join(name).exists() {
            std::fs::copy(source_temp.path().join(name), fixture.root.join(name)).unwrap();
            private(&fixture.root.join(name));
        }
    }
    live.close().unwrap();
    let h = handoff(&fixture.root);
    let lease = h.advance(&mut origin(&h)).unwrap();
    assert_eq!(count(&lease.database_path()), 17);
    assert!(h.archive().join("inputia_memory.db-wal").exists());
    assert!(!h.managed().join("memory.sqlite-wal").exists());
}
// 自建进程专用入口：不读取固定用户路径，不进行生产origin发行。
#[test]
fn child_fixture() {
    let Ok(root) = std::env::var("INPUTIA_HANDOFF_TEST_ROOT") else {
        return;
    };
    let mode = std::env::var("INPUTIA_HANDOFF_TEST_MODE").unwrap();
    let root = PathBuf::from(root);
    if mode == "hold" {
        let file = File::open(
            root.join(DOMAIN_DIRECTORY)
                .join("legacy-source")
                .join(OLD_DATABASE_NAME),
        )
        .unwrap();
        println!("HANDOFF_CHILD_READY");
        std::io::stdout().flush().unwrap();
        let mut byte = [0u8; 1];
        std::io::stdin().read_exact(&mut byte).unwrap();
        drop(file);
        return;
    }
    if mode == "lease" {
        let journal: Journal =
            serde_json::from_slice(&std::fs::read(root.join(JOURNAL)).unwrap()).unwrap();
        let result = OwnedMemoryDomainLease::acquire(
            &root,
            unsafe { libc::geteuid() },
            &binding(&journal).unwrap(),
        );
        assert!(matches!(
            result,
            Err(inputia_settings::memory_domain::LeaseError::Busy)
        ));
        return;
    }
    if mode == "hot_journal" {
        let db = rusqlite::Connection::open(root.join(OLD_DATABASE_NAME)).unwrap();
        db.execute_batch("PRAGMA cache_size=1; PRAGMA synchronous=FULL; BEGIN IMMEDIATE; UPDATE terms SET count=99; CREATE TABLE padding AS WITH RECURSIVE x(n) AS(SELECT 1 UNION ALL SELECT n+1 FROM x WHERE n<1000) SELECT randomblob(1024) AS body FROM x;").unwrap();
        println!("HANDOFF_CHILD_READY");
        std::io::stdout().flush().unwrap();
        loop {
            std::thread::park();
        }
    }
    let h = handoff(&root);
    let mut proof = origin(&h);
    h.drive(&mut proof, &mut |point| {
        if point == mode {
            println!("HANDOFF_CHILD_READY");
            std::io::stdout().flush().unwrap();
            loop {
                std::thread::park();
            }
        }
        Ok(())
    })
    .unwrap();
}
fn child(root: &Path, mode: &str) -> std::process::Child {
    Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "legacy_handoff::tests::child_fixture",
            "--nocapture",
        ])
        .env("INPUTIA_HANDOFF_TEST_ROOT", root)
        .env("INPUTIA_HANDOFF_TEST_MODE", mode)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap()
}
fn await_ready(child: &mut std::process::Child) {
    let mut line = String::new();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    loop {
        line.clear();
        assert!(
            reader.read_line(&mut line).unwrap() > 0,
            "fixture exited before checkpoint"
        );
        if line.contains("HANDOFF_CHILD_READY") {
            break;
        }
    }
    // 保留读端直到子进程退出，不能让夹具关闭pipe造成测试框架结束输出失败。
    child.stdout = Some(reader.into_inner());
}
#[test]
fn actual_single_process_crashes_recover_at_all_durable_boundaries() {
    for point in [
        "journal",
        "fence",
        "archive_0",
        "archive_1",
        "archive_2",
        "archive_3",
        "origin",
        "snapshot",
        "published",
        "ready",
    ] {
        let fixture = Fixture::new();
        let mut process = child(&fixture.root, point);
        await_ready(&mut process);
        process.kill().unwrap();
        process.wait().unwrap();
        let h = handoff(&fixture.root);
        let lease = h
            .advance(&mut origin(&h))
            .unwrap_or_else(|e| panic!("{point}: {e}"));
        assert_eq!(count(&lease.database_path()), 7);
    }
}
#[test]
fn rollback_journal_is_recovered_and_old_open_inode_after_fence_blocks_publication() {
    let fixture = Fixture::new();
    let mut process = child(&fixture.root, "hot_journal");
    await_ready(&mut process);
    process.kill().unwrap();
    process.wait().unwrap();
    private(&fixture.root.join("inputia_memory.db-journal"));
    let h = handoff(&fixture.root);
    let lease = h.advance(&mut origin(&h)).unwrap();
    assert_eq!(count(&lease.database_path()), 7);
    drop(lease);
    drop(h);

    let fixture = Fixture::new();
    let h = handoff(&fixture.root);
    let mut proof = origin(&h);
    // 先在全部文件归档之后暂停；旧进程此时重开实际归档inode，必须由post-fence核验拒绝。
    let _ = h.drive(&mut proof, &mut |point| {
        if point == "archive_3" {
            Err(HandoffError::Busy)
        } else {
            Ok(())
        }
    });
    let process = Rc::new(std::cell::RefCell::new(child(&fixture.root, "hold")));
    await_ready(&mut process.borrow_mut());
    let witness = process.clone();
    proof.check = Box::new(move |_, post| {
        if post && witness.borrow_mut().try_wait().unwrap().is_none() {
            Err(HandoffError::OriginProofRequired)
        } else {
            Ok(())
        }
    });
    assert!(matches!(
        h.advance(&mut proof),
        Err(HandoffError::OriginProofRequired)
    ));
    assert!(!h.managed().join(DATABASE_NAME).exists());
    process
        .borrow_mut()
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"x")
        .unwrap();
    assert!(process.borrow_mut().wait().unwrap().success());
    assert_eq!(count(&h.advance(&mut proof).unwrap().database_path()), 7);
}

#[test]
fn cooperative_lock_survives_connection_lifetime_and_blocks_another_process() {
    let fixture = Fixture::new();
    let h = handoff(&fixture.root);
    let lease = h.advance(&mut origin(&h)).unwrap();
    let db = rusqlite::Connection::open(lease.database_path()).unwrap();
    assert_eq!(
        db.query_row("SELECT count FROM terms", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        7
    );
    let resource = lease.bind_resource(db);
    let mut other = child(&fixture.root, "lease");
    assert!(other.wait().unwrap().success());
    resource.assert_current().unwrap();
    // 正常SQLite写入不变更文件身份；合作租约不把初始快照摘要当作永久不可变内容。
    resource
        .resource()
        .execute("UPDATE terms SET count=8", [])
        .unwrap();
    resource.assert_current().unwrap();
    drop(resource);
    let journal = h.load().unwrap().unwrap();
    OwnedMemoryDomainLease::acquire(&fixture.root, h.uid(), &binding(&journal).unwrap()).unwrap();
}

#[test]
fn sqlite_handle_has_moved_detects_wrong_connection_even_when_path_reopens() {
    let fixture = Fixture::new();
    let h = handoff(&fixture.root);
    let lease = h.advance(&mut origin(&h)).unwrap();
    let path = lease.database_path();
    let db = rusqlite::Connection::open(&path).unwrap();
    // 先使用SQLite句柄再替换固定路径，不能仅核database_list字符串。
    db.query_row("SELECT count(*) FROM terms", [], |row| row.get::<_, i64>(0))
        .unwrap();
    std::fs::rename(&path, h.managed().join("preserved-instance")).unwrap();
    std::fs::copy(h.managed().join("preserved-instance"), &path).unwrap();
    let mut moved: libc::c_int = 0;
    let result = unsafe {
        rusqlite::ffi::sqlite3_file_control(
            db.handle(),
            c"main".as_ptr(),
            rusqlite::ffi::SQLITE_FCNTL_HAS_MOVED,
            (&mut moved as *mut libc::c_int).cast(),
        )
    };
    assert_eq!(result, rusqlite::ffi::SQLITE_OK);
    assert_ne!(moved, 0);
    assert!(lease.assert_current().is_err());
    drop(db);
    drop(lease);
}

#[test]
fn move_effect_with_failed_directory_sync_never_becomes_success_on_reentry() {
    for point in [
        "exchange_sync",
        "archive_sync_0",
        "archive_sync_1",
        "archive_sync_2",
        "archive_sync_3",
        "publish_sync",
    ] {
        let fixture = Fixture::new();
        let h = handoff(&fixture.root);
        let mut proof = origin(&h);
        for _ in 0..2 {
            let result = h.drive(&mut proof, &mut |observed| {
                if observed == point {
                    Err(HandoffError::Io(std::io::Error::from_raw_os_error(
                        libc::EIO,
                    )))
                } else {
                    Ok(())
                }
            });
            assert!(matches!(result, Err(HandoffError::Io(_))), "{point}");
            assert!(!h.load().unwrap().unwrap().ready);
        }
        let lease = h.advance(&mut proof).unwrap();
        assert_eq!(count(&lease.database_path()), 7);
    }
}

#[test]
fn unknown_workspace_and_staged_target_sidecars_are_preserved_before_sqlite_open() {
    for where_ in ["workspace", "target"] {
        for suffix in ["-wal", "-shm", "-journal"] {
            let fixture = Fixture::new();
            let h = handoff(&fixture.root);
            let unknown = if where_ == "workspace" {
                h.managed()
                    .join("snapshot-source")
                    .join(format!("{OLD_DATABASE_NAME}{suffix}"))
            } else {
                h.managed().join(format!("snapshot.sqlite{suffix}"))
            };
            let mut proof = origin(&h);
            let result = h.drive(&mut proof, &mut |point| {
                if point == "before_sqlite" {
                    std::fs::write(&unknown, b"unclaimed-private-sidecar").unwrap();
                    private(&unknown);
                }
                Ok(())
            });
            assert!(
                matches!(
                    result,
                    Err(HandoffError::RepairRequired(
                        "unclaimed_workspace_sidecar" | "unclaimed_target_sidecar"
                    ))
                ),
                "{where_} {suffix}"
            );
            assert_eq!(
                std::fs::read(&unknown).unwrap(),
                b"unclaimed-private-sidecar"
            );
            assert!(!h.managed().join(DATABASE_NAME).exists());
            assert_eq!(
                std::fs::metadata(h.managed().join("snapshot.sqlite"))
                    .unwrap()
                    .len(),
                0
            );
        }
    }
}
