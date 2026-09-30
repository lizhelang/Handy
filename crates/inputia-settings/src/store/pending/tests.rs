use super::*;
use serde_json::json;
use std::{
    fs,
    os::unix::fs::{symlink, PermissionsExt},
};

struct Schema;
impl DocumentSchema for Schema {
    const FILE_NAME: &'static str = "control.json";
    const MARKER_NAME: &'static str = ".control-marker.json";
    const PENDING_NAME: Option<&'static str> = Some(".control-pending.json");
    const DOMAIN: &'static str = "fixture.pending";
    fn defaults(_: &Path) -> Result<Map<String, Value>> {
        Ok(json!({"enabled":true,"credential":""})
            .as_object()
            .unwrap()
            .clone())
    }
    fn validate(values: &Map<String, Value>, _: &Path, _: bool) -> Result<Map<String, Value>> {
        if !values.get("enabled").is_some_and(Value::is_boolean)
            || !values.get("credential").is_some_and(Value::is_string)
        {
            return Err(Error::InvalidRequest);
        }
        Ok(values.clone())
    }
}
type TestStore = DocumentStore<Schema>;
struct Fixture {
    _temp: tempfile::TempDir,
    home: PathBuf,
    path: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().canonicalize().unwrap();
        let path = home.join(Schema::FILE_NAME);
        Self {
            _temp: temp,
            home,
            path,
        }
    }
    fn open(&self) -> TestStore {
        TestStore::open(&self.path, &self.home, unsafe { libc::geteuid() }).unwrap()
    }
    fn activate(&self) -> Snapshot {
        let store = self.open();
        store.read_observed_initialization(&mut |_| Ok(())).unwrap();
        store.activate_pending_protocol(&mut |_| Ok(())).unwrap()
    }
    fn files(&self) -> [Option<Vec<u8>>; 3] {
        [
            Schema::PENDING_NAME.unwrap(),
            Schema::FILE_NAME,
            Schema::MARKER_NAME,
        ]
        .map(|name| fs::read(self.home.join(name)).ok())
    }
    fn ledger(&self) -> PathBuf {
        self.home.join(Schema::PENDING_NAME.unwrap())
    }
    fn marker(&self) -> PathBuf {
        self.home.join(Schema::MARKER_NAME)
    }
}
fn request(snapshot: &Snapshot) -> PatchRequest {
    PatchRequest {
        operation_id: snapshot.operation_id(),
        expected_store_id: snapshot.store_id.clone(),
        expected_revision: snapshot.revision.clone(),
        patch: BTreeMap::from([("credential".into(), json!("private-test-secret"))]),
    }
}
const WRITES: [Boundary; 4] = [
    Boundary::TempWritten,
    Boundary::FileSynced,
    Boundary::Renamed,
    Boundary::DirectorySynced,
];
fn fail_at(stage: Stage, boundary: Boundary) -> impl FnMut(Stage, Boundary) -> Result<()> {
    move |s, b| {
        if s == stage && b == boundary {
            Err(Error::StorageUnavailable)
        } else {
            Ok(())
        }
    }
}
fn assert_saved(result: ApplyResult, id: &str) {
    match result {
        ApplyResult::Saved {
            commit_revision,
            current,
            ..
        } => {
            assert_eq!(commit_revision, "1");
            assert_eq!(current.revision, "1");
            assert_eq!(current.values["credential"], "private-test-secret");
            assert!(!id.is_empty());
        }
        _ => panic!("expected saved"),
    }
}

