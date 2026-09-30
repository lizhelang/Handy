use super::*;

struct Fixture {
    _temp: tempfile::TempDir,
    home: PathBuf,
    handy: PathBuf,
    inputia: PathBuf,
    backup: PathBuf,
    locks: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let home = fs::canonicalize(temp.path()).unwrap();
        let handy = home.join("Handy");
        let inputia = home.join("Inputia");
        fs::create_dir(&handy).unwrap();
        fs::create_dir(&inputia).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for path in [temp.path(), &handy, &inputia] {
                fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
            }
        }
        let backup = home.join("backups");
        let locks = home.join("locks");
        Self {
            _temp: temp,
            home,
            handy,
            inputia,
            backup,
            locks,
        }
    }
    fn run<F: FnOnce() -> Result<()>>(&self, preflight: F) -> Result<Option<StartupMigration>> {
        prepare_paths(
            &self.handy,
            Some(&self.inputia),
            &self.backup,
            &self.locks,
            None,
            preflight,
        )
    }
    fn prepare(&self) -> StartupMigration {
        self.run(|| Ok(())).unwrap().unwrap()
    }
    fn settings(&self) -> PathBuf {
        self.handy.join("settings_store.json")
    }
    fn marker(&self) -> PathBuf {
        self.handy
            .join(".inputia-control-settings-initialized.json")
    }
    fn put(&self, value: bool) {
        fs::write(self.settings(), valid_settings(value)).unwrap();
    }
    fn authorized_put(&self, guard: &mut StartupMigration, value: bool) {
        let document = valid_settings(value);
        guard
            .record_settings_initialization(&self.intent(&document, b"authorized marker", true))
            .unwrap();
        fs::write(self.settings(), document).unwrap();
    }
    fn read(&self) -> Vec<u8> {
        fs::read(self.settings()).unwrap()
    }
    fn journal(&self) -> Journal {
        read_json(&self.backup.join(JOURNAL), JOURNAL_LIMIT).unwrap()
    }
    fn strict(&self) -> Result<()> {
        // 使用生产控制中心领域 schema 的只读预检，无默认值写回。
        #[cfg(unix)]
        let uid = unsafe { libc::geteuid() };
        #[cfg(not(unix))]
        let uid = 0;
        crate::settings::runtime::RuntimeSettings::preflight(&self.settings(), &self.home, uid)
            .map_err(Into::into)
    }
    fn intent(&self, document: &[u8], marker: &[u8], write: bool) -> InitializationIntent {
        InitializationIntent {
            domain: "inputia.control-settings".into(),
            file_name: "settings_store.json".into(),
            marker_name: ".inputia-control-settings-initialized.json".into(),
            store_id: "11111111-1111-4111-8111-111111111111".into(),
            original_document_sha256: fs::read(self.settings()).ok().map(|v| sha(&v)),
            original_document_size: fs::read(self.settings()).ok().map(|v| v.len() as u64),
            document_size: document.len() as u64,
            marker_size: marker.len() as u64,
            document_sha256: sha(document),
            marker_sha256: sha(marker),
            will_write_document: write,
            will_create_marker: true,
        }
    }
}
fn valid_settings(debug: bool) -> Vec<u8> {
    let mut settings = crate::settings::get_default_settings();
    settings.debug_mode = debug;
    serde_json::to_vec(&serde_json::json!({"settings":settings})).unwrap()
}
fn inject(point: &'static str) {
    FAIL.with(|v| *v.borrow_mut() = Some(point));
}
fn reset() {
    FAIL.with(|v| *v.borrow_mut() = None);
}
fn kind(error: &anyhow::Error) -> StartupFailureKind {
    error.downcast_ref::<StartupFailure>().unwrap().kind
}

