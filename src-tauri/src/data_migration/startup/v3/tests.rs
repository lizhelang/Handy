use super::*;
use inputia_settings::store::{
    ApplyResult, DocumentSchema, DocumentStore, Error, PatchRequest, PendingStatus, Snapshot,
};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
struct Legacy;
struct Pending;
macro_rules! schema {($t:ty,$pending:expr)=>{impl DocumentSchema for $t {
    const FILE_NAME:&'static str="settings_store.json";
    const MARKER_NAME:&'static str=".inputia-control-settings-initialized.json";
    const DOMAIN:&'static str="inputia.control-settings";
    const PENDING_NAME:Option<&'static str>=$pending;
    fn defaults(_: &Path)->inputia_settings::store::Result<Map<String,Value>> {Ok(json!({"flag":false}).as_object().unwrap().clone())}
    fn validate(values:&Map<String,Value>,_:&Path,_:bool)->inputia_settings::store::Result<Map<String,Value>> {
        if !values.get("flag").is_some_and(Value::is_boolean){return Err(Error::InvalidDocument);} Ok(values.clone())
    }
}};}
schema!(Legacy, None);
schema!(Pending, Some(CONTROL_SETTINGS_PENDING_NAME));
struct Fixture {
    _temp: tempfile::TempDir,
    home: PathBuf,
    root: PathBuf,
    backup: PathBuf,
    locks: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let home = fs::canonicalize(temp.path()).unwrap();
        let root = home.join("control");
        fs::create_dir(&root).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for path in [&home, &root] {
                fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
            }
        }
        Self {
            backup: home.join("backups"),
            locks: home.join("locks"),
            _temp: temp,
            home,
            root,
        }
    }
    fn store<S: DocumentSchema>(&self) -> DocumentStore<S> {
        DocumentStore::<S>::open(&self.root.join(Legacy::FILE_NAME), &self.home, unsafe {
            libc::geteuid()
        })
        .unwrap()
    }
    fn prepare(&self, need: StartupNeed) -> Result<StartupPreparation> {
        prepare_with_state(&self.root, None, &self.backup, &self.locks, None, || {
            Ok(need)
        })
    }
    fn attempt(&self, need: StartupNeed) -> StartupTransaction {
        match self.prepare(need).unwrap() {
            StartupPreparation::Attempt(t) => t,
            _ => panic!("expected attempt"),
        }
    }
    fn capture(&self, raw: &mut BTreeMap<String, Vec<u8>>) {
        for name in [
            Legacy::FILE_NAME,
            Legacy::MARKER_NAME,
            CONTROL_SETTINGS_PENDING_NAME,
        ] {
            if let Ok(bytes) = fs::read(self.root.join(name)) {
                raw.insert(sha(&bytes), bytes);
            }
        }
    }
    fn rewrite(&self, state: &[Option<FileStamp>], raw: &BTreeMap<String, Vec<u8>>) {
        for (name, digest) in [
            Legacy::FILE_NAME,
            Legacy::MARKER_NAME,
            CONTROL_SETTINGS_PENDING_NAME,
        ]
        .iter()
        .zip(state)
        {
            let path = self.root.join(name);
            match digest {
                Some(d) => fs::write(path, &raw[&d.sha256]).unwrap(),
                None => {
                    if path.exists() {
                        fs::remove_file(path).unwrap();
                    }
                }
            }
        }
    }
    fn legacy_full(&self) {
        let mut t = self.attempt(StartupNeed::FullStartup);
        t.begin_mutations().unwrap();
        self.store::<Legacy>()
            .read_observed_initialization(&mut |i| {
                t.record_settings_initialization(i).unwrap();
                Ok(())
            })
            .unwrap();
        t.complete(StartupConfirmation::FullStartupReady, || Ok(()))
            .unwrap();
    }
    fn activate(&self) -> String {
        let mut t = self.attempt(StartupNeed::SettingsProtocol);
        assert_eq!(t.purpose(), StartupPurpose::SettingsProtocol);
        t.begin_mutations().unwrap();
        self.store::<Pending>()
            .activate_pending_protocol(&mut |i| {
                t.record_protocol_activation(i).unwrap();
                Ok(())
            })
            .unwrap();
        let id = t.journal.full_completion.as_ref().unwrap().sha256.clone();
        t.complete(StartupConfirmation::ProtocolReady, || Ok(()))
            .unwrap();
        id
    }
    fn active_request(&self) -> String {
        let store = self.store::<Pending>();
        let snapshot = store.read().unwrap();
        let request = request(&snapshot, true);
        assert!(store
            .apply_observed(&request, &snapshot, &mut |i| if i.phase
                == inputia_settings::store::TransitionPhase::CommitDocument
            {
                Err(Error::StorageUnavailable)
            } else {
                Ok(())
            })
            .is_err());
        assert!(matches!(
            store.pending_status().unwrap(),
            PendingStatus::Active { .. }
        ));
        request.operation_id
    }
}
fn request(snapshot: &Snapshot, value: bool) -> PatchRequest {
    PatchRequest {
        operation_id: snapshot.operation_id(),
        expected_store_id: snapshot.store_id.clone(),
        expected_revision: snapshot.revision.clone(),
        patch: BTreeMap::from([("flag".into(), json!(value))]),
    }
}
fn full_trace(f: &Fixture) -> (StartupTransaction, BTreeMap<String, Vec<u8>>) {
    let mut t = f.attempt(StartupNeed::FullStartup);
    t.begin_mutations().unwrap();
    let mut raw = BTreeMap::new();
    f.store::<Legacy>()
        .read_observed_initialization(&mut |i| {
            t.record_settings_initialization(i).unwrap();
            Ok(())
        })
        .unwrap();
    f.capture(&mut raw);
    let store = f.store::<Pending>();
    let mut snapshot = store
        .activate_pending_protocol(&mut |i| {
            t.record_protocol_activation(i).unwrap();
            Ok(())
        })
        .unwrap();
    f.capture(&mut raw);
    for value in [true, false] {
        let request = request(&snapshot, value);
        let result = store
            .apply_observed(&request, &snapshot, &mut |i| {
                f.capture(&mut raw);
                t.record_settings_transition(i).unwrap();
                Ok(())
            })
            .unwrap();
        f.capture(&mut raw);
        snapshot = match result {
            ApplyResult::Saved { current, .. } => current,
            _ => panic!("expected saved"),
        };
    }
    (t, raw)
}
#[test]
fn all_real_initialization_activation_and_patch_replace_prefixes_recover_absent_baseline() {
    let sample = Fixture::new();
    let (sample_tx, _) = full_trace(&sample);
    let count = prefixes(&sample_tx.journal).unwrap().len();
    drop(sample_tx);
    assert_eq!(count, 12);
    for prefix in 0..count {
        let f = Fixture::new();
        let (t, raw) = full_trace(&f);
        let state = prefixes(&t.journal).unwrap()[prefix].clone();
        f.rewrite(&state, &raw);
        drop(t);
        let recovered = f.attempt(StartupNeed::FullStartup);
        drop(recovered);
        for name in [
            Legacy::FILE_NAME,
            Legacy::MARKER_NAME,
            CONTROL_SETTINGS_PENDING_NAME,
        ] {
            assert!(!f.root.join(name).exists(), "prefix={prefix} member={name}");
        }
    }
}
#[test]
fn historical_hash_mixture_is_not_a_legal_interruption_and_is_preserved() {
    let f = Fixture::new();
    let (t, raw) = full_trace(&f);
    let states = prefixes(&t.journal).unwrap();
    let mut mixed = states.last().unwrap().clone();
    mixed[2] = states[6][2].clone();
    assert!(!states.contains(&mixed));
    f.rewrite(&mixed, &raw);
    drop(t);
    assert!(f.prepare(StartupNeed::FullStartup).is_err());
    for (name, value) in [
        Legacy::FILE_NAME,
        Legacy::MARKER_NAME,
        CONTROL_SETTINGS_PENDING_NAME,
    ]
    .iter()
    .zip(&mixed)
    {
        assert_eq!(digest(&observe(&f.root.join(name)).unwrap()), *value);
    }
    assert!(!f.root.join("migration_restore_quarantine").exists());
}
#[test]
fn observer_recorded_before_source_write_retries_same_sequence_without_branch() {
    let f = Fixture::new();
    let mut t = f.attempt(StartupNeed::FullStartup);
    t.begin_mutations().unwrap();
    let store = f.store::<Legacy>();
    let mut captured = None;
    assert!(store
        .read_observed_initialization(&mut |i| {
            t.record_settings_initialization(i).unwrap();
            captured = Some(i.clone());
            Err(Error::StorageUnavailable)
        })
        .is_err());
    let intent = captured.unwrap();
    assert_eq!(t.journal.authorizations.len(), 1);
    t.record_settings_initialization(&intent).unwrap();
    assert_eq!(t.journal.authorizations.len(), 1);
    let mut wrong = intent.clone();
    wrong.store_id = uuid::Uuid::new_v4().to_string();
    assert!(t.record_settings_initialization(&wrong).is_err());
    wrong = intent;
    wrong.original_document_size = Some(1);
    assert!(t.record_settings_initialization(&wrong).is_err());
}
#[test]
fn versioned_document_missing_marker_can_only_repair_same_store_without_rewriting_document() {
    let f = Fixture::new();
    f.store::<Legacy>().read().unwrap();
    fs::remove_file(f.root.join(Legacy::MARKER_NAME)).unwrap();
    let document = fs::read(f.root.join(Legacy::FILE_NAME)).unwrap();
    let mut t = f.attempt(StartupNeed::FullStartup);
    t.begin_mutations().unwrap();
    f.store::<Legacy>()
        .read_observed_initialization(&mut |i| {
            assert!(!i.will_write_document);
            let mut wrong = i.clone();
            wrong.store_id = uuid::Uuid::new_v4().to_string();
            assert!(t.record_settings_initialization(&wrong).is_err());
            t.record_settings_initialization(i).unwrap();
            Ok(())
        })
        .unwrap();
    assert_eq!(fs::read(f.root.join(Legacy::FILE_NAME)).unwrap(), document);
    drop(t);
    drop(f.attempt(StartupNeed::FullStartup));
    assert_eq!(fs::read(f.root.join(Legacy::FILE_NAME)).unwrap(), document);
    assert!(!f.root.join(Legacy::MARKER_NAME).exists());
}
#[test]
fn wrong_phase_file_identity_and_sequence_are_rejected() {
    let f = Fixture::new();
    let (mut t, _) = full_trace(&f);
    let old_commit = t.journal.authorizations[3].clone();
    assert!(t.record(old_commit).is_err());
    for alteration in 0..5 {
        let mut j = t.journal.clone();
        match alteration {
            0 => j.authorizations[0].sequence = 2,
            1 => j.authorizations[2].changes[0].member = 1,
            2 => {
                if let Owner::Transition { store_id, .. } = &mut j.authorizations[2].owner {
                    *store_id = uuid::Uuid::new_v4().to_string();
                }
            }
            3 => {
                if let Owner::Transition { phase, .. } = &mut j.authorizations[3].owner {
                    *phase = StepPhase::PrepareRequest;
                }
            }
            _ => {
                j.authorizations[3].changes[0].before =
                    j.authorizations[2].changes[0].before.clone()
            }
        }
        assert!(prefixes(&j).is_err(), "alteration={alteration}");
    }
}
#[test]
fn small_replay_never_restores_history_and_keeps_full_completion_evidence() {
    let f = Fixture::new();
    f.legacy_full();
    let full = f.activate();
    fs::create_dir(f.root.join("recordings")).unwrap();
    fs::write(f.root.join("recordings/user.wav"), b"untouched").unwrap();
    for value in [b"new-one".as_slice(), b"new-two".as_slice()] {
        let _id = f.active_request();
        let mut t = f.attempt(StartupNeed::SettingsReplay);
        assert_eq!(t.journal.full_completion.as_ref().unwrap().sha256, full);
        assert_eq!(t.outcome.manifest.entries.len(), 3);
        t.begin_mutations().unwrap();
        let store = f.store::<Pending>();
        store
            .reconcile_pending_observed(&mut |i| {
                t.record_settings_transition(i).unwrap();
                Ok(())
            })
            .unwrap();
        drop(store);
        fs::write(f.root.join("recordings/user.wav"), value).unwrap();
        drop(t);
        let mut recovery = f.attempt(StartupNeed::SettingsReplay);
        assert_eq!(fs::read(f.root.join("recordings/user.wav")).unwrap(), value);
        assert_eq!(
            recovery.journal.full_completion.as_ref().unwrap().sha256,
            full
        );
        recovery.begin_mutations().unwrap();
        let store = f.store::<Pending>();
        let operation = match store.pending_status().unwrap() {
            PendingStatus::Active { operation_id } => operation_id,
            _ => panic!("restored original active"),
        };
        store
            .reconcile_pending_observed(&mut |i| {
                recovery.record_settings_transition(i).unwrap();
                Ok(())
            })
            .unwrap();
        drop(store);
        recovery
            .complete(
                StartupConfirmation::RequestSaved {
                    operation_id: operation,
                },
                || Ok(()),
            )
            .unwrap();
        drop(recovery);
    }
    let StartupPreparation::NoWork(observation) = f.prepare(StartupNeed::NoWork).unwrap() else {
        panic!("expected no work");
    };
    let roots = vec![MigrationSourceRoot {
        label: "handy".into(),
        root: f.root.clone(),
    }];
    observation.recheck(&roots).unwrap();
    let path = f.root.join(CONTROL_SETTINGS_PENDING_NAME);
    fs::write(path, b"externally changed").unwrap();
    assert!(observation.recheck(&roots).is_err());
}
#[test]
fn quarantine_and_cursor_durability_boundaries_are_reentrant() {
    for point in [
        "v3_restoring",
        "v3_quarantine_renamed",
        "v3_quarantine_file_synced",
        "v3_quarantine_source_synced",
        "v3_quarantine_target_synced",
        "v3_quarantine_ancestor_synced",
        "v3_restore_cursor",
        "v3_restored",
        "v3_recovered",
    ] {
        let f = Fixture::new();
        let (t, _) = full_trace(&f);
        drop(t);
        FAIL.with(|v| *v.borrow_mut() = Some(point));
        assert!(f.prepare(StartupNeed::FullStartup).is_err(), "{point}");
        FAIL.with(|v| *v.borrow_mut() = None);
        drop(f.attempt(StartupNeed::FullStartup));
        for name in [
            Legacy::FILE_NAME,
            Legacy::MARKER_NAME,
            CONTROL_SETTINGS_PENDING_NAME,
        ] {
            assert!(!f.root.join(name).exists(), "{point}");
        }
    }
}
#[test]
fn present_restore_and_cursor_restart_do_not_accept_unknown_user_edits() {
    let f = Fixture::new();
    f.legacy_full();
    f.activate();
    let _ = f.active_request();
    let mut t = f.attempt(StartupNeed::SettingsReplay);
    t.begin_mutations().unwrap();
    f.store::<Pending>()
        .reconcile_pending_observed(&mut |i| {
            t.record_settings_transition(i).unwrap();
            Ok(())
        })
        .unwrap();
    drop(t);
    FAIL.with(|v| *v.borrow_mut() = Some("v3_present_restored"));
    assert!(f.prepare(StartupNeed::SettingsReplay).is_err());
    FAIL.with(|v| *v.borrow_mut() = None);
    fs::write(
        f.root.join(Legacy::FILE_NAME),
        b"manual repair must survive",
    )
    .unwrap();
    assert!(f.prepare(StartupNeed::SettingsReplay).is_err());
    assert_eq!(
        fs::read(f.root.join(Legacy::FILE_NAME)).unwrap(),
        b"manual repair must survive"
    );
}
#[test]
fn completed_full_does_not_rescan_old_payload_and_small_scope_rejects_extra_entry() {
    let f = Fixture::new();
    f.store::<Legacy>().read().unwrap();
    f.legacy_full();
    let Versioned::V3(j) = read_versioned(&f.backup.join(JOURNAL)).unwrap().unwrap() else {
        panic!()
    };
    let outcome = validate(
        &j,
        &f.backup,
        &[MigrationSourceRoot {
            label: "handy".into(),
            root: f.root.clone(),
        }],
    )
    .unwrap();
    assert!(!outcome.manifest.entries.is_empty());
    for entry in &outcome.manifest.entries {
        fs::remove_file(outcome.backup_dir.join(&entry.backup_relative_path)).unwrap();
    }
    assert!(matches!(
        f.prepare(StartupNeed::NoWork).unwrap(),
        StartupPreparation::NoWork(_)
    ));
    let mut t = f.attempt(StartupNeed::SettingsProtocol);
    t.begin_mutations().unwrap();
    let mut entry = t.outcome.manifest.entries[0].clone();
    entry.source_relative_path = "history.db".into();
    entry.backup_relative_path = "handy/history.db".into();
    t.outcome.manifest.entries.push(entry);
    write_json_atomically(&t.outcome.manifest_path, &t.outcome.manifest).unwrap();
    t.journal.manifest_sha256 = file_fingerprint(&t.outcome.manifest_path).unwrap().1;
    assert!(t.validate().is_err());
}
#[test]
fn schema_dispatch_rejects_duplicate_unknown_and_mixed_fields() {
    let f = Fixture::new();
    let t = f.attempt(StartupNeed::FullStartup);
    let mut value = serde_json::to_value(&t.journal).unwrap();
    drop(t);
    value["schema_version"] = json!(2);
    fs::write(f.backup.join(JOURNAL), serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(read_versioned(&f.backup.join(JOURNAL)).is_err());
    value["schema_version"] = json!(3);
    value["extra"] = json!(true);
    fs::write(f.backup.join(JOURNAL), serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(read_versioned(&f.backup.join(JOURNAL)).is_err());
    fs::write(
        f.backup.join(JOURNAL),
        b"{\"schema_version\":2,\"schema_version\":3}",
    )
    .unwrap();
    assert!(read_versioned(&f.backup.join(JOURNAL)).is_err());
}
#[test]
fn v2_all_phases_and_no_journal_preserve_unknown_pending_before_upgrade() {
    for phase in [
        None,
        Some(Phase::Prepared),
        Some(Phase::Mutating),
        Some(Phase::Restoring),
        Some(Phase::Recovered),
        Some(Phase::Completed),
    ] {
        let f = Fixture::new();
        if let Some(phase) = phase {
            let mut old =
                super::super::prepare_paths(&f.root, None, &f.backup, &f.locks, None, || Ok(()))
                    .unwrap()
                    .unwrap();
            old.journal.phase = phase;
            old.persist().unwrap();
            drop(old);
        }
        let path = f.root.join(CONTROL_SETTINGS_PENDING_NAME);
        fs::write(&path, b"unknown pending preserve").unwrap();
        assert!(f.prepare(StartupNeed::FullStartup).is_err());
        assert_eq!(fs::read(path).unwrap(), b"unknown pending preserve");
        if phase.is_some() {
            assert!(matches!(
                read_versioned(&f.backup.join(JOURNAL)).unwrap(),
                Some(Versioned::V2(_))
            ));
        }
    }
}
#[test]
fn v2_recovery_must_be_durable_before_v3_prepared_and_unknown_present_cannot_upgrade() {
    let f = Fixture::new();
    let mut old = super::super::prepare_paths(&f.root, None, &f.backup, &f.locks, None, || Ok(()))
        .unwrap()
        .unwrap();
    old.begin_mutations().unwrap();
    f.store::<Legacy>()
        .read_observed_initialization(&mut |i| {
            old.record_settings_initialization(i).unwrap();
            Ok(())
        })
        .unwrap();
    drop(old);
    FAIL.with(|v| *v.borrow_mut() = Some("v3_legacy_terminal_synced"));
    assert!(f.prepare(StartupNeed::FullStartup).is_err());
    FAIL.with(|v| *v.borrow_mut() = None);
    let Some(Versioned::V2(j)) = read_versioned(&f.backup.join(JOURNAL)).unwrap() else {
        panic!("must keep exact v2 chain");
    };
    assert_eq!(j.phase, Phase::Recovered);
    let upgraded = f.attempt(StartupNeed::FullStartup);
    assert_eq!(upgraded.journal.phase, Phase::Prepared);
    drop(upgraded);
    let unknown = Fixture::new();
    unknown.store::<Legacy>().read().unwrap();
    let mut old = super::super::prepare_paths(
        &unknown.root,
        None,
        &unknown.backup,
        &unknown.locks,
        None,
        || Ok(()),
    )
    .unwrap()
    .unwrap();
    old.begin_mutations().unwrap();
    drop(old);
    fs::write(unknown.root.join(Legacy::FILE_NAME), b"later user repair").unwrap();
    assert!(unknown.prepare(StartupNeed::FullStartup).is_err());
    assert_eq!(
        fs::read(unknown.root.join(Legacy::FILE_NAME)).unwrap(),
        b"later user repair"
    );
    assert!(matches!(
        read_versioned(&unknown.backup.join(JOURNAL)).unwrap(),
        Some(Versioned::V2(_))
    ));
}
#[test]
fn baseline_active_operation_is_exact_and_unresolved_or_wrong_outcome_cannot_complete() {
    let f = Fixture::new();
    f.legacy_full();
    f.activate();
    let operation = f.active_request();
    let mut t = f.attempt(StartupNeed::SettingsReplay);
    t.begin_mutations().unwrap();
    assert!(t
        .complete(
            StartupConfirmation::RequestSaved {
                operation_id: operation.clone()
            },
            || Ok(())
        )
        .is_err());
    let mut first = None;
    assert!(f
        .store::<Pending>()
        .reconcile_pending_observed(&mut |intent| {
            first = Some(intent.clone());
            Err(Error::StorageUnavailable)
        })
        .is_err());
    let mut wrong = first.unwrap();
    wrong.operation_id = "foreign-request".into();
    assert!(t.record_settings_transition(&wrong).is_err());
    f.store::<Pending>()
        .reconcile_pending_observed(&mut |i| {
            t.record_settings_transition(i).unwrap();
            Ok(())
        })
        .unwrap();
    assert!(t
        .complete(
            StartupConfirmation::RequestConflict {
                operation_id: operation.clone()
            },
            || Ok(())
        )
        .is_err());
    t.complete(
        StartupConfirmation::RequestSaved {
            operation_id: operation,
        },
        || Ok(()),
    )
    .unwrap();
}
#[test]
fn each_transition_journal_io_boundary_blocks_source_and_recovers_exact_prefix() {
    for phase in [
        inputia_settings::store::TransitionPhase::PrepareRequest,
        inputia_settings::store::TransitionPhase::CommitDocument,
        inputia_settings::store::TransitionPhase::ResolveRequest,
    ] {
        for boundary in [
            "journal_before_rename",
            "journal_after_rename",
            "journal_after_sync",
        ] {
            let f = Fixture::new();
            let mut t = f.attempt(StartupNeed::FullStartup);
            t.begin_mutations().unwrap();
            f.store::<Legacy>()
                .read_observed_initialization(&mut |i| {
                    t.record_settings_initialization(i).unwrap();
                    Ok(())
                })
                .unwrap();
            let store = f.store::<Pending>();
            let snapshot = store
                .activate_pending_protocol(&mut |i| {
                    t.record_protocol_activation(i).unwrap();
                    Ok(())
                })
                .unwrap();
            let req = request(&snapshot, true);
            assert!(
                store
                    .apply_observed(&req, &snapshot, &mut |i| {
                        if i.phase == phase {
                            FAIL.with(|v| *v.borrow_mut() = Some(boundary));
                        }
                        let result = t
                            .record_settings_transition(i)
                            .map_err(|_| Error::StorageUnavailable);
                        FAIL.with(|v| *v.borrow_mut() = None);
                        result
                    })
                    .is_err(),
                "{phase:?}/{boundary}"
            );
            drop(store);
            drop(t);
            drop(f.attempt(StartupNeed::FullStartup));
            assert!(
                !f.root.join(Legacy::FILE_NAME).exists(),
                "{phase:?}/{boundary}"
            );
            assert!(!f.root.join(CONTROL_SETTINGS_PENDING_NAME).exists());
        }
    }
}
#[test]
fn present_restore_temp_file_rename_and_directory_sync_failures_resume() {
    for point in [
        "v3_restore_temp_written",
        "v3_restore_file_synced",
        "v3_restore_renamed",
        "v3_restore_directory_synced",
    ] {
        let f = Fixture::new();
        f.legacy_full();
        f.activate();
        let operation = f.active_request();
        let mut t = f.attempt(StartupNeed::SettingsReplay);
        t.begin_mutations().unwrap();
        f.store::<Pending>()
            .reconcile_pending_observed(&mut |i| {
                t.record_settings_transition(i).unwrap();
                Ok(())
            })
            .unwrap();
        drop(t);
        FAIL.with(|v| *v.borrow_mut() = Some(point));
        assert!(f.prepare(StartupNeed::SettingsReplay).is_err(), "{point}");
        FAIL.with(|v| *v.borrow_mut() = None);
        drop(f.attempt(StartupNeed::SettingsReplay));
        assert!(
            matches!(f.store::<Pending>().pending_status().unwrap(),PendingStatus::Active{operation_id} if operation_id==operation)
        );
    }
}

#[test]
fn active_retry_after_authorized_but_unwritten_commit_or_resolution_keeps_original_chain() {
    use inputia_settings::store::TransitionPhase;
    for failed_phase in [
        TransitionPhase::CommitDocument,
        TransitionPhase::ResolveRequest,
    ] {
        let f = Fixture::new();
        f.legacy_full();
        f.activate();
        let operation = f.active_request();
        let mut t = f.attempt(StartupNeed::SettingsReplay);
        t.begin_mutations().unwrap();
        assert!(f
            .store::<Pending>()
            .reconcile_pending_observed(&mut |i| {
                t.record_settings_transition(i).unwrap();
                if i.phase == failed_phase {
                    Err(Error::StorageUnavailable)
                } else {
                    Ok(())
                }
            })
            .is_err());
        let prior = t.journal.authorizations.clone();
        let frontier = prefixes(&t.journal).unwrap();
        f.store::<Pending>()
            .reconcile_pending_observed(&mut |i| {
                t.record_settings_transition(i).unwrap();
                Ok(())
            })
            .unwrap();
        assert_eq!(&t.journal.authorizations[..prior.len()], &prior);
        assert!(t
            .journal
            .authorizations
            .iter()
            .any(|a| a.reaffirm_prefix.is_some()));
        assert!(prefixes(&t.journal).unwrap().starts_with(&frontier));
        t.complete(
            StartupConfirmation::RequestSaved {
                operation_id: operation,
            },
            || Ok(()),
        )
        .unwrap();
        assert_eq!(f.store::<Pending>().read().unwrap().values["flag"], true);
    }
}

#[test]
fn reaffirmation_cannot_authorize_foreign_content_or_a_nonmatching_whole_prefix() {
    let f = Fixture::new();
    f.legacy_full();
    f.activate();
    f.active_request();
    let mut t = f.attempt(StartupNeed::SettingsReplay);
    t.begin_mutations().unwrap();
    let mut captured = None;
    assert!(f
        .store::<Pending>()
        .reconcile_pending_observed(&mut |i| {
            captured = Some(i.clone());
            Err(Error::StorageUnavailable)
        })
        .is_err());
    let intent = captured.unwrap();
    let mut wrong = intent.clone();
    wrong.after.sha256 = "0".repeat(64);
    wrong.before.sha256 = wrong.after.sha256.clone();
    assert!(t.record_settings_transition(&wrong).is_err());
    assert!(t.journal.authorizations.is_empty());
    t.record_settings_transition(&intent).unwrap();
    let mut corrupt = t.journal.clone();
    corrupt.authorizations[0].reaffirm_prefix = Some(usize::MAX);
    assert!(prefixes(&corrupt).is_err());
    fs::write(f.root.join(Legacy::FILE_NAME), b"unknown later edit").unwrap();
    assert!(t.record_settings_transition(&intent).is_err());
}

#[test]
fn resolved_saved_receipt_retry_keeps_same_operation_and_never_commits_twice() {
    let f = Fixture::new();
    f.legacy_full();
    f.activate();
    let mut t = f.attempt(StartupNeed::FullStartup);
    t.begin_mutations().unwrap();
    let store = f.store::<Pending>();
    let floor = store.read().unwrap();
    let req = request(&floor, true);
    let first = store
        .apply_observed(&req, &floor, &mut |i| {
            t.record_settings_transition(i).unwrap();
            Ok(())
        })
        .unwrap();
    let ApplyResult::Saved {
        commit_revision, ..
    } = first
    else {
        panic!("saved expected")
    };
    let retried = store
        .apply_observed(&req, &floor, &mut |i| {
            t.record_settings_transition(i).unwrap();
            Ok(())
        })
        .unwrap();
    assert!(
        matches!(retried, ApplyResult::Saved {commit_revision: ref revision, replayed:true, ..} if revision == &commit_revision)
    );
    assert_eq!(
        t.journal
            .authorizations
            .iter()
            .filter(|a| matches!(
                a.owner,
                Owner::Transition {
                    phase: StepPhase::CommitDocument,
                    ..
                }
            ))
            .count(),
        1
    );
    t.complete(StartupConfirmation::FullStartupReady, || Ok(()))
        .unwrap();
}

#[test]
fn changed_restore_payload_is_rejected_before_replacing_current_document() {
    let f = Fixture::new();
    let target = f.root.join("settings_store.json");
    let payload = f.root.join("payload.json");
    fs::write(&target, b"current owned document").unwrap();
    fs::write(&payload, b"later unknown backup").unwrap();
    let expected = FileStamp {
        sha256: sha(b"original"),
        size: 8,
    };
    assert!(restore::restore_present(
        &payload,
        &target,
        &digest(&observe(&target).unwrap()),
        &expected
    )
    .is_err());
    assert_eq!(fs::read(&target).unwrap(), b"current owned document");
}

thread_local! {
    static CHANGE_BEFORE_PRESENT_RESTORE: std::cell::RefCell<Option<(PathBuf, Vec<u8>)>> = const {std::cell::RefCell::new(None)};
}
pub(super) fn inject_before_present_restore() {
    CHANGE_BEFORE_PRESENT_RESTORE.with(|slot| {
        if let Some((path, bytes)) = slot.borrow_mut().take() {
            fs::write(path, bytes).unwrap();
        }
    });
}
#[test]
fn unknown_document_between_prefix_check_and_restore_is_preserved_without_cursor_progress() {
    let f = Fixture::new();
    f.legacy_full();
    f.activate();
    f.active_request();
    let mut t = f.attempt(StartupNeed::SettingsReplay);
    t.begin_mutations().unwrap();
    f.store::<Pending>()
        .reconcile_pending_observed(&mut |i| {
            t.record_settings_transition(i).unwrap();
            Ok(())
        })
        .unwrap();
    drop(t);
    let document = f.root.join(Legacy::FILE_NAME);
    let unknown = b"user changed after outer prefix check".to_vec();
    CHANGE_BEFORE_PRESENT_RESTORE
        .with(|slot| *slot.borrow_mut() = Some((document.clone(), unknown.clone())));
    assert!(f.prepare(StartupNeed::SettingsReplay).is_err());
    assert_eq!(fs::read(&document).unwrap(), unknown);
    let Some(Versioned::V3(j)) = read_versioned(&f.backup.join(JOURNAL)).unwrap() else {
        panic!("v3")
    };
    assert_eq!(j.phase, Phase::Restoring);
    assert_eq!(j.recovery.unwrap().cursor, 0);
    assert!(f.prepare(StartupNeed::SettingsReplay).is_err());
    assert_eq!(fs::read(document).unwrap(), unknown);
}