#[test]
fn activation_is_explicit_observed_and_protocol_floor_prevents_downgrade() {
    let fixture = Fixture::new();
    let store = fixture.open();
    assert_eq!(
        store
            .activate_pending_protocol(&mut |_| panic!("no implicit initialization"))
            .unwrap_err(),
        Error::RepairRequired
    );
    let old = store.read_observed_initialization(&mut |_| Ok(())).unwrap();
    let originals = fixture.files();
    assert_eq!(
        store.apply(&request(&old)).unwrap_err(),
        Error::PendingProtocolRequired
    );
    assert_eq!(
        store
            .activate_pending_protocol(&mut |_| Err(Error::StorageUnavailable))
            .unwrap_err(),
        Error::StorageUnavailable
    );
    assert_eq!(fixture.files(), originals);
    let upgraded = store
        .activate_pending_protocol(&mut |intent| {
            assert_eq!(intent.files.len(), 3);
            for (file, original) in intent.files.iter().zip(&originals) {
                assert_eq!(
                    file.original_sha256,
                    original.as_ref().map(|bytes| raw_digest(bytes))
                );
            }
            assert_eq!(fixture.files(), originals);
            Ok(())
        })
        .unwrap();
    assert_eq!(old.values, upgraded.values);
    assert_eq!(old.revision, upgraded.revision);
    assert!(!old.same_protocol(&upgraded));
    let v2 = fixture.files();
    let again = store
        .activate_pending_protocol(&mut |_| panic!("already active"))
        .unwrap();
    assert_eq!(fixture.files(), v2);
    assert!(again.same_protocol(&upgraded));
    fs::write(&fixture.path, originals[1].as_ref().unwrap()).unwrap();
    fs::write(fixture.marker(), originals[2].as_ref().unwrap()).unwrap();
    fs::remove_file(fixture.ledger()).unwrap();
    assert_eq!(
        store.read_at_least(&upgraded).unwrap_err(),
        Error::RepairRequired
    );
    assert_eq!(
        store.apply(&request(&old)).unwrap_err(),
        Error::PendingProtocolRequired
    );
}

#[test]
fn activation_observer_changes_to_any_original_abort_without_activation_writes() {
    for changed in 0..3 {
        let fixture = Fixture::new();
        let store = fixture.open();
        store.read_observed_initialization(&mut |_| Ok(())).unwrap();
        let names = [
            Schema::PENDING_NAME.unwrap(),
            Schema::FILE_NAME,
            Schema::MARKER_NAME,
        ];
        let mut modified = None;
        assert_eq!(
            store
                .activate_pending_protocol(&mut |_| {
                    let path = fixture.home.join(names[changed]);
                    fs::write(&path, b"{}").unwrap();
                    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
                    modified = Some(fixture.files());
                    Ok(())
                })
                .unwrap_err(),
            Error::ExternalChanged
        );
        assert_eq!(fixture.files(), modified.unwrap());
    }
}

#[test]
fn activation_fault_windows_are_never_implicitly_repaired() {
    for stage in [
        Stage::ActivationLedger,
        Stage::ActivationDocument,
        Stage::ActivationMarker,
    ] {
        for boundary in WRITES {
            let fixture = Fixture::new();
            let old;
            {
                let store = fixture.open();
                old = store.read_observed_initialization(&mut |_| Ok(())).unwrap();
                assert!(store
                    .activate_with_hook(&mut |_| Ok(()), &mut fail_at(stage, boundary))
                    .is_err());
            }
            let after = fixture.files();
            let store = fixture.open();
            let complete = stage == Stage::ActivationMarker
                && matches!(boundary, Boundary::Renamed | Boundary::DirectorySynced);
            let untouched = stage == Stage::ActivationLedger
                && matches!(boundary, Boundary::TempWritten | Boundary::FileSynced);
            if complete {
                store.read().unwrap();
                let mut synced = 0;
                store
                    .activate_with_hook(&mut |_| panic!("no fresh activation"), &mut |_, b| {
                        if b == Boundary::ReplayDirectorySynced {
                            synced += 1;
                        }
                        Ok(())
                    })
                    .unwrap();
                assert_eq!(synced, 2);
            } else if untouched {
                store.read().unwrap();
                assert_eq!(
                    store.apply(&request(&old)).unwrap_err(),
                    Error::PendingProtocolRequired
                );
            } else {
                assert!(store.read().is_err());
                assert!(store.preflight().is_err());
                assert!(store.apply(&request(&old)).is_err());
                assert!(store
                    .activate_pending_protocol(&mut |_| panic!("partial cannot activate"))
                    .is_err());
            }
            assert_eq!(fixture.files(), after);
        }
    }
}

