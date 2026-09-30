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
    assert!(matches!(
        h.status(),
        Err(HandoffError::Lease(
            inputia_settings::memory_domain::LeaseError::Busy
        ))
    ));
    drop(lease);
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
    if mode == "service_busy" || mode == "hold_service" {
        // 服务进程只持域锁；交接事务的外层更新锁仍可由父进程持有。
        let journal: Journal =
            serde_json::from_slice(&std::fs::read(root.join(JOURNAL)).unwrap()).unwrap();
        let result = OwnedMemoryServiceLock::acquire(
            &root,
            unsafe { libc::geteuid() },
            journal.managed_dir.unwrap(),
            journal.service_lock.unwrap(),
        );
        if mode == "service_busy" {
            assert!(matches!(
                result,
                Err(inputia_settings::memory_domain::LeaseError::Busy)
            ));
            return;
        }
        let service = result.unwrap();
        println!("HANDOFF_CHILD_READY");
        std::io::stdout().flush().unwrap();
        let mut byte = [0];
        std::io::stdin().read_exact(&mut byte).unwrap();
        drop(service);
        return;
    }
    if mode == "reenter_busy" {
        let h = handoff(&root);
        let mut proof = origin(&h);
        proof.check = Box::new(|_, _| panic!("live服务锁前不应执行origin回调"));
        assert!(matches!(
            h.drive(&mut proof, &mut |_| panic!("live服务锁前不应推进")),
            Err(HandoffError::Lease(
                inputia_settings::memory_domain::LeaseError::Busy
            ))
        ));
        assert!(matches!(
            h.status(),
            Err(HandoffError::Lease(
                inputia_settings::memory_domain::LeaseError::Busy
            ))
        ));
        return;
    }
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
    if mode == "sqlite_locked" || mode == "sqlite_writable" {
        let db =
            rusqlite::Connection::open(root.join(DOMAIN_DIRECTORY).join(DATABASE_NAME)).unwrap();
        db.busy_timeout(std::time::Duration::ZERO).unwrap();
        let result = db.execute_batch("BEGIN IMMEDIATE; UPDATE terms SET count=count+1; ROLLBACK;");
        if mode == "sqlite_locked" {
            assert_eq!(
                result.unwrap_err().sqlite_error_code(),
                Some(rusqlite::ErrorCode::DatabaseBusy)
            );
        } else {
            result.unwrap();
        }
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
        "bootstrap_journal",
        "managed_registered",
        "service_registered",
        "service_sync",
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
fn lease_validation_preserves_sqlite_transaction_locks_against_another_process() {
    let fixture = Fixture::new();
    let h = handoff(&fixture.root);
    let lease = h.advance(&mut origin(&h)).unwrap();
    let db = rusqlite::Connection::open(lease.database_path()).unwrap();
    db.execute_batch("BEGIN IMMEDIATE;").unwrap();
    assert!(child(&fixture.root, "sqlite_locked")
        .wait()
        .unwrap()
        .success());
    for _ in 0..3 {
        lease.assert_current().unwrap();
        // 不能另开关主文件FD：Unix的POSIX锁可能随同进程任何该inode的FD关闭而释放。
        assert!(child(&fixture.root, "sqlite_locked")
            .wait()
            .unwrap()
            .success());
    }
    db.execute_batch("ROLLBACK;").unwrap();
    db.close().unwrap();
    assert!(child(&fixture.root, "sqlite_writable")
        .wait()
        .unwrap()
        .success());
    drop(lease);
}

#[test]
fn database_metadata_checks_reject_replacement_links_and_unsafe_permissions() {
    for mode in [
        "replacement",
        "symlink",
        "hardlink",
        "permissions",
        "parent_symlink",
    ] {
        let fixture = Fixture::new();
        let h = handoff(&fixture.root);
        let lease = h.advance(&mut origin(&h)).unwrap();
        let path = lease.database_path();
        match mode {
            "replacement" | "symlink" => {
                let preserved = h.managed().join("preserved-instance");
                std::fs::rename(&path, &preserved).unwrap();
                if mode == "symlink" {
                    std::os::unix::fs::symlink(&preserved, &path).unwrap();
                } else {
                    std::fs::copy(&preserved, &path).unwrap();
                    private(&path);
                }
            }
            "hardlink" => std::fs::hard_link(&path, h.managed().join("alias")).unwrap(),
            "permissions" => {
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            }
            _ => {
                let preserved = fixture.root.join("preserved-domain");
                std::fs::rename(h.managed(), &preserved).unwrap();
                std::os::unix::fs::symlink(&preserved, h.managed()).unwrap();
            }
        }
        assert!(lease.assert_current().is_err(), "{mode}");
    }
}

#[test]
fn active_lease_blocks_reentry_and_status_before_callbacks_or_file_changes() {
    let fixture = Fixture::new();
    let h = handoff(&fixture.root);
    let lease = h.advance(&mut origin(&h)).unwrap();
    let journal_before = std::fs::read(fixture.root.join(JOURNAL)).unwrap();
    let database_before = std::fs::read(lease.database_path()).unwrap();
    let db = rusqlite::Connection::open(lease.database_path()).unwrap();
    db.execute_batch("BEGIN IMMEDIATE;").unwrap();
    let mut proof = origin(&h);
    proof.check = Box::new(|_, _| panic!("不应在服务锁准入前调用origin"));
    assert!(matches!(
        h.drive(&mut proof, &mut |_| panic!("锁忙时不应推进任何阶段")),
        Err(HandoffError::Lease(
            inputia_settings::memory_domain::LeaseError::Busy
        ))
    ));
    assert!(matches!(
        h.status(),
        Err(HandoffError::Lease(
            inputia_settings::memory_domain::LeaseError::Busy
        ))
    ));
    // 让子进程真正取得更新事务锁，再验证仍由本进程持有的域锁阻止它重入。
    drop(h);
    assert!(child(&fixture.root, "reenter_busy")
        .wait()
        .unwrap()
        .success());
    // status/重入不能open-close主库而释放本进程的SQLite事务锁。
    assert!(child(&fixture.root, "sqlite_locked")
        .wait()
        .unwrap()
        .success());
    db.execute_batch("ROLLBACK;").unwrap();
    db.close().unwrap();
    assert_eq!(
        journal_before,
        std::fs::read(fixture.root.join(JOURNAL)).unwrap()
    );
    assert_eq!(
        database_before,
        std::fs::read(lease.database_path()).unwrap()
    );
}

#[test]
fn service_lock_exists_before_database_and_transfers_without_reacquiring() {
    let fixture = Fixture::new();
    let h = handoff(&fixture.root);
    assert!(h
        .drive(&mut origin(&h), &mut |point| {
            if point == "service_sync" {
                Err(HandoffError::Busy)
            } else {
                Ok(())
            }
        })
        .is_err());
    let journal = h.load().unwrap().unwrap();
    assert_eq!(journal.phase, LockPhase::Bootstrap);
    assert!(journal.fence.sources.is_empty());
    assert!(!h.managed().join(DATABASE_NAME).exists());
    let service = h.acquire_service(&journal).unwrap();
    assert!(child(&fixture.root, "service_busy")
        .wait()
        .unwrap()
        .success());
    assert!(matches!(
        h.advance(&mut origin(&h)),
        Err(HandoffError::Lease(
            inputia_settings::memory_domain::LeaseError::Busy
        ))
    ));
    drop(service);
    drop(h.advance(&mut origin(&h)).unwrap());
    let journal = h.load().unwrap().unwrap();
    let service = h.acquire_service(&journal).unwrap();
    assert!(child(&fixture.root, "service_busy")
        .wait()
        .unwrap()
        .success());
    // 若内部重新flock独立OFD，这里同进程也会Busy；消费原锁对象必须直接成功。
    let lease = service.bind_database(&binding(&journal).unwrap()).unwrap();
    assert!(child(&fixture.root, "service_busy")
        .wait()
        .unwrap()
        .success());
    lease.assert_current().unwrap();
    drop(lease);
    h.acquire_service(&journal).unwrap();
}

#[test]
fn bootstrap_lock_held_by_another_process_prevents_source_capture_until_exit() {
    let fixture = Fixture::new();
    let h = handoff(&fixture.root);
    let _ = h.drive(&mut origin(&h), &mut |point| {
        if point == "service_sync" {
            Err(HandoffError::Busy)
        } else {
            Ok(())
        }
    });
    let before = std::fs::read(fixture.root.join(JOURNAL)).unwrap();
    let mut holder = child(&fixture.root, "hold_service");
    await_ready(&mut holder);
    let mut proof = origin(&h);
    proof.check = Box::new(|_, _| panic!("被其他进程持锁时不能检查来源"));
    assert!(matches!(
        h.advance(&mut proof),
        Err(HandoffError::Lease(
            inputia_settings::memory_domain::LeaseError::Busy
        ))
    ));
    assert_eq!(before, std::fs::read(fixture.root.join(JOURNAL)).unwrap());
    assert!(!h.managed().join(DATABASE_NAME).exists());
    holder.kill().unwrap();
    holder.wait().unwrap();
    drop(h);
    assert!(child(&fixture.root, "complete").wait().unwrap().success());
    assert_eq!(
        count(&fixture.root.join(DOMAIN_DIRECTORY).join(DATABASE_NAME)),
        7
    );
}

#[test]
fn unregistered_bootstrap_effects_remain_repair_after_real_process_crash() {
    for point in ["managed_created", "service_created"] {
        let fixture = Fixture::new();
        let mut process = child(&fixture.root, point);
        await_ready(&mut process);
        process.kill().unwrap();
        process.wait().unwrap();
        let h = handoff(&fixture.root);
        let before = std::fs::read(fixture.root.join(JOURNAL)).unwrap();
        let directory = identity(&h.managed(), h.uid(), true).unwrap();
        let lock = identity(&h.managed().join("service.lock"), h.uid(), false).unwrap();
        assert!(
            matches!(
                h.advance(&mut origin(&h)),
                Err(HandoffError::RepairRequired(_))
            ),
            "{point}"
        );
        assert_eq!(directory, identity(&h.managed(), h.uid(), true).unwrap());
        assert_eq!(
            lock,
            identity(&h.managed().join("service.lock"), h.uid(), false).unwrap()
        );
        assert_eq!(before, std::fs::read(fixture.root.join(JOURNAL)).unwrap());
        assert!(!h.managed().join(DATABASE_NAME).exists());
        assert_eq!(count(&fixture.root.join(OLD_DATABASE_NAME)), 7);
    }
}

#[test]
fn bootstrap_sync_failure_is_retried_before_binding_any_source() {
    let fixture = Fixture::new();
    let h = handoff(&fixture.root);
    for _ in 0..2 {
        let mut proof = origin(&h);
        proof.check = Box::new(|_, _| panic!("锁命名空间耐久性未知不能核来源"));
        assert!(h
            .drive(&mut proof, &mut |point| {
                if point == "service_sync" {
                    Err(std::io::Error::from_raw_os_error(libc::EIO).into())
                } else {
                    Ok(())
                }
            })
            .is_err());
        let journal = h.load().unwrap().unwrap();
        assert_eq!(journal.phase, LockPhase::Bootstrap);
        assert!(journal.fence.sources.is_empty() && !journal.ready);
    }
    assert_eq!(
        count(&h.advance(&mut origin(&h)).unwrap().database_path()),
        7
    );
}

#[test]
fn bootstrap_rejects_source_or_later_stage_evidence_instead_of_reinterpreting_it() {
    for mode in [
        "source",
        "archive",
        "fence",
        "fence_record",
        "workspace",
        "database",
        "frozen",
        "snapshot",
        "ready",
        "orphan_lock",
    ] {
        let fixture = Fixture::new();
        let h = handoff(&fixture.root);
        let mut journal = h.initialize().unwrap();
        let id = FileIdentity {
            device: 1,
            inode: 2,
        };
        match mode {
            "source" => journal.fence.sources.push(Source {
                name: OLD_DATABASE_NAME.into(),
                identity: Some(id),
            }),
            "archive" => journal.archive_dir = Some(id),
            "fence" => journal.staged_fence = Some(id),
            "fence_record" => journal.fence_record = Some(id),
            "workspace" => journal.workspace = Some(id),
            "database" => journal.staged_database = Some(id),
            "frozen" => journal.frozen_sources = Some(vec![]),
            "snapshot" => {
                journal.snapshot = Some(Content {
                    bytes: 0,
                    sha256: "a".repeat(64),
                })
            }
            "ready" => journal.ready = true,
            _ => journal.service_lock = Some(id),
        }
        h.save(&journal).unwrap();
        assert!(
            matches!(
                h.advance(&mut origin(&h)),
                Err(HandoffError::RepairRequired(_))
            ),
            "{mode}"
        );
        assert!(!h.managed().exists());
    }
}

#[test]
fn legacy_complete_journal_upgrades_only_with_registered_service_lock() {
    for missing_lock in [false, true] {
        let fixture = Fixture::new();
        let h = handoff(&fixture.root);
        drop(h.advance(&mut origin(&h)).unwrap());
        let mut legacy: serde_json::Value =
            serde_json::from_slice(&std::fs::read(fixture.root.join(JOURNAL)).unwrap()).unwrap();
        let object = legacy.as_object_mut().unwrap();
        object.remove("schema");
        object.remove("phase");
        if missing_lock {
            object.insert("service_lock".into(), serde_json::Value::Null);
        }
        std::fs::write(
            fixture.root.join(JOURNAL),
            serde_json::to_vec(&legacy).unwrap(),
        )
        .unwrap();
        let before = std::fs::read(fixture.root.join(JOURNAL)).unwrap();
        let lock = identity(&h.managed().join("service.lock"), h.uid(), false).unwrap();
        if missing_lock {
            assert!(matches!(
                h.advance(&mut origin(&h)),
                Err(HandoffError::RepairRequired(
                    "legacy_service_lock_evidence_missing"
                ))
            ));
            assert_eq!(before, std::fs::read(fixture.root.join(JOURNAL)).unwrap());
        } else {
            drop(h.advance(&mut origin(&h)).unwrap());
            let upgraded: serde_json::Value =
                serde_json::from_slice(&std::fs::read(fixture.root.join(JOURNAL)).unwrap())
                    .unwrap();
            assert_eq!(upgraded["schema"], 2);
            assert_eq!(upgraded["phase"], "bound");
        }
        assert_eq!(
            lock,
            identity(&h.managed().join("service.lock"), h.uid(), false).unwrap()
        );
    }
}

#[test]
fn replaced_lock_or_directory_is_not_repaired_by_reusing_child_inodes() {
    for mode in ["lock", "directory"] {
        let fixture = Fixture::new();
        let h = handoff(&fixture.root);
        let lease = h.advance(&mut origin(&h)).unwrap();
        let before = std::fs::read(fixture.root.join(JOURNAL)).unwrap();
        if mode == "lock" {
            std::fs::rename(
                h.managed().join("service.lock"),
                h.managed().join("preserved-lock"),
            )
            .unwrap();
            std::fs::write(h.managed().join("service.lock"), b"").unwrap();
            private(&h.managed().join("service.lock"));
        } else {
            let preserved = fixture.root.join("preserved-managed");
            std::fs::rename(h.managed(), &preserved).unwrap();
            std::fs::create_dir(h.managed()).unwrap();
            std::fs::set_permissions(h.managed(), std::fs::Permissions::from_mode(0o700)).unwrap();
            for name in ["service.lock", DATABASE_NAME] {
                std::fs::rename(preserved.join(name), h.managed().join(name)).unwrap();
            }
        }
        assert!(lease.assert_current().is_err(), "{mode}");
        let mut proof = origin(&h);
        proof.check = Box::new(|_, _| panic!("锁绑定已换不能核来源"));
        assert!(h.advance(&mut proof).is_err(), "{mode}");
        assert_eq!(before, std::fs::read(fixture.root.join(JOURNAL)).unwrap());
    }
}

#[test]
fn status_does_not_create_bootstrap_or_recreate_missing_operation_lock() {
    let fixture = Fixture::new();
    let h = handoff(&fixture.root);
    assert!(!h.status().unwrap().snapshot_published);
    assert!(!fixture.root.join(JOURNAL).exists());
    assert!(!fixture.root.join(".legacy-memory-handoff.lock").exists());
    assert!(!h.managed().exists());
    drop(h.advance(&mut origin(&h)).unwrap());
    std::fs::remove_file(fixture.root.join(".legacy-memory-handoff.lock")).unwrap();
    assert!(h.status().is_err());
    assert!(!fixture.root.join(".legacy-memory-handoff.lock").exists());
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