#[test]
fn bad_preflight_then_manual_repair_is_never_restored() {
    let f = Fixture::new();
    fs::write(f.settings(), b"{bad-json").unwrap();
    assert_eq!(
        kind(&f.run(|| f.strict()).err().unwrap()),
        StartupFailureKind::CleanPreflight
    );
    assert!(!f.backup.exists());
    f.put(true);
    let expected = f.read();
    let guard = f.run(|| f.strict()).unwrap().unwrap();
    drop(guard);
    assert_eq!(f.read(), expected);
    assert_eq!(f.journal().phase, Phase::Prepared);
}
#[test]
fn prepared_crash_does_not_restore_user_repair_or_allow_complete() {
    let f = Fixture::new();
    f.put(false);
    let mut guard = f.prepare();
    assert!(guard.complete().is_err());
    drop(guard);
    f.put(true);
    let expected = f.read();
    let guard = f.run(|| f.strict()).unwrap().unwrap();
    drop(guard);
    assert_eq!(f.read(), expected);
}
#[test]
fn source_changed_after_backup_cannot_arm() {
    let f = Fixture::new();
    f.put(false);
    let mut guard = f.prepare();
    f.put(true);
    assert!(guard.begin_mutations().is_err());
    assert_eq!(f.journal().phase, Phase::Prepared);
}
#[test]
fn mutating_failure_restores_database_then_recovered_repair_survives() {
    let f = Fixture::new();
    f.put(false);
    let db = f.handy.join("history.db");
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch("CREATE TABLE items(value TEXT);INSERT INTO items VALUES('old');")
        .unwrap();
    drop(conn);
    let mut guard = f.prepare();
    let manifest = fs::read(&guard.outcome.manifest_path).unwrap();
    guard.begin_mutations().unwrap();
    f.authorized_put(&mut guard, true);
    let conn = Connection::open(&db).unwrap();
    conn.execute_batch("UPDATE items SET value='new';").unwrap();
    drop(conn);
    drop(guard);
    assert_eq!(
        kind(
            &f.run(|| anyhow::bail!("forced strict failure after restore"))
                .err()
                .unwrap()
        ),
        StartupFailureKind::CleanPreflight
    );
    assert_eq!(f.journal().phase, Phase::Recovered);
    assert_eq!(f.read(), valid_settings(false));
    assert_eq!(
        fs::read(f.backup.join(f.journal().manifest_relative)).unwrap(),
        manifest
    );
    assert_eq!(
        Connection::open(&db)
            .unwrap()
            .query_row("SELECT value FROM items", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "old"
    );
    f.put(true);
    let guard = f.run(|| f.strict()).unwrap().unwrap();
    drop(guard);
    assert_eq!(f.read(), valid_settings(true));
}
#[test]
fn recovery_crash_boundaries_are_idempotent_and_recovered_never_replays() {
    for point in ["restoring", "restored", "recovered"] {
        let f = Fixture::new();
        f.put(false);
        let mut guard = f.prepare();
        guard.begin_mutations().unwrap();
        f.authorized_put(&mut guard, true);
        drop(guard);
        inject(point);
        assert!(f.run(|| Ok(())).is_err());
        reset();
        if point == "recovered" {
            f.put(true);
        } // 只有耐久 Recovered 后的手修才合法。
        let guard = f.prepare();
        drop(guard);
        assert_eq!(f.read(), valid_settings(point == "recovered"), "{point}");
    }
}
#[test]
fn next_backup_crash_keeps_previous_recovered_terminal() {
    let f = Fixture::new();
    f.put(false);
    let mut guard = f.prepare();
    guard.begin_mutations().unwrap();
    f.authorized_put(&mut guard, true);
    drop(guard);
    assert!(f.run(|| anyhow::bail!("pause after recovery")).is_err());
    let old = f.journal().attempt_id;
    f.put(true);
    inject("backup_created");
    assert!(f.run(|| Ok(())).is_err());
    reset();
    assert_eq!(f.journal().attempt_id, old);
    assert_eq!(f.journal().phase, Phase::Recovered);
    let guard = f.prepare();
    drop(guard);
    assert_eq!(f.read(), valid_settings(true));
}
#[test]
fn exact_manifest_is_used_not_a_newer_directory() {
    let f = Fixture::new();
    f.put(false);
    let mut guard = f.prepare();
    guard.begin_mutations().unwrap();
    f.authorized_put(&mut guard, true);
    drop(guard);
    fs::create_dir_all(f.backup.join("handy-data-zzzz")).unwrap();
    fs::write(
        f.backup.join("handy-data-zzzz/manifest.json"),
        b"not selected",
    )
    .unwrap();
    let guard = f.prepare();
    drop(guard);
    assert_eq!(f.read(), valid_settings(false));
}
#[test]
fn legacy_without_attempt_requires_explicit_recovery() {
    let f = Fixture::new();
    f.put(true);
    fs::create_dir_all(f.backup.join("handy-data-old")).unwrap();
    assert_eq!(
        kind(&f.run(|| Ok(())).err().unwrap()),
        StartupFailureKind::LegacyRecoveryRequired
    );
    assert_eq!(f.read(), valid_settings(true));
}
#[test]
fn unknown_new_marker_is_preserved_before_any_other_restore() {
    let f = Fixture::new();
    f.put(false);
    let mut guard = f.prepare();
    guard.begin_mutations().unwrap();
    f.put(true);
    fs::write(f.marker(), b"unknown marker").unwrap();
    drop(guard);
    assert_eq!(
        kind(&f.run(|| Ok(())).err().unwrap()),
        StartupFailureKind::RepairRequired
    );
    assert_eq!(f.read(), valid_settings(true));
    assert_eq!(fs::read(f.marker()).unwrap(), b"unknown marker");
}
#[test]
fn authorized_new_pair_and_document_only_crash_are_quarantined_not_deleted() {
    for marker_created in [false, true] {
        let f = Fixture::new();
        let mut guard = f.prepare();
        guard.begin_mutations().unwrap();
        let document = valid_settings(false);
        let marker = b"synthetic marker";
        let intent = f.intent(&document, marker, true);
        guard.record_settings_initialization(&intent).unwrap();
        let attempt = guard.journal.attempt_id.clone();
        fs::write(f.settings(), &document).unwrap();
        if marker_created {
            fs::write(f.marker(), marker).unwrap();
        }
        drop(guard);
        let guard = f.prepare();
        drop(guard);
        assert!(!f.settings().exists());
        assert!(!f.marker().exists());
        assert_eq!(
            fs::read(f.handy.join(format!(
                "migration_restore_quarantine/startup-{attempt}/settings_store.json"
            )))
            .unwrap(),
            document
        );
    }
}
#[test]
fn marker_only_initialization_does_not_claim_existing_document() {
    let f = Fixture::new();
    f.put(false);
    let original = f.read();
    let mut guard = f.prepare();
    guard.begin_mutations().unwrap();
    guard
        .record_settings_initialization(&f.intent(&original, b"new marker", false))
        .unwrap();
    fs::write(f.marker(), b"new marker").unwrap();
    f.put(true);
    drop(guard);
    assert_eq!(
        kind(&f.run(|| Ok(())).err().unwrap()),
        StartupFailureKind::RepairRequired
    );
    assert_eq!(f.read(), valid_settings(true));
    assert_eq!(fs::read(f.marker()).unwrap(), b"new marker");
}
#[test]
fn existing_pair_unknown_changes_are_preserved_for_repair() {
    let f = Fixture::new();
    f.put(false);
    fs::write(f.marker(), b"old marker").unwrap();
    let mut guard = f.prepare();
    guard.begin_mutations().unwrap();
    f.put(true);
    fs::write(f.marker(), b"changed marker").unwrap();
    drop(guard);
    assert_eq!(
        kind(&f.run(|| Ok(())).err().unwrap()),
        StartupFailureKind::RepairRequired
    );
    assert_eq!(f.read(), valid_settings(true));
    assert_eq!(fs::read(f.marker()).unwrap(), b"changed marker");
}
#[test]
fn creation_intent_must_match_baseline_and_be_durable_before_source_write() {
    let f = Fixture::new();
    let mut guard = f.prepare();
    let intent = f.intent(&valid_settings(false), b"marker", true);
    assert!(guard.record_settings_initialization(&intent).is_err());
    guard.begin_mutations().unwrap();
    let mut wrong = intent.clone();
    wrong.domain = "foreign".into();
    assert!(guard.record_settings_initialization(&wrong).is_err());
    inject("creation_authorized");
    assert!(guard.record_settings_initialization(&intent).is_err());
    reset();
    drop(guard);
    assert_eq!(f.journal().creations.len(), 1);
    assert!(!f.settings().exists());
    let guard = f.prepare();
    drop(guard);
    assert!(!f.settings().exists());
}
#[test]
fn mismatched_created_document_and_missing_intent_fail_closed() {
    for record in [false, true] {
        let f = Fixture::new();
        let mut guard = f.prepare();
        guard.begin_mutations().unwrap();
        if record {
            guard
                .record_settings_initialization(&f.intent(&valid_settings(false), b"marker", true))
                .unwrap();
        }
        f.put(true);
        drop(guard);
        assert!(f.run(|| Ok(())).is_err());
        assert_eq!(f.read(), valid_settings(true));
    }
}
#[test]
fn quarantine_boundary_resume_preserves_original_new_bytes() {
    let f = Fixture::new();
    let mut guard = f.prepare();
    guard.begin_mutations().unwrap();
    let document = valid_settings(false);
    guard
        .record_settings_initialization(&f.intent(&document, b"marker", true))
        .unwrap();
    f.put(false);
    fs::write(f.marker(), b"marker").unwrap();
    drop(guard);
    inject("quarantined");
    assert!(f.run(|| Ok(())).is_err());
    reset();
    let guard = f.prepare();
    drop(guard);
    assert!(!f.settings().exists());
    assert!(!f.marker().exists());
}
#[test]
fn changed_manifest_and_concurrent_start_are_rejected() {
    let f = Fixture::new();
    f.put(false);
    let guard = f.prepare();
    assert!(f.run(|| Ok(())).is_err());
    let manifest = guard.outcome.manifest_path.clone();
    drop(guard);
    fs::write(manifest, b"{}").unwrap();
    assert_eq!(
        kind(&f.run(|| Ok(())).err().unwrap()),
        StartupFailureKind::RepairRequired
    );
}
#[test]
fn complete_is_only_durable_after_mutating_and_still_runs_preflight() {
    let f = Fixture::new();
    f.put(false);
    let mut guard = f.prepare();
    guard.begin_mutations().unwrap();
    guard.complete().unwrap();
    drop(guard);
    assert_eq!(f.journal().phase, Phase::Completed);
    assert!(f.run(|| Ok(())).unwrap().is_none());
    assert_eq!(
        kind(&f.run(|| anyhow::bail!("bad later settings")).err().unwrap()),
        StartupFailureKind::CleanPreflight
    );
}

#[test]
fn journal_rename_sync_windows_never_restore_unarmed_manual_repair() {
    for point in [
        "journal_before_rename",
        "journal_after_rename",
        "journal_after_sync",
    ] {
        let f = Fixture::new();
        f.put(false);
        let mut guard = f.prepare();
        inject(point);
        assert!(guard.begin_mutations().is_err());
        reset();
        drop(guard);
        let phase = f.journal().phase;
        // Arm 返回失败后没有写入；仅仍 Prepared 时允许下一次纯预检接收用户修复。
        if phase == Phase::Prepared {
            f.put(true);
        }
        let guard = f.prepare();
        drop(guard);
        assert_eq!(
            f.read(),
            valid_settings(phase == Phase::Prepared),
            "{point}"
        );
    }
}
#[test]
fn recovered_journal_rename_sync_windows_preserve_repair_only_after_terminal() {
    for point in [
        "journal_before_rename",
        "journal_after_rename",
        "journal_after_sync",
    ] {
        let f = Fixture::new();
        f.put(false);
        let mut guard = f.prepare();
        guard.begin_mutations().unwrap();
        f.put(true);
        // 模拟恢复已经完成，接着在 Recovered 日志的不同 I/O 边界崩溃。
        guard.journal.phase = Phase::Restoring;
        guard.persist().unwrap();
        restore_backup_inner(
            &guard.outcome,
            &guard.source_roots,
            SqliteRestorePolicy::ConsistentSnapshot,
            false,
        )
        .unwrap();
        guard.journal.phase = Phase::Recovered;
        inject(point);
        assert!(guard.persist().is_err());
        reset();
        drop(guard);
        let terminal = f.journal().phase == Phase::Recovered;
        if terminal {
            f.put(true);
        }
        let guard = f.prepare();
        drop(guard);
        assert_eq!(f.read(), valid_settings(terminal), "{point}");
    }
}
#[test]
fn real_settings_observer_creates_exact_owned_pair_before_manager_failure() {
    let f = Fixture::new();
    let mut guard = f.run(|| f.strict()).unwrap().unwrap();
    guard.begin_mutations().unwrap();
    let runtime = crate::settings::runtime::RuntimeSettings::default();
    #[cfg(unix)]
    let uid = unsafe { libc::geteuid() };
    #[cfg(not(unix))]
    let uid = 0;
    runtime
        .initialize(&f.settings(), &f.home, uid, &mut |intent| {
            guard
                .record_settings_initialization(intent)
                .map_err(|_| inputia_settings::store::Error::StorageUnavailable)
        })
        .unwrap();
    assert!(f.settings().exists());
    assert!(f.marker().exists());
    assert_eq!(f.journal().creations.len(), 1);
    let created = f.read();
    drop(runtime);
    drop(guard);
    let guard = f.run(|| f.strict()).unwrap().unwrap();
    drop(guard);
    assert!(!f.settings().exists());
    assert!(!f.marker().exists());
    let recovered = fs::read_dir(f.handy.join("migration_restore_quarantine"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(
        fs::read(recovered.join("settings_store.json")).unwrap(),
        created
    );
}

#[test]
fn completed_uses_manifest_binding_without_reading_old_payload() {
    let f = Fixture::new();
    f.put(false);
    fs::create_dir(f.handy.join("models")).unwrap();
    fs::write(f.handy.join("models/model.bin"), b"synthetic model").unwrap();
    let mut guard = f.prepare();
    guard.begin_mutations().unwrap();
    guard.complete().unwrap();
    let backup = guard.outcome.backup_dir.clone();
    drop(guard);
    fs::remove_file(backup.join("handy/models/model.bin")).unwrap();
    fs::remove_file(backup.join("handy/settings_store.json")).unwrap();
    assert!(f.run(|| f.strict()).unwrap().is_none());
}
#[test]
fn armed_startup_still_requires_every_backup_payload_before_restore() {
    let f = Fixture::new();
    f.put(false);
    let mut guard = f.prepare();
    guard.begin_mutations().unwrap();
    let backup = guard.outcome.backup_dir.clone();
    f.authorized_put(&mut guard, true);
    drop(guard);
    fs::remove_file(backup.join("handy/settings_store.json")).unwrap();
    assert_eq!(
        kind(&f.run(|| Ok(())).err().unwrap()),
        StartupFailureKind::PendingRecovery
    );
    assert_eq!(f.read(), valid_settings(true));
}

#[test]
fn every_quarantine_sync_window_rechecks_and_confirms_exact_destination() {
    for point in [
        "quarantine_after_rename",
        "quarantine_before_file_sync",
        "quarantine_after_file_sync",
        "quarantine_before_source_sync",
        "quarantine_after_source_sync",
        "quarantine_before_target_sync",
        "quarantine_after_target_sync",
        "quarantine_before_ancestor_sync",
        "quarantine_after_ancestor_sync",
    ] {
        let f = Fixture::new();
        let mut guard = f.prepare();
        guard.begin_mutations().unwrap();
        let document = valid_settings(false);
        guard
            .record_settings_initialization(&f.intent(&document, b"marker", true))
            .unwrap();
        f.put(false);
        drop(guard);
        inject(point);
        assert!(f.run(|| Ok(())).is_err(), "{point}");
        reset();
        assert!(!f.settings().exists());
        assert_eq!(f.journal().phase, Phase::Restoring);
        // 再次失败证明源 Absent 不会跳过隔离文件的耐久确认。
        inject("quarantine_before_file_sync");
        assert!(f.run(|| Ok(())).is_err(), "{point}");
        reset();
        let guard = f.prepare();
        drop(guard);
        assert!(!f.settings().exists());
    }
}
#[test]
fn changed_quarantine_after_rename_failure_is_preserved_for_repair() {
    let f = Fixture::new();
    let mut guard = f.prepare();
    guard.begin_mutations().unwrap();
    let document = valid_settings(false);
    guard
        .record_settings_initialization(&f.intent(&document, b"marker", true))
        .unwrap();
    let id = guard.journal.attempt_id.clone();
    f.put(false);
    drop(guard);
    inject("quarantine_after_rename");
    assert!(f.run(|| Ok(())).is_err());
    reset();
    let target = f.handy.join(format!(
        "migration_restore_quarantine/startup-{id}/settings_store.json"
    ));
    fs::write(&target, b"unknown new quarantine bytes").unwrap();
    assert!(f.run(|| Ok(())).is_err());
    assert_eq!(fs::read(target).unwrap(), b"unknown new quarantine bytes");
    assert_eq!(f.journal().phase, Phase::Restoring);
}

#[test]
fn every_legacy_phase_preserves_unknown_pending_instead_of_restoring_a_partial_domain() {
    for phase in [
        None,
        Some(Phase::Prepared),
        Some(Phase::Mutating),
        Some(Phase::Restoring),
        Some(Phase::Recovered),
        Some(Phase::Completed),
    ] {
        let f = Fixture::new();
        f.put(false);
        if let Some(phase) = phase {
            let mut guard = f.prepare();
            guard.journal.phase = phase;
            guard.persist().unwrap();
            drop(guard);
        }
        f.put(true);
        let pending = f.handy.join(CONTROL_SETTINGS_PENDING_NAME);
        fs::write(&pending, b"unknown pending source must be preserved").unwrap();
        let original = f.read();
        let original_pending = fs::read(&pending).unwrap();
        let original_journal = fs::read(f.backup.join(JOURNAL)).ok();
        let error = f
            .run(|| panic!("must reject before preflight or restore"))
            .err()
            .unwrap();
        assert_eq!(kind(&error), StartupFailureKind::RepairRequired);
        assert_eq!(f.read(), original);
        assert_eq!(fs::read(&pending).unwrap(), original_pending);
        assert_eq!(fs::read(f.backup.join(JOURNAL)).ok(), original_journal);
        assert!(!f.handy.join("migration_restore_quarantine").exists());
    }
}

#[test]
fn late_unknown_pending_revokes_legacy_mutation_and_completion() {
    for when in ["before_arm", "before_initialize", "before_complete"] {
        let f = Fixture::new();
        f.put(false);
        let mut guard = f.prepare();
        if when != "before_arm" {
            guard.begin_mutations().unwrap();
        }
        let pending = f.handy.join(CONTROL_SETTINGS_PENDING_NAME);
        fs::write(&pending, b"preserve").unwrap();
        let original = f.read();
        let original_journal = fs::read(f.backup.join(JOURNAL)).unwrap();
        match when {
            "before_arm" => assert!(guard.begin_mutations().is_err()),
            "before_initialize" => assert!(guard
                .record_settings_initialization(&f.intent(b"document", b"marker", true))
                .is_err()),
            _ => assert!(guard.complete().is_err()),
        }
        assert_eq!(f.read(), original);
        assert_eq!(fs::read(&pending).unwrap(), b"preserve");
        assert_eq!(fs::read(f.backup.join(JOURNAL)).unwrap(), original_journal);
    }
}