#[test]
fn prepare_document_and_tombstone_faults_recover_original_request() {
    for stage in [Stage::Prepare, Stage::Document, Stage::Resolve] {
        for boundary in WRITES {
            let fixture = Fixture::new();
            let snapshot = fixture.activate();
            let req = request(&snapshot);
            let original_document = fs::read(&fixture.path).unwrap();
            {
                let store = fixture.open();
                assert!(store
                    .apply_pending(&req, None, &mut fail_at(stage, boundary))
                    .is_err());
            }
            if stage == Stage::Prepare {
                assert_eq!(fs::read(&fixture.path).unwrap(), original_document);
            }
            let store = fixture.open();
            let active = matches!(
                store.pending_status().unwrap(),
                PendingStatus::Active { .. }
            );
            if active {
                let other = request(&snapshot);
                assert_eq!(store.apply(&other).unwrap_err(), Error::PendingOperation);
                let mut changed = req.clone();
                changed
                    .patch
                    .insert("credential".into(), json!("different"));
                assert_eq!(store.apply(&changed).unwrap_err(), Error::OperationMismatch);
                assert_saved(
                    store.reconcile_pending().unwrap().unwrap(),
                    &req.operation_id,
                );
            } else {
                assert_saved(store.apply(&req).unwrap(), &req.operation_id);
            }
            let raw = fs::read_to_string(fixture.ledger()).unwrap();
            assert!(!raw.contains("private-test-secret"));
            assert!(!raw.contains("\"patch\""));
            assert_saved(store.apply(&req).unwrap(), &req.operation_id);
            let document = strict_json(&fs::read(&fixture.path).unwrap()).unwrap();
            assert_eq!(document[META]["receipts"].as_array().unwrap().len(), 1);
            assert_eq!(
                document[META]["receipts"][0]["operation_id"],
                req.operation_id
            );
        }
    }
}

#[test]
fn active_ledger_blocks_external_import_and_keeps_original_preview() {
    let fixture = Fixture::new();
    let snapshot = fixture.activate();
    let req = request(&snapshot);
    {
        let store = fixture.open();
        assert!(store
            .apply_pending(
                &req,
                None,
                &mut fail_at(Stage::Prepare, Boundary::DirectorySynced)
            )
            .is_err());
    }
    let mut raw = strict_json(&fs::read(&fixture.path).unwrap()).unwrap();
    raw["enabled"] = json!(false);
    fs::write(&fixture.path, canonical(&raw).unwrap()).unwrap();
    let store = fixture.open();
    let preview = store.inspect_external().unwrap();
    let import = ImportRequest {
        operation_id: snapshot.operation_id(),
        expected_store_id: snapshot.store_id,
        expected_revision: snapshot.revision,
        observed_file_digest: preview.observed_file_digest,
    };
    assert_eq!(
        store.import_external(&import).unwrap_err(),
        Error::PendingOperation
    );
    assert_eq!(store.reconcile_pending().unwrap_err(), Error::ExternalEdit);
    assert!(fs::read_to_string(fixture.ledger())
        .unwrap()
        .contains(&req.operation_id));
}

