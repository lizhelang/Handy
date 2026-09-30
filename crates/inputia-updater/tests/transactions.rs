use inputia_settings::installation::{
    ComponentPaths, DataLocation, InstallationReceipt, InstallationScope, UpdateChannel,
};
use inputia_updater::*;
use std::{
    collections::BTreeSet,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

fn write(path: &Path, data: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, data).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}
fn component(root: &Path, text: &str) {
    write(&root.join("Contents/MacOS/main"), text.as_bytes());
    fs::set_permissions(
        root.join("Contents/MacOS/main"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
}
fn roles() -> BTreeSet<Role> {
    Role::COMPONENTS.into_iter().collect()
}
fn receipt(home: &Path, release: &str) -> InstallationReceipt {
    InstallationReceipt {
        schema_version: 1,
        product_id: "com.inputia".into(),
        installation_id: "11111111-1111-4111-8111-111111111111".into(),
        profile_id: "22222222-2222-4222-8222-222222222222".into(),
        uid: unsafe { libc::geteuid() },
        scope: InstallationScope::User,
        data: DataLocation::Managed,
        components: ComponentPaths {
            control: home.join("Applications/Inputia.app"),
            ime: home.join("Library/Input Methods/InputiaUnifiedCandidate.app"),
            settings: home.join("Applications/Inputia 设置.app"),
        },
        release_id: release.into(),
        channel: UpdateChannel::Candidate,
    }
}
struct Fixture {
    _temp: tempfile::TempDir,
    home: PathBuf,
    updater: Updater,
    request: InstallRequest,
    old: Option<InstallationReceipt>,
    data: PathBuf,
}
impl Fixture {
    fn new(update: bool) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().canonicalize().unwrap().join("home");
        fs::create_dir(&home).unwrap();
        let uid = unsafe { libc::geteuid() };
        let updater = Updater::new(home.clone(), uid).unwrap();
        let old = update.then(|| receipt(&home, "inputia-1.0.0-old"));
        if let Some(old) = &old {
            for path in [
                &old.components.control,
                &old.components.ime,
                &old.components.settings,
            ] {
                component(path, "old-code");
            }
            write(
                &home.join("Library/Application Support/Inputia/installation.json"),
                &serde_json::to_vec(old).unwrap(),
            );
            write(&home.join("Library/Application Support/Inputia/Releases/inputia-1.0.0-old/pair-manifest.json"), b"old-pair-synthetic");
        }
        let mut artifacts = Vec::new();
        for role in [Role::Control, Role::Ime, Role::Settings, Role::PairManifest] {
            let source = temp
                .path()
                .canonicalize()
                .unwrap()
                .join(format!("sources/{}", role.label()));
            if role == Role::PairManifest {
                write(&source, b"new-pair-synthetic");
            } else {
                component(&source, "new-code");
            }
            artifacts.push(Artifact {
                role,
                expected: fingerprint(&source, uid).unwrap(),
                source,
            });
        }
        let data = home.join("Library/Application Support/Inputia/Profiles/22222222-2222-4222-8222-222222222222/new-user-data");
        write(&data, b"original + new revisions + privacy tombstones");
        let request = InstallRequest {
            transaction_id: "33333333-3333-4333-8333-333333333333".into(),
            new_receipt: receipt(&home, "inputia-1.1.0-new"),
            artifacts,
        };
        Self {
            _temp: temp,
            home,
            updater,
            request,
            old,
            data,
        }
    }
    fn prepare(&self) -> PreparedPlan {
        self.updater.prepare(self.request.clone()).unwrap()
    }
    fn adapter(&self) -> SyntheticNative {
        SyntheticNative {
            root: self.updater.root(),
            uid: unsafe { libc::geteuid() },
            fail_postcheck: false,
            allow_compatibility: true,
            permissions: PermissionState::BothMissing,
            fail_recovery: false,
            compatibility_calls: 0,
            stopped_epoch: None,
            stale_compatibility_epoch: false,
        }
    }
    fn installed(&self, new: bool) {
        for role in Role::COMPONENTS {
            let path = match role {
                Role::Control => &self.request.new_receipt.components.control,
                Role::Ime => &self.request.new_receipt.components.ime,
                _ => &self.request.new_receipt.components.settings,
            };
            if !new && self.old.is_none() {
                assert!(!path.exists());
            } else {
                assert_eq!(
                    fs::read(path.join("Contents/MacOS/main")).unwrap(),
                    if new { b"new-code" } else { b"old-code" }
                );
            }
        }
        assert_eq!(
            fs::read(&self.data).unwrap(),
            b"original + new revisions + privacy tombstones"
        );
        for artifact in &self.request.artifacts {
            assert_eq!(
                fingerprint(&artifact.source, unsafe { libc::geteuid() }).unwrap(),
                artifact.expected
            );
        }
    }
}

// 明确的合成适配器：只提供夹具证据，不能用于 Security/TIS/SQLite 产品验收。
struct SyntheticNative {
    root: PathBuf,
    uid: u32,
    fail_postcheck: bool,
    allow_compatibility: bool,
    permissions: PermissionState,
    fail_recovery: bool,
    compatibility_calls: usize,
    stopped_epoch: Option<String>,
    stale_compatibility_epoch: bool,
}
impl SyntheticNative {
    fn file(&self, path: PathBuf, text: &[u8]) -> FileEvidence {
        if !path.exists() {
            write(&path, text);
        }
        FileEvidence {
            fingerprint: fingerprint(&path, self.uid).unwrap(),
            path,
        }
    }
}
impl NativeAdapter for SyntheticNative {
    fn prepare_recovery_environment(
        &mut self,
        subject: &Subject,
        root: &Path,
    ) -> Result<RecoveryEnvironmentReceipt> {
        if self.fail_recovery {
            return Err(Error::Adapter("bootstrap unavailable"));
        }
        Ok(RecoveryEnvironmentReceipt {
            subject: subject.clone(),
            bootstrap: self.file(root.join("bootstrap"), b"synthetic-nonexecutable-bootstrap"),
            helper: self.file(
                root.join("versions/test/helper"),
                b"synthetic-nonexecutable-helper",
            ),
            recovery_registration_sha256: "a".repeat(64),
            journal_contract: 1,
        })
    }
    fn verify_artifacts(
        &mut self,
        subject: &Subject,
        entries: &[Entry],
        purpose: VerificationPurpose,
    ) -> Result<VerificationReceipt> {
        for entry in entries.iter().filter(|entry| entry.role != Role::Receipt) {
            let path = if !matches!(
                purpose,
                VerificationPurpose::StagedNew | VerificationPurpose::DownloadedNew
            ) {
                &entry.destination
            } else {
                &entry.stage
            };
            if fingerprint(path, self.uid)? != entry.new {
                return Err(Error::ArtifactMismatch);
            }
        }
        Ok(VerificationReceipt {
            subject: subject.clone(),
            roles: [Role::Control, Role::Ime, Role::Settings, Role::PairManifest]
                .into_iter()
                .collect(),
            artifact_set_sha256: artifact_set_digest(entries)?,
            checks: [
                VerificationCheck::SecurityAllArchitectures,
                VerificationCheck::HardenedRuntime,
                VerificationCheck::ReleaseSignature,
                VerificationCheck::PairSignature,
                VerificationCheck::BundleMetadata,
            ]
            .into_iter()
            .collect(),
            evidence_sha256: "b".repeat(64),
        })
    }
    fn capture_environment(&mut self, subject: &Subject) -> Result<EnvironmentSnapshot> {
        Ok(EnvironmentSnapshot {
            subject: subject.clone(),
            previously_running_roles: [Role::Ime].into_iter().collect(),
            previous_input_source: "com.apple.keylayout.ABC".into(),
            process_snapshot_sha256: "c".repeat(64),
        })
    }
    fn quiesce(
        &mut self,
        subject: &Subject,
        marker: &MaintenanceMarker,
        previous: &EnvironmentSnapshot,
    ) -> Result<QuiescenceReceipt> {
        assert!(self.root.join("maintenance.json").is_file());
        self.stopped_epoch = Some(marker.epoch.clone());
        Ok(QuiescenceReceipt {
            subject: subject.clone(),
            epoch: marker.epoch.clone(),
            stopped_roles: roles(),
            previously_running_roles: previous.previously_running_roles.clone(),
            previous_input_source: previous.previous_input_source.clone(),
            process_snapshot_sha256: "d".repeat(64),
        })
    }
    fn assert_quiesced(&mut self, _: &Subject, _: &MaintenanceMarker) -> Result<()> {
        assert!(self.root.join("maintenance.json").is_file());
        Ok(())
    }
    fn snapshot(
        &mut self,
        subject: &Subject,
        marker: &MaintenanceMarker,
        directory: &Path,
    ) -> Result<SnapshotReceipt> {
        Ok(SnapshotReceipt {
            subject: subject.clone(),
            epoch: marker.epoch.clone(),
            covers: DataDomain::all(),
            files: vec![self.file(
                directory.join("fixture-snapshot.json"),
                b"synthetic-snapshot-not-sqlite-proof",
            )],
            consistency_evidence_sha256: "e".repeat(64),
        })
    }
    fn postcheck(
        &mut self,
        subject: &Subject,
        plan: &PreparedPlan,
        marker: &MaintenanceMarker,
    ) -> Result<PostcheckReceipt> {
        assert!(self.root.join("maintenance.json").is_file());
        if self.fail_postcheck {
            return Err(Error::Adapter("synthetic handshake failed"));
        }
        let entry = plan
            .entries
            .iter()
            .find(|entry| entry.role == Role::Receipt)
            .unwrap();
        assert_eq!(
            InstallationReceipt::parse(&fs::read(&entry.destination).unwrap()).unwrap(),
            plan.request.new_receipt
        );
        Ok(PostcheckReceipt {
            subject: subject.clone(),
            release_id: plan.request.new_receipt.release_id.clone(),
            profile_id: plan.request.new_receipt.profile_id.clone(),
            receipt_fingerprint: fingerprint(&entry.destination, self.uid)?,
            epoch: marker.epoch.clone(),
            handshake_and_schema_evidence_sha256: "f".repeat(64),
            permissions: self.permissions.clone(),
        })
    }
    fn rollback_compatibility(
        &mut self,
        subject: &Subject,
        old: &InstallationReceipt,
        marker: &MaintenanceMarker,
    ) -> Result<RollbackCompatibilityReceipt> {
        assert_eq!(self.stopped_epoch.as_ref(), Some(&marker.epoch));
        assert!(self.root.join("maintenance.json").is_file());
        self.compatibility_calls += 1;
        if !self.allow_compatibility {
            return Err(Error::Adapter("no proven write/outbox/privacy rollback"));
        }
        Ok(RollbackCompatibilityReceipt {
            subject: subject.clone(),
            epoch: if self.stale_compatibility_epoch {
                "stale-epoch".into()
            } else {
                marker.epoch.clone()
            },
            target_release_id: old.release_id.clone(),
            tested_domains: DataDomain::all(),
            read_write_outbox_privacy_evidence_sha256: "1".repeat(64),
        })
    }
    fn restore_environment(
        &mut self,
        subject: &Subject,
        prior: &QuiescenceReceipt,
        _: bool,
    ) -> Result<RestorationReceipt> {
        let journal: Journal = serde_json::from_slice(
            &fs::read(
                self.root
                    .join("transactions")
                    .join(&subject.transaction_id)
                    .join("journal.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(matches!(
            journal.phase,
            Phase::Committed | Phase::RollingBack
        ));
        Ok(RestorationReceipt {
            subject: subject.clone(),
            input_source: InputSourceRestoration::NeedsUserAction,
            restored_running_roles: prior.previously_running_roles.clone(),
            evidence_sha256: "2".repeat(64),
        })
    }
}
#[derive(Default)]
struct Trace {
    points: Vec<FaultPoint>,
    fail: Option<usize>,
}
impl FaultInjector for Trace {
    fn checkpoint(&mut self, point: FaultPoint) -> Result<()> {
        self.points.push(point);
        if self.fail == Some(self.points.len() - 1) {
            Err(Error::InjectedCrash)
        } else {
            Ok(())
        }
    }
}

#[test]
fn prepare_is_read_only_and_missing_bootstrap_never_moves_components() {
    let fixture = Fixture::new(true);
    let plan = fixture.prepare();
    assert!(!fixture.updater.root().exists());
    let mut native = fixture.adapter();
    native.fail_recovery = true;
    let mut transaction = fixture.updater.begin(plan).unwrap();
    assert!(transaction
        .run(RecoveryPolicy::Resume, &mut native, &mut NoFaults)
        .is_err());
    fixture.installed(false);
    assert!(!fixture.updater.maintenance_path().exists());
}
#[test]
fn new_install_and_update_commit_receipt_before_postcheck_and_permissions_are_not_corruption() {
    for update in [false, true] {
        let fixture = Fixture::new(update);
        let mut transaction = fixture.updater.begin(fixture.prepare()).unwrap();
        transaction
            .run(
                RecoveryPolicy::Resume,
                &mut fixture.adapter(),
                &mut NoFaults,
            )
            .unwrap();
        assert_eq!(transaction.journal().phase, Phase::WritesReleased);
        assert!(transaction.journal().writes_released);
        assert_eq!(
            transaction
                .journal()
                .postcheck
                .as_ref()
                .unwrap()
                .permissions,
            PermissionState::BothMissing
        );
        assert_eq!(
            transaction
                .journal()
                .restoration
                .as_ref()
                .unwrap()
                .input_source,
            InputSourceRestoration::NeedsUserAction
        );
        assert!(!fixture.updater.maintenance_path().exists());
        fixture.installed(true);
        drop(transaction);
        fixture
            .updater
            .recover(
                &fixture.request.transaction_id,
                RecoveryPolicy::Resume,
                &mut fixture.adapter(),
                &mut NoFaults,
            )
            .unwrap();
        fixture.installed(true);
    }
}
#[test]
fn lock_blocks_concurrent_writer_and_completed_transaction_id_cannot_be_reused() {
    let fixture = Fixture::new(true);
    let plan = fixture.prepare();
    let mut first = fixture.updater.begin(plan.clone()).unwrap();
    assert!(matches!(
        fixture.updater.begin(plan.clone()),
        Err(Error::Busy)
    ));
    first
        .run(
            RecoveryPolicy::Resume,
            &mut fixture.adapter(),
            &mut NoFaults,
        )
        .unwrap();
    drop(first);
    assert!(fixture.updater.begin(plan).is_err());
}
#[test]
fn every_update_rename_boundary_recovers_idempotently_without_losing_original_sources() {
    let baseline = Fixture::new(true);
    let mut trace = Trace::default();
    baseline
        .updater
        .begin(baseline.prepare())
        .unwrap()
        .run(RecoveryPolicy::Resume, &mut baseline.adapter(), &mut trace)
        .unwrap();
    let cases: Vec<_> = trace
        .points
        .iter()
        .enumerate()
        .filter_map(|(index, point)| {
            matches!(
                point,
                FaultPoint::BeforeRename(_) | FaultPoint::AfterRename(_)
            )
            .then_some((index, point.clone()))
        })
        .collect();
    assert_eq!(cases.len(), 28);
    for (index, point) in cases {
        let fixture = Fixture::new(true);
        let mut transaction = fixture.updater.begin(fixture.prepare()).unwrap();
        let mut crash = Trace {
            fail: Some(index),
            ..Default::default()
        };
        assert!(
            matches!(
                transaction.run(RecoveryPolicy::Resume, &mut fixture.adapter(), &mut crash),
                Err(Error::InjectedCrash)
            ),
            "{point:?}"
        );
        drop(transaction);
        fixture
            .updater
            .recover(
                &fixture.request.transaction_id,
                RecoveryPolicy::Resume,
                &mut fixture.adapter(),
                &mut NoFaults,
            )
            .unwrap_or_else(|e| panic!("{point:?}: {e:?}"));
        fixture
            .updater
            .recover(
                &fixture.request.transaction_id,
                RecoveryPolicy::Resume,
                &mut fixture.adapter(),
                &mut NoFaults,
            )
            .unwrap();
        fixture.installed(true);
    }
}
#[test]
fn all_journal_boundaries_recover_from_durable_facts() {
    let baseline = Fixture::new(true);
    let mut trace = Trace::default();
    baseline
        .updater
        .begin(baseline.prepare())
        .unwrap()
        .run(RecoveryPolicy::Resume, &mut baseline.adapter(), &mut trace)
        .unwrap();
    for (index, point) in trace.points.into_iter().enumerate().filter(|(_, point)| {
        matches!(
            point,
            FaultPoint::BeforeJournal(_) | FaultPoint::AfterJournal(_)
        )
    }) {
        let fixture = Fixture::new(true);
        let mut transaction = fixture.updater.begin(fixture.prepare()).unwrap();
        let mut crash = Trace {
            fail: Some(index),
            ..Default::default()
        };
        assert!(
            matches!(
                transaction.run(RecoveryPolicy::Resume, &mut fixture.adapter(), &mut crash),
                Err(Error::InjectedCrash)
            ),
            "{point:?}"
        );
        drop(transaction);
        fixture
            .updater
            .recover(
                &fixture.request.transaction_id,
                RecoveryPolicy::Resume,
                &mut fixture.adapter(),
                &mut NoFaults,
            )
            .unwrap_or_else(|e| panic!("{index} {point:?}: {e:?}"));
        fixture.installed(true);
    }
}
#[test]
fn failed_postcheck_rolls_back_binaries_without_restoring_or_deleting_user_data() {
    let fixture = Fixture::new(true);
    let mut native = fixture.adapter();
    native.fail_postcheck = true;
    let mut transaction = fixture.updater.begin(fixture.prepare()).unwrap();
    assert!(transaction
        .run(RecoveryPolicy::Resume, &mut native, &mut NoFaults)
        .is_err());
    assert!(fixture.updater.maintenance_path().is_file());
    drop(transaction);
    let result = fixture
        .updater
        .recover(
            &fixture.request.transaction_id,
            RecoveryPolicy::BinaryRollback,
            &mut fixture.adapter(),
            &mut NoFaults,
        )
        .unwrap();
    assert_eq!(result.phase, Phase::RolledBack);
    assert!(!fixture.updater.maintenance_path().exists());
    fixture.installed(false);
    fixture
        .updater
        .recover(
            &fixture.request.transaction_id,
            RecoveryPolicy::Resume,
            &mut fixture.adapter(),
            &mut NoFaults,
        )
        .unwrap();
    fixture.installed(false);
}
#[test]
fn already_released_writes_need_proven_compatible_rollback() {
    let fixture = Fixture::new(true);
    let mut transaction = fixture.updater.begin(fixture.prepare()).unwrap();
    transaction
        .run(
            RecoveryPolicy::Resume,
            &mut fixture.adapter(),
            &mut NoFaults,
        )
        .unwrap();
    drop(transaction);
    let mut reject = fixture.adapter();
    reject.allow_compatibility = false;
    assert!(fixture
        .updater
        .recover(
            &fixture.request.transaction_id,
            RecoveryPolicy::BinaryRollback,
            &mut reject,
            &mut NoFaults
        )
        .is_err());
    fixture.installed(true);
    fixture
        .updater
        .recover(
            &fixture.request.transaction_id,
            RecoveryPolicy::BinaryRollback,
            &mut fixture.adapter(),
            &mut NoFaults,
        )
        .unwrap();
    fixture.installed(false);
}
#[test]
fn rollback_compatibility_requires_current_quiescence_epoch_and_is_rechecked_after_crash() {
    let fixture = Fixture::new(true);
    let mut transaction = fixture.updater.begin(fixture.prepare()).unwrap();
    transaction
        .run(
            RecoveryPolicy::Resume,
            &mut fixture.adapter(),
            &mut NoFaults,
        )
        .unwrap();
    drop(transaction);
    let mut stale = fixture.adapter();
    stale.stale_compatibility_epoch = true;
    let mut trace = Trace::default();
    assert!(matches!(
        fixture.updater.recover(
            &fixture.request.transaction_id,
            RecoveryPolicy::BinaryRollback,
            &mut stale,
            &mut trace
        ),
        Err(Error::Adapter(_))
    ));
    assert_eq!(stale.compatibility_calls, 1);
    assert!(!trace
        .points
        .iter()
        .any(|point| matches!(point, FaultPoint::BeforeRename(_))));
    assert!(fixture.updater.maintenance_path().is_file());
    fixture.installed(true);

    struct AfterCompatibility(PathBuf);
    impl FaultInjector for AfterCompatibility {
        fn checkpoint(&mut self, point: FaultPoint) -> Result<()> {
            if matches!(point, FaultPoint::AfterJournal(_)) {
                let journal: Journal = serde_json::from_slice(&fs::read(&self.0)?)?;
                if journal.rollback_compatibility.is_some() {
                    return Err(Error::InjectedCrash);
                }
            }
            Ok(())
        }
    }
    let journal_path = fixture
        .updater
        .root()
        .join("transactions")
        .join(&fixture.request.transaction_id)
        .join("journal.json");
    assert!(matches!(
        fixture.updater.recover(
            &fixture.request.transaction_id,
            RecoveryPolicy::Resume,
            &mut fixture.adapter(),
            &mut AfterCompatibility(journal_path)
        ),
        Err(Error::InjectedCrash)
    ));
    let mut changed_data = fixture.adapter();
    changed_data.allow_compatibility = false;
    let mut trace = Trace::default();
    assert!(matches!(
        fixture.updater.recover(
            &fixture.request.transaction_id,
            RecoveryPolicy::Resume,
            &mut changed_data,
            &mut trace
        ),
        Err(Error::Adapter(_))
    ));
    assert_eq!(changed_data.compatibility_calls, 1);
    assert!(!trace
        .points
        .iter()
        .any(|point| matches!(point, FaultPoint::BeforeRename(_))));
    fixture.installed(true);
}

#[test]
fn changed_target_after_crash_is_retained_and_not_overwritten_by_recovery() {
    let fixture = Fixture::new(true);
    let plan = fixture.prepare();
    let control = plan
        .entries
        .iter()
        .find(|entry| entry.role == Role::Control)
        .unwrap()
        .clone();
    let mut trace = Trace::default();
    let baseline = Fixture::new(true);
    baseline
        .updater
        .begin(baseline.prepare())
        .unwrap()
        .run(RecoveryPolicy::Resume, &mut baseline.adapter(), &mut trace)
        .unwrap();
    let index = trace
        .points
        .iter()
        .position(|p| {
            *p == FaultPoint::AfterRename(Action::Backup {
                role: Role::Control,
            })
        })
        .unwrap();
    let mut transaction = fixture.updater.begin(plan).unwrap();
    let _ = transaction.run(
        RecoveryPolicy::Resume,
        &mut fixture.adapter(),
        &mut Trace {
            fail: Some(index),
            ..Default::default()
        },
    );
    drop(transaction);
    component(&control.destination, "unexpected-user-file");
    assert!(fixture
        .updater
        .recover(
            &fixture.request.transaction_id,
            RecoveryPolicy::Resume,
            &mut fixture.adapter(),
            &mut NoFaults
        )
        .is_err());
    assert_eq!(
        fs::read(control.destination.join("Contents/MacOS/main")).unwrap(),
        b"unexpected-user-file"
    );
    assert_eq!(
        fs::read(control.backup.join("Contents/MacOS/main")).unwrap(),
        b"old-code"
    );
}
#[test]
fn archive_boundary_and_hardlinks_are_rejected() {
    for path in ["../outside", "/absolute", "A/../../B", "A//B", "A\\B"] {
        assert!(validate_archive_entries(
            &[ArchiveEntry {
                path: path.into(),
                kind: ArchiveKind::File,
                unpacked_bytes: 1,
                link_target: None
            }],
            10
        )
        .is_err());
    }
    assert!(validate_archive_entries(
        &[
            ArchiveEntry {
                path: "A".into(),
                kind: ArchiveKind::File,
                unpacked_bytes: 1,
                link_target: None
            },
            ArchiveEntry {
                path: "A/B".into(),
                kind: ArchiveKind::File,
                unpacked_bytes: 1,
                link_target: None
            }
        ],
        10
    )
    .is_err());
    let fixture = Fixture::new(true);
    let artifact = &fixture.request.artifacts[0];
    let target = artifact.source.join("Contents/MacOS/main");
    fs::hard_link(&target, artifact.source.join("copy")).unwrap();
    assert!(fingerprint(&artifact.source, unsafe { libc::geteuid() }).is_err());
}

#[test]
fn bundle_internal_symlinks_keep_exact_structure_and_external_or_cyclic_links_fail() {
    let mut fixture = Fixture::new(true);
    let source = fixture.request.artifacts[0].source.clone();
    let framework = source.join("Contents/Frameworks/Sample.framework");
    write(&framework.join("Versions/A/Sample"), b"framework-code");
    std::os::unix::fs::symlink("A", framework.join("Versions/Current")).unwrap();
    std::os::unix::fs::symlink("Versions/Current/Sample", framework.join("Sample")).unwrap();
    fixture.request.artifacts[0].expected =
        fingerprint(&source, unsafe { libc::geteuid() }).unwrap();
    let mut transaction = fixture.updater.begin(fixture.prepare()).unwrap();
    transaction
        .run(
            RecoveryPolicy::Resume,
            &mut fixture.adapter(),
            &mut NoFaults,
        )
        .unwrap();
    let installed = fixture
        .request
        .new_receipt
        .components
        .control
        .join("Contents/Frameworks/Sample.framework/Sample");
    assert_eq!(
        fs::read_link(&installed).unwrap(),
        Path::new("Versions/Current/Sample")
    );
    assert_eq!(fs::read(&installed).unwrap(), b"framework-code");
    std::os::unix::fs::symlink("../../../../../../outside", framework.join("escape")).unwrap();
    assert!(fingerprint(&source, unsafe { libc::geteuid() }).is_err());
    fs::remove_file(framework.join("escape")).unwrap();
    std::os::unix::fs::symlink("loop-b", framework.join("loop-a")).unwrap();
    std::os::unix::fs::symlink("loop-a", framework.join("loop-b")).unwrap();
    assert!(fingerprint(&source, unsafe { libc::geteuid() }).is_err());
    let archive = vec![
        ArchiveEntry {
            path: "Versions/A/Code".into(),
            kind: ArchiveKind::File,
            unpacked_bytes: 4,
            link_target: None,
        },
        ArchiveEntry {
            path: "Current".into(),
            kind: ArchiveKind::Symlink,
            unpacked_bytes: 0,
            link_target: Some("Versions/A".into()),
        },
    ];
    validate_archive_entries(&archive, 32).unwrap();
    let mut bad = archive.clone();
    bad[1].link_target = Some("../outside".into());
    assert!(validate_archive_entries(&bad, 32).is_err());
}
#[test]
fn rollback_rename_crashes_continue_rollback_instead_of_reinstalling_new_set() {
    let baseline = Fixture::new(true);
    let mut native = baseline.adapter();
    native.fail_postcheck = true;
    let mut transaction = baseline.updater.begin(baseline.prepare()).unwrap();
    assert!(transaction
        .run(RecoveryPolicy::Resume, &mut native, &mut NoFaults)
        .is_err());
    drop(transaction);
    let mut trace = Trace::default();
    baseline
        .updater
        .recover(
            &baseline.request.transaction_id,
            RecoveryPolicy::BinaryRollback,
            &mut baseline.adapter(),
            &mut trace,
        )
        .unwrap();
    let cases: Vec<_> = trace
        .points
        .iter()
        .enumerate()
        .filter_map(|(index, point)| {
            matches!(
                point,
                FaultPoint::BeforeRename(_) | FaultPoint::AfterRename(_)
            )
            .then_some(index)
        })
        .collect();
    assert_eq!(cases.len(), 18);
    for index in cases {
        let fixture = Fixture::new(true);
        let mut native = fixture.adapter();
        native.fail_postcheck = true;
        let mut transaction = fixture.updater.begin(fixture.prepare()).unwrap();
        assert!(transaction
            .run(RecoveryPolicy::Resume, &mut native, &mut NoFaults)
            .is_err());
        drop(transaction);
        assert!(matches!(
            fixture.updater.recover(
                &fixture.request.transaction_id,
                RecoveryPolicy::BinaryRollback,
                &mut fixture.adapter(),
                &mut Trace {
                    fail: Some(index),
                    ..Default::default()
                }
            ),
            Err(Error::InjectedCrash)
        ));
        fixture
            .updater
            .recover(
                &fixture.request.transaction_id,
                RecoveryPolicy::Resume,
                &mut fixture.adapter(),
                &mut NoFaults,
            )
            .unwrap();
        fixture.installed(false);
    }
}
#[test]
fn source_or_backup_corruption_never_overwrites_old_or_new_files() {
    let fixture = Fixture::new(true);
    let plan = fixture.prepare();
    write(
        &fixture.request.artifacts[0]
            .source
            .join("Contents/MacOS/main"),
        b"changed-source",
    );
    assert!(fixture.updater.begin(plan).is_err());
    assert_eq!(
        fs::read(
            fixture
                .request
                .new_receipt
                .components
                .control
                .join("Contents/MacOS/main")
        )
        .unwrap(),
        b"old-code"
    );
    let fixture = Fixture::new(true);
    let plan = fixture.prepare();
    let control = plan
        .entries
        .iter()
        .find(|e| e.role == Role::Control)
        .unwrap()
        .clone();
    let mut transaction = fixture.updater.begin(plan).unwrap();
    transaction
        .run(
            RecoveryPolicy::Resume,
            &mut fixture.adapter(),
            &mut NoFaults,
        )
        .unwrap();
    drop(transaction);
    write(
        &control.backup.join("Contents/MacOS/main"),
        b"corrupted-backup",
    );
    assert!(fixture
        .updater
        .recover(
            &fixture.request.transaction_id,
            RecoveryPolicy::BinaryRollback,
            &mut fixture.adapter(),
            &mut NoFaults
        )
        .is_err());
    fixture.installed(true);
}
#[test]
fn missing_write_permissions_are_reported_separately() {
    let fixture = Fixture::new(true);
    let applications = fixture.home.join("Applications");
    fs::set_permissions(&applications, fs::Permissions::from_mode(0o500)).unwrap();
    let result = fixture.updater.prepare(fixture.request.clone());
    fs::set_permissions(&applications, fs::Permissions::from_mode(0o700)).unwrap();
    if unsafe { libc::geteuid() } != 0 {
        assert!(matches!(result, Err(Error::PermissionRequired)));
    }
}

#[test]
fn rollback_journal_and_marker_boundaries_restore_the_complete_old_set() {
    let baseline = Fixture::new(true);
    let mut native = baseline.adapter();
    native.fail_postcheck = true;
    let mut transaction = baseline.updater.begin(baseline.prepare()).unwrap();
    assert!(transaction
        .run(RecoveryPolicy::Resume, &mut native, &mut NoFaults)
        .is_err());
    drop(transaction);
    let mut trace = Trace::default();
    baseline
        .updater
        .recover(
            &baseline.request.transaction_id,
            RecoveryPolicy::BinaryRollback,
            &mut baseline.adapter(),
            &mut trace,
        )
        .unwrap();
    for (index, point) in trace.points.iter().enumerate().filter(|(_, point)| {
        matches!(
            point,
            FaultPoint::BeforeJournal(_)
                | FaultPoint::AfterJournal(_)
                | FaultPoint::BeforeMarkerRemoval { .. }
                | FaultPoint::AfterMarkerRemoval { .. }
        )
    }) {
        let fixture = Fixture::new(true);
        let mut native = fixture.adapter();
        native.fail_postcheck = true;
        let mut transaction = fixture.updater.begin(fixture.prepare()).unwrap();
        assert!(transaction
            .run(RecoveryPolicy::Resume, &mut native, &mut NoFaults)
            .is_err());
        drop(transaction);
        assert!(
            matches!(
                fixture.updater.recover(
                    &fixture.request.transaction_id,
                    RecoveryPolicy::BinaryRollback,
                    &mut fixture.adapter(),
                    &mut Trace {
                        fail: Some(index),
                        ..Default::default()
                    }
                ),
                Err(Error::InjectedCrash)
            ),
            "{index} {point:?}"
        );
        // 首次回滚请求尚未耐久时重试原请求；已耐久后普通恢复必须继续回滚。
        let policy = if fixture
            .updater
            .inspect(&fixture.request.transaction_id)
            .unwrap()
            .rollback_requested
        {
            RecoveryPolicy::Resume
        } else {
            RecoveryPolicy::BinaryRollback
        };
        fixture
            .updater
            .recover(
                &fixture.request.transaction_id,
                policy,
                &mut fixture.adapter(),
                &mut NoFaults,
            )
            .unwrap_or_else(|error| panic!("{index} {point:?}: {error:?}"));
        fixture.installed(false);
        assert!(!fixture.updater.maintenance_path().exists());
    }
}
#[test]
fn write_release_marker_boundary_never_loses_possible_new_write_history() {
    let baseline = Fixture::new(true);
    let mut trace = Trace::default();
    baseline
        .updater
        .begin(baseline.prepare())
        .unwrap()
        .run(RecoveryPolicy::Resume, &mut baseline.adapter(), &mut trace)
        .unwrap();
    for (index, point) in trace.points.iter().enumerate().filter(|(_, point)| {
        matches!(
            point,
            FaultPoint::BeforeMarkerRemoval { rolled_back: false }
                | FaultPoint::AfterMarkerRemoval { rolled_back: false }
        )
    }) {
        let fixture = Fixture::new(true);
        let mut transaction = fixture.updater.begin(fixture.prepare()).unwrap();
        assert!(
            matches!(
                transaction.run(
                    RecoveryPolicy::Resume,
                    &mut fixture.adapter(),
                    &mut Trace {
                        fail: Some(index),
                        ..Default::default()
                    }
                ),
                Err(Error::InjectedCrash)
            ),
            "{point:?}"
        );
        drop(transaction);
        assert!(
            fixture
                .updater
                .inspect(&fixture.request.transaction_id)
                .unwrap()
                .writes_released
        );
        let mut native = fixture.adapter();
        native.allow_compatibility = false;
        assert!(fixture
            .updater
            .recover(
                &fixture.request.transaction_id,
                RecoveryPolicy::BinaryRollback,
                &mut native,
                &mut NoFaults
            )
            .is_err());
        assert_eq!(native.compatibility_calls, 1);
        fixture.installed(true);
    }
}
#[test]
fn inconsistent_released_flag_and_absent_maintenance_marker_cannot_bypass_rollback_gate() {
    let fixture = Fixture::new(true);
    let mut transaction = fixture.updater.begin(fixture.prepare()).unwrap();
    transaction
        .run(
            RecoveryPolicy::Resume,
            &mut fixture.adapter(),
            &mut NoFaults,
        )
        .unwrap();
    drop(transaction);
    let path = fixture
        .updater
        .root()
        .join("transactions")
        .join(&fixture.request.transaction_id)
        .join("journal.json");
    let mut journal: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    journal["writes_released"] = false.into();
    write(&path, &serde_json::to_vec(&journal).unwrap());
    let mut trace = Trace::default();
    let mut native = fixture.adapter();
    native.allow_compatibility = false;
    assert!(matches!(
        fixture.updater.recover(
            &fixture.request.transaction_id,
            RecoveryPolicy::BinaryRollback,
            &mut native,
            &mut trace
        ),
        Err(Error::Invalid(_))
    ));
    assert_eq!(native.compatibility_calls, 0);
    assert!(trace.points.is_empty());
    fixture.installed(true);

    let fixture = Fixture::new(true);
    let mut native = fixture.adapter();
    native.fail_postcheck = true;
    let mut transaction = fixture.updater.begin(fixture.prepare()).unwrap();
    assert!(transaction
        .run(RecoveryPolicy::Resume, &mut native, &mut NoFaults)
        .is_err());
    drop(transaction);
    fs::remove_file(fixture.updater.maintenance_path()).unwrap();
    write(&fixture.data, b"new data after unexpectedly absent marker");
    let mut trace = Trace::default();
    assert!(matches!(
        fixture.updater.recover(
            &fixture.request.transaction_id,
            RecoveryPolicy::BinaryRollback,
            &mut fixture.adapter(),
            &mut trace
        ),
        Err(Error::NeedsRepair(_))
    ));
    assert!(!trace.points.iter().any(|point| matches!(
        point,
        FaultPoint::BeforeRename(_) | FaultPoint::AfterRename(_)
    )));
    assert_eq!(
        fs::read(fixture.data).unwrap(),
        b"new data after unexpectedly absent marker"
    );
}
#[test]
fn rejected_pending_rollback_blocks_the_next_transaction_and_legacy_scope_is_explicit() {
    let fixture = Fixture::new(true);
    let mut transaction = fixture.updater.begin(fixture.prepare()).unwrap();
    transaction
        .run(
            RecoveryPolicy::Resume,
            &mut fixture.adapter(),
            &mut NoFaults,
        )
        .unwrap();
    drop(transaction);
    let mut native = fixture.adapter();
    native.allow_compatibility = false;
    assert!(fixture
        .updater
        .recover(
            &fixture.request.transaction_id,
            RecoveryPolicy::BinaryRollback,
            &mut native,
            &mut NoFaults
        )
        .is_err());
    let mut next = fixture.request.clone();
    next.transaction_id = "44444444-4444-4444-8444-444444444444".into();
    next.new_receipt.release_id = "inputia-next".into();
    assert!(matches!(
        fixture
            .updater
            .begin(fixture.updater.prepare(next).unwrap()),
        Err(Error::Busy)
    ));
    let mut legacy = fixture.request.clone();
    legacy.new_receipt.scope = InstallationScope::LegacySingleUser;
    legacy.new_receipt.components.control = "/Applications/Inputia.app".into();
    assert!(matches!(
        fixture.updater.prepare(legacy),
        Err(Error::PermissionRequired)
    ));
}

#[test]
fn actual_second_process_cannot_acquire_the_transaction_lock() {
    if let Ok(home) = std::env::var("INPUTIA_UPDATER_SYNTHETIC_LOCK_HOME") {
        let updater = Updater::new(home.into(), unsafe { libc::geteuid() }).unwrap();
        let plan: PreparedPlan = serde_json::from_slice(
            &fs::read(std::env::var("INPUTIA_UPDATER_SYNTHETIC_LOCK_PLAN").unwrap()).unwrap(),
        )
        .unwrap();
        assert!(matches!(updater.begin(plan), Err(Error::Busy)));
        return;
    }
    let fixture = Fixture::new(true);
    let plan = fixture.prepare();
    let path = fixture._temp.path().join("synthetic-plan.json");
    write(&path, &serde_json::to_vec(&plan).unwrap());
    let _transaction = fixture.updater.begin(plan).unwrap();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "actual_second_process_cannot_acquire_the_transaction_lock",
            "--nocapture",
        ])
        .env("INPUTIA_UPDATER_SYNTHETIC_LOCK_HOME", &fixture.home)
        .env("INPUTIA_UPDATER_SYNTHETIC_LOCK_PLAN", path)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
#[test]
fn unchanged_component_bytes_still_have_a_recoverable_backup() {
    let mut fixture = Fixture::new(true);
    let artifact = fixture
        .request
        .artifacts
        .iter_mut()
        .find(|artifact| artifact.role == Role::Settings)
        .unwrap();
    component(&artifact.source, "old-code");
    artifact.expected = fingerprint(&artifact.source, unsafe { libc::geteuid() }).unwrap();
    let plan = fixture.prepare();
    let unchanged = plan
        .entries
        .iter()
        .find(|entry| entry.role == Role::Settings)
        .unwrap()
        .clone();
    assert_eq!(unchanged.old.as_ref(), Some(&unchanged.new));
    fixture
        .updater
        .begin(plan)
        .unwrap()
        .run(
            RecoveryPolicy::Resume,
            &mut fixture.adapter(),
            &mut NoFaults,
        )
        .unwrap();
    assert_eq!(
        fingerprint(&unchanged.backup, unsafe { libc::geteuid() }).unwrap(),
        unchanged.new
    );
}
#[test]
fn crash_after_persisting_rollback_request_keeps_exclusive_transaction_ownership() {
    let baseline = Fixture::new(true);
    let mut transaction = baseline.updater.begin(baseline.prepare()).unwrap();
    transaction
        .run(
            RecoveryPolicy::Resume,
            &mut baseline.adapter(),
            &mut NoFaults,
        )
        .unwrap();
    drop(transaction);
    let mut trace = Trace::default();
    baseline
        .updater
        .recover(
            &baseline.request.transaction_id,
            RecoveryPolicy::BinaryRollback,
            &mut baseline.adapter(),
            &mut trace,
        )
        .unwrap();
    let index = trace
        .points
        .iter()
        .enumerate()
        .filter(|(_, point)| **point == FaultPoint::AfterJournal(Phase::WritesReleased))
        .nth(1)
        .unwrap()
        .0;
    let fixture = Fixture::new(true);
    let mut transaction = fixture.updater.begin(fixture.prepare()).unwrap();
    transaction
        .run(
            RecoveryPolicy::Resume,
            &mut fixture.adapter(),
            &mut NoFaults,
        )
        .unwrap();
    drop(transaction);
    assert!(matches!(
        fixture.updater.recover(
            &fixture.request.transaction_id,
            RecoveryPolicy::BinaryRollback,
            &mut fixture.adapter(),
            &mut Trace {
                fail: Some(index),
                ..Default::default()
            }
        ),
        Err(Error::InjectedCrash)
    ));
    assert!(
        fixture
            .updater
            .inspect(&fixture.request.transaction_id)
            .unwrap()
            .rollback_requested
    );
    let mut next = fixture.request.clone();
    next.transaction_id = "44444444-4444-4444-8444-444444444444".into();
    next.new_receipt.release_id = "inputia-next".into();
    assert!(matches!(
        fixture
            .updater
            .begin(fixture.updater.prepare(next).unwrap()),
        Err(Error::Busy)
    ));
}