#[test]
fn import_request_reopens_with_same_preview_and_durable_receipt() {
    for stage in [Stage::Prepare, Stage::Document, Stage::Resolve] {
        for boundary in WRITES {
            let fixture = Fixture::new();
            let snapshot = fixture.activate();
            let mut raw = strict_json(&fs::read(&fixture.path).unwrap()).unwrap();
            raw["credential"] = json!("private-test-secret");
            fs::write(&fixture.path, canonical(&raw).unwrap()).unwrap();
            let request;
            {
                let store = fixture.open();
                let preview = store.inspect_external().unwrap();
                request = ImportRequest {
                    operation_id: snapshot.operation_id(),
                    expected_store_id: snapshot.store_id,
                    expected_revision: snapshot.revision,
                    observed_file_digest: preview.observed_file_digest,
                };
                assert!(store
                    .import_pending(&request, &mut fail_at(stage, boundary))
                    .is_err());
            }
            let store = fixture.open();
            match store.reconcile_pending().unwrap() {
                Some(result) => assert_saved(result, &request.operation_id),
                None => assert_saved(
                    store.import_external(&request).unwrap(),
                    &request.operation_id,
                ),
            }
            assert!(!fs::read_to_string(fixture.ledger())
                .unwrap()
                .contains("private-test-secret"));
            assert_eq!(store.read().unwrap().revision, "1");
        }
    }
}

#[test]
fn missing_mixed_unsafe_or_regressed_files_do_not_reset_pending() {
    for missing in 0..3 {
        let fixture = Fixture::new();
        let snapshot = fixture.activate();
        let req = request(&snapshot);
        let names = [
            Schema::PENDING_NAME.unwrap(),
            Schema::FILE_NAME,
            Schema::MARKER_NAME,
        ];
        fs::remove_file(fixture.home.join(names[missing])).unwrap();
        let before = fixture.files();
        let store = fixture.open();
        assert!(store.read().is_err());
        assert!(store.apply(&req).is_err());
        assert!(store
            .activate_pending_protocol(&mut |_| panic!("missing protocol file"))
            .is_err());
        assert_eq!(fixture.files(), before);
    }
    let fixture = Fixture::new();
    let snapshot = fixture.activate();
    let original = fs::read(&fixture.path).unwrap();
    {
        let store = fixture.open();
        store.apply(&request(&snapshot)).unwrap();
    }
    fs::write(&fixture.path, &original).unwrap();
    let before = fixture.files();
    {
        let store = fixture.open();
        assert!(store.read().is_err());
        assert_eq!(fixture.files(), before);
    }
    let other = Fixture::new();
    other.activate();
    fs::write(fixture.ledger(), fs::read(other.ledger()).unwrap()).unwrap();
    assert!(fixture.open().read().is_err());
    fs::remove_file(fixture.ledger()).unwrap();
    symlink(other.ledger(), fixture.ledger()).unwrap();
    assert!(fixture.open().read().is_err());
}

#[test]
fn oversized_invalid_or_future_request_never_poison_idle_ledger() {
    let fixture = Fixture::new();
    let snapshot = fixture.activate();
    let before = fixture.files();
    let store = fixture.open();
    let mut req = request(&snapshot);
    req.patch.insert("enabled".into(), json!("invalid"));
    assert_eq!(store.apply(&req).unwrap_err(), Error::InvalidRequest);
    assert_eq!(fixture.files(), before);
    req = request(&snapshot);
    req.patch
        .insert("credential".into(), json!("x".repeat(LIMIT)));
    assert!(store.apply(&req).is_err());
    assert_eq!(fixture.files(), before);
    req = request(&snapshot);
    req.expected_revision = "10".into();
    req.operation_id = format!("v1:{}:10:{}", snapshot.store_id, uuid::Uuid::new_v4());
    assert!(matches!(
        store.apply(&req).unwrap(),
        ApplyResult::Conflict { .. }
    ));
    assert_eq!(fixture.files(), before);
}

#[test]
fn tombstone_never_substitutes_for_real_receipt_and_active_expired_stays_blocked() {
    let fixture = Fixture::new();
    let snapshot = fixture.activate();
    let req = request(&snapshot);
    let saved_ledger;
    {
        let store = fixture.open();
        store.apply(&req).unwrap();
        saved_ledger = fs::read(fixture.ledger()).unwrap();
        for index in 0..RECEIPT_LIMIT {
            let snap = store.read().unwrap();
            let mut next = request(&snap);
            next.patch
                .insert("credential".into(), json!(index.to_string()));
            store.apply(&next).unwrap();
        }
        assert!(matches!(
            store.apply(&req).unwrap(),
            ApplyResult::OutcomeExpired { .. }
        ));
        assert!(matches!(
            store.pending_status().unwrap(),
            PendingStatus::Active { .. }
        ));
        assert_eq!(
            store.apply(&request(&store.read().unwrap())).unwrap_err(),
            Error::PendingOperation
        );
    }
    fs::write(fixture.ledger(), saved_ledger).unwrap();
    assert_eq!(fixture.open().read().unwrap_err(), Error::RepairRequired);
}

#[test]
fn child_crash_writer() {
    let Ok(home) = std::env::var("INPUTIA_PENDING_CHILD_HOME") else {
        return;
    };
    let home = PathBuf::from(home);
    let store = TestStore::open(&home.join(Schema::FILE_NAME), &home, unsafe {
        libc::geteuid()
    })
    .unwrap();
    let request: PatchRequest =
        serde_json::from_str(&std::env::var("INPUTIA_PENDING_CHILD_REQUEST").unwrap()).unwrap();
    let stage = std::env::var("INPUTIA_PENDING_CHILD_STAGE")
        .unwrap()
        .parse::<usize>()
        .unwrap();
    let boundary = std::env::var("INPUTIA_PENDING_CHILD_BOUNDARY")
        .unwrap()
        .parse::<usize>()
        .unwrap();
    store
        .apply_pending(&request, None, &mut |s, b| {
            if s == [Stage::Prepare, Stage::Document, Stage::Resolve][stage]
                && b == WRITES[boundary]
            {
                // 仅杀测试自身，跳过 Drop 与临时文件清理，重启后由父测试核验。
                unsafe {
                    libc::kill(libc::getpid(), libc::SIGKILL);
                }
                unreachable!("test child SIGKILL");
            }
            Ok(())
        })
        .unwrap();
    panic!("expected crash boundary");
}

#[test]
fn separate_process_sigkill_keeps_original_operation_and_no_new_writer_can_bypass_it() {
    use std::{
        os::unix::process::ExitStatusExt,
        process::{Command, Stdio},
    };
    for stage in 0..3 {
        for boundary in 0..4 {
            let fixture = Fixture::new();
            let snapshot = fixture.activate();
            let req = request(&snapshot);
            let status = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "store::pending::tests::child_crash_writer"])
                .env("INPUTIA_PENDING_CHILD_HOME", &fixture.home)
                .env(
                    "INPUTIA_PENDING_CHILD_REQUEST",
                    serde_json::to_string(&req).unwrap(),
                )
                .env("INPUTIA_PENDING_CHILD_STAGE", stage.to_string())
                .env("INPUTIA_PENDING_CHILD_BOUNDARY", boundary.to_string())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .output()
                .unwrap()
                .status;
            assert_eq!(status.signal(), Some(libc::SIGKILL));
            let store = fixture.open();
            if matches!(
                store.pending_status().unwrap(),
                PendingStatus::Active { .. }
            ) {
                assert_eq!(
                    store.apply(&request(&snapshot)).unwrap_err(),
                    Error::PendingOperation
                );
            }
            match store.reconcile_pending().unwrap() {
                Some(result) => assert_saved(result, &req.operation_id),
                None => assert_saved(store.apply(&req).unwrap(), &req.operation_id),
            }
            let raw = strict_json(&fs::read(&fixture.path).unwrap()).unwrap();
            assert_eq!(raw[META]["receipts"].as_array().unwrap().len(), 1);
            assert_eq!(raw[META]["receipts"][0]["operation_id"], req.operation_id);
        }
    }
}

#[test]
fn changed_import_preview_and_invalid_ledger_are_preserved_without_new_write() {
    let fixture = Fixture::new();
    let snapshot = fixture.activate();
    let mut raw = strict_json(&fs::read(&fixture.path).unwrap()).unwrap();
    raw["enabled"] = json!(false);
    fs::write(&fixture.path, canonical(&raw).unwrap()).unwrap();
    let import;
    {
        let store = fixture.open();
        let preview = store.inspect_external().unwrap();
        import = ImportRequest {
            operation_id: snapshot.operation_id(),
            expected_store_id: snapshot.store_id,
            expected_revision: snapshot.revision,
            observed_file_digest: preview.observed_file_digest,
        };
        assert!(store
            .import_pending(
                &import,
                &mut fail_at(Stage::Prepare, Boundary::DirectorySynced)
            )
            .is_err());
    }
    raw["credential"] = json!("changed-after-preview");
    fs::write(&fixture.path, canonical(&raw).unwrap()).unwrap();
    let before = fixture.files();
    {
        let store = fixture.open();
        assert!(matches!(
            store.pending_status().unwrap(),
            PendingStatus::Active { .. }
        ));
        assert_eq!(
            store.reconcile_pending().unwrap_err(),
            Error::ExternalChanged
        );
        assert_eq!(fixture.files(), before);
    }
    let valid = fs::read(fixture.ledger()).unwrap();
    for invalid in [
        br#"{"schema_version":1,"schema_version":1}"#.to_vec(),
        vec![b' '; LIMIT + 1],
        {
            let mut ledger = strict_json(&valid).unwrap();
            ledger["ledger_id"] = json!(uuid::Uuid::new_v4().to_string());
            canonical(&ledger).unwrap()
        },
    ] {
        fs::write(fixture.ledger(), invalid).unwrap();
        let before = fixture.files();
        let store = fixture.open();
        assert!(store.reconcile_pending().is_err());
        assert!(store.pending_status().is_err());
        assert_eq!(fixture.files(), before);
    }
    fs::write(fixture.ledger(), valid).unwrap();
    fs::set_permissions(fixture.ledger(), fs::Permissions::from_mode(0o644)).unwrap();
    assert!(fixture.open().pending_status().is_err());
}

#[test]
fn opted_in_plain_read_and_apply_never_initialize_without_observer() {
    for state in ["absent", "flat", "missing_marker"] {
        let fixture = Fixture::new();
        let request = if state == "missing_marker" {
            let snapshot = fixture
                .open()
                .read_observed_initialization(&mut |_| Ok(()))
                .unwrap();
            fs::remove_file(fixture.marker()).unwrap();
            request(&snapshot)
        } else {
            if state == "flat" {
                fs::write(&fixture.path, br#"{"enabled":false,"credential":"old"}"#).unwrap();
            }
            let store_id = uuid::Uuid::new_v4().to_string();
            PatchRequest {
                operation_id: format!("v1:{store_id}:0:{}", uuid::Uuid::new_v4()),
                expected_store_id: store_id,
                expected_revision: "0".into(),
                patch: BTreeMap::from([("enabled".into(), json!(true))]),
            }
        };
        let before = fixture.files();
        let store = fixture.open();
        assert_eq!(
            store.apply(&request).unwrap_err(),
            Error::PendingProtocolRequired
        );
        assert_eq!(fixture.files(), before);
        assert_eq!(store.read().unwrap_err(), Error::PendingProtocolRequired);
        assert_eq!(fixture.files(), before);
        store.preflight().unwrap();
        assert_eq!(fixture.files(), before);
        let mut observed = false;
        store
            .read_observed_initialization(&mut |_| {
                observed = true;
                Ok(())
            })
            .unwrap();
        assert!(observed);
        store.activate_pending_protocol(&mut |_| Ok(())).unwrap();
        store.read().unwrap();
    }
}

#[test]
fn transition_observer_registers_each_exact_before_and_after_without_values() {
    let fixture = Fixture::new();
    let snapshot = fixture.activate();
    let req = request(&snapshot);
    let store = fixture.open();
    let mut intents = Vec::new();
    store
        .apply_observed(&req, &snapshot, &mut |intent| {
            assert_eq!(
                intent.before,
                FileDigest::of(&fs::read(fixture.home.join(&intent.file_name)).unwrap())
            );
            assert_eq!(intent.domain, Schema::DOMAIN);
            assert_eq!(intent.store_id, snapshot.store_id);
            assert_eq!(intent.operation_id, req.operation_id);
            assert!(!serde_json::to_string(intent)
                .unwrap()
                .contains("private-test-secret"));
            intents.push(intent.clone());
            Ok(())
        })
        .unwrap();
    assert_eq!(
        intents.iter().map(|i| i.phase).collect::<Vec<_>>(),
        vec![
            TransitionPhase::PrepareRequest,
            TransitionPhase::CommitDocument,
            TransitionPhase::ResolveRequest
        ]
    );
    assert_eq!(intents[0].after, intents[2].before);
    assert_eq!(
        intents[1].after,
        FileDigest::of(&fs::read(&fixture.path).unwrap())
    );
    assert_eq!(
        intents[2].after,
        FileDigest::of(&fs::read(fixture.ledger()).unwrap())
    );
    store
        .reconcile_pending_observed(&mut |_| panic!("resolved only needs durability confirmation"))
        .unwrap();
}

#[test]
fn observer_failure_never_writes_its_target_and_recovery_keeps_original_id() {
    for rejected in [
        TransitionPhase::PrepareRequest,
        TransitionPhase::CommitDocument,
        TransitionPhase::ResolveRequest,
    ] {
        let fixture = Fixture::new();
        let snapshot = fixture.activate();
        let req = request(&snapshot);
        let mut stopped = None;
        {
            let store = fixture.open();
            let failure = store
                .apply_observed(&req, &snapshot, &mut |intent| {
                    if intent.phase == rejected {
                        stopped = Some((
                            intent.file_name.clone(),
                            fs::read(fixture.home.join(&intent.file_name)).unwrap(),
                        ));
                        Err(Error::StorageUnavailable)
                    } else {
                        Ok(())
                    }
                })
                .unwrap_err();
            if rejected == TransitionPhase::ResolveRequest {
                assert_eq!(failure, Error::CommitUncertain);
            }
        }
        let (name, original) = stopped.unwrap();
        assert_eq!(fs::read(fixture.home.join(name)).unwrap(), original);
        let store = fixture.open();
        let mut observer = |intent: &TransitionIntent| {
            assert_eq!(intent.operation_id, req.operation_id);
            Ok(())
        };
        match store.reconcile_pending_observed(&mut observer).unwrap() {
            Some(result) => assert_saved(result, &req.operation_id),
            None => assert_saved(
                store
                    .apply_observed(&req, &snapshot, &mut observer)
                    .unwrap(),
                &req.operation_id,
            ),
        }
        let raw = strict_json(&fs::read(&fixture.path).unwrap()).unwrap();
        assert_eq!(raw[META]["receipts"].as_array().unwrap().len(), 1);
    }
}

#[test]
fn transition_callback_cannot_authorize_changes_to_any_source_file() {
    for changed in [
        Schema::FILE_NAME,
        Schema::MARKER_NAME,
        Schema::PENDING_NAME.unwrap(),
    ] {
        let fixture = Fixture::new();
        let snapshot = fixture.activate();
        let store = fixture.open();
        let mut after_external = None;
        assert_eq!(
            store
                .apply_observed(&request(&snapshot), &snapshot, &mut |_| {
                    fs::write(fixture.home.join(changed), b"{}").unwrap();
                    after_external = Some(fixture.files());
                    Ok(())
                })
                .unwrap_err(),
            Error::ExternalChanged
        );
        assert_eq!(fixture.files(), after_external.unwrap());
    }
}

#[test]
fn recovery_preflight_only_accepts_original_pending_external_preview() {
    let fixture = Fixture::new();
    let snapshot = fixture.activate();
    let mut raw = strict_json(&fs::read(&fixture.path).unwrap()).unwrap();
    raw["enabled"] = json!(false);
    fs::write(&fixture.path, canonical(&raw).unwrap()).unwrap();
    let import;
    {
        let store = fixture.open();
        assert_eq!(
            store.preflight_pending_recovery().unwrap_err(),
            Error::ExternalEdit
        );
        let preview = store.inspect_external().unwrap();
        import = ImportRequest {
            operation_id: snapshot.operation_id(),
            expected_store_id: snapshot.store_id,
            expected_revision: snapshot.revision,
            observed_file_digest: preview.observed_file_digest,
        };
        assert!(store
            .import_pending(
                &import,
                &mut fail_at(Stage::Prepare, Boundary::DirectorySynced)
            )
            .is_err());
    }
    let store = fixture.open();
    let before = fixture.files();
    store.preflight_pending_recovery().unwrap();
    assert_eq!(fixture.files(), before);
    assert_eq!(store.preflight().unwrap_err(), Error::ExternalEdit);
    let mut phases = Vec::new();
    store
        .reconcile_pending_observed(&mut |intent| {
            phases.push(intent.phase);
            assert_eq!(intent.operation_id, import.operation_id);
            Ok(())
        })
        .unwrap();
    assert_eq!(phases.len(), 3);
    store.preflight().unwrap();
    raw["credential"] = json!("changed-after-confirmation");
    fs::write(&fixture.path, canonical(&raw).unwrap()).unwrap();
    assert!(store.preflight_pending_recovery().is_err());
}

#[test]
fn changes_between_commit_phases_cannot_be_registered_as_a_new_baseline() {
    for observed in [false, true] {
        for stage in [Stage::Prepare, Stage::Document] {
            for changed in [
                Schema::FILE_NAME,
                Schema::MARKER_NAME,
                Schema::PENDING_NAME.unwrap(),
            ] {
                let fixture = Fixture::new();
                let snapshot = fixture.activate();
                let req = request(&snapshot);
                let before_document = fs::read(&fixture.path).unwrap();
                let store = fixture.open();
                let mut phases = Vec::new();
                let mut observer = |intent: &TransitionIntent| {
                    phases.push(intent.phase);
                    Ok(())
                };
                let mut hook = |s, b| {
                    if s == stage && b == Boundary::DirectorySynced {
                        let path = fixture.home.join(changed);
                        let mut raw = fs::read(&path).unwrap();
                        raw.push(b'\n');
                        fs::write(path, raw).unwrap();
                    }
                    Ok(())
                };
                let mut hooks = PendingHooks {
                    fault: &mut hook,
                    observer: if observed { Some(&mut observer) } else { None },
                };
                let error = store
                    .apply_with_observer(&req, Some(&snapshot), &mut hooks)
                    .unwrap_err();
                if stage == Stage::Document {
                    assert_eq!(error, Error::CommitUncertain);
                } else {
                    assert_eq!(error, Error::ExternalChanged);
                    if changed != Schema::FILE_NAME {
                        assert_eq!(fs::read(&fixture.path).unwrap(), before_document);
                    }
                }
                assert!(!phases.contains(&TransitionPhase::ResolveRequest));
                let ledger = strict_json(&fs::read(fixture.ledger()).unwrap()).unwrap();
                assert_eq!(ledger["state"]["phase"], "active");
                assert_eq!(
                    ledger["state"]["request"]["request"]["operation_id"],
                    req.operation_id
                );
            }
        }
    }
}
