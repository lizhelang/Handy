//! 固定启动日志的第三版。此模块尚未启用 App 的 pending 协议。
use super::*;
mod model;
mod restore;
use inputia_settings::store::{LedgerActivationIntent, TransitionIntent};
pub use model::StartupPurpose;
use model::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupNeed {
    NoWork,
    FullStartup,
    SettingsProtocol,
    SettingsReplay,
}
/// 仅是一次观察锚点；调用方仍须在最终 Files 锁内核协议状态和这些准确字节。
#[derive(Debug, Clone)]
pub struct StartupObservation {
    members: Vec<Member>,
}
impl StartupObservation {
    pub fn recheck(&self, roots: &[MigrationSourceRoot]) -> Result<()> {
        anyhow::ensure!(
            observe_members(roots, &self.members)? == self.members,
            "startup observation changed"
        );
        Ok(())
    }
    pub fn digest(&self) -> Result<String> {
        Ok(sha(&serde_json::to_vec(&self.members)?))
    }
}
pub enum StartupPreparation {
    NoWork(StartupObservation),
    Attempt(StartupTransaction),
}
/// Unknown/Expired 不属于可完成结果；配置恢复从来不重放设备、模型或输入副作用。
pub enum StartupConfirmation {
    FullStartupReady,
    ProtocolReady,
    RequestSaved { operation_id: String },
    RequestConflict { operation_id: String },
}
pub struct StartupTransaction {
    _lock: Option<StartupLock>,
    roots: Vec<MigrationSourceRoot>,
    backup: PathBuf,
    outcome: MigrationBackupOutcome,
    journal: JournalV3,
}
impl StartupTransaction {
    pub fn purpose(&self) -> StartupPurpose {
        self.journal.purpose
    }
    pub fn begin_mutations(&mut self) -> Result<()> {
        self.validate()?;
        verify_backup(&self.outcome)?;
        anyhow::ensure!(
            self.journal.phase == Phase::Prepared
                && observe_members(&self.roots, &self.journal.members)? == self.journal.members,
            "startup no longer matches prepared snapshot"
        );
        self.journal.phase = Phase::Mutating;
        self.persist()?;
        fault("v3_armed")
    }
    pub fn record_settings_initialization(&mut self, intent: &InitializationIntent) -> Result<()> {
        // 重试必须用原 intent，从此前 frontier 构造，而非把上次 after 当成新 before。
        let mut base = self.journal.clone();
        if matches!(base.authorizations.last().map(|a|&a.owner),Some(Owner::Initialize{domain,..}) if domain==&intent.domain)
        {
            base.authorizations.pop();
        }
        self.record(model::initialization(&base, intent)?)
    }
    pub fn record_protocol_activation(&mut self, intent: &LedgerActivationIntent) -> Result<()> {
        self.record(model::activation(&self.journal, intent)?)
    }
    pub fn record_settings_transition(&mut self, intent: &TransitionIntent) -> Result<()> {
        self.record(model::transition(&self.journal, intent)?)
    }
    fn record(&mut self, mut authorization: Authorization) -> Result<()> {
        anyhow::ensure!(
            self.journal.phase == Phase::Mutating,
            "startup is not armed"
        );
        self.validate()?;
        let states = prefixes(&self.journal)?;
        let current = observed_digests(&self.roots, &self.journal.members)?;
        // 仅尾随的补同步许可可以跨越；不能重放已经被后续真实写入超越的许可。
        if let Some(index) = self
            .journal
            .authorizations
            .iter()
            .rposition(|a| a.reaffirm_prefix.is_none())
        {
            let previous = &self.journal.authorizations[index];
            authorization.sequence = previous.sequence;
            if &authorization == previous {
                let first = self.journal.authorizations[..index]
                    .iter()
                    .filter(|a| a.reaffirm_prefix.is_none())
                    .map(|a| a.changes.len())
                    .sum::<usize>();
                anyhow::ensure!(
                    states[first..=first + previous.changes.len()].contains(&current),
                    "retry source is not an authorized interruption"
                );
                self.persist()?;
                return fault("v3_authorized");
            }
        }
        if matches!(
            authorization.owner,
            Owner::Transition {
                phase: StepPhase::PrepareRequest,
                ..
            }
        ) && authorization.changes.len() == 1
            && authorization.changes[0].before.as_ref() == Some(&authorization.changes[0].after)
        {
            authorization.reaffirm_prefix =
                Some(states.iter().rposition(|state| state == &current).context(
                    "durability reaffirmation source is not an authorized interruption",
                )?);
            if let Some(last) = self.journal.authorizations.last() {
                authorization.sequence = last.sequence;
                if &authorization == last {
                    self.persist()?;
                    return fault("v3_authorized");
                }
            }
        } else {
            anyhow::ensure!(
                states.last() == Some(&current),
                "new authorization cannot branch from an unfinished write"
            );
        }
        authorization.sequence = self.journal.authorizations.len() as u64 + 1;
        let mut next = self.journal.clone();
        next.authorizations.push(authorization);
        anyhow::ensure!(
            next.authorizations.len() <= 2048,
            "startup authorization bound exceeded"
        );
        prefixes(&next)?;
        self.journal = next;
        self.persist()?;
        fault("v3_authorized")
    }
    /// verify 必须执行相应领域的真实只读确认；不得把 CommitUncertain/Expired 映成 Ready。
    pub fn complete(
        &mut self,
        confirmation: StartupConfirmation,
        verify: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        anyhow::ensure!(
            self.journal.phase == Phase::Mutating,
            "startup is not mutating"
        );
        self.validate()?;
        let expected = prefixes(&self.journal)?
            .pop()
            .context("missing completed frontier")?;
        anyhow::ensure!(
            observed_digests(&self.roots, &self.journal.members)? == expected,
            "incomplete authorized write"
        );
        let request = match &confirmation {
            StartupConfirmation::RequestSaved { operation_id }
            | StartupConfirmation::RequestConflict { operation_id } => Some(operation_id),
            _ => None,
        };
        match (&self.journal.purpose, &confirmation) {
            (StartupPurpose::FullStartup, StartupConfirmation::FullStartupReady) => {}
            (StartupPurpose::SettingsProtocol, StartupConfirmation::ProtocolReady) => {
                anyhow::ensure!(
                    self.journal
                        .authorizations
                        .iter()
                        .any(|a| matches!(a.owner, Owner::Activate { .. })),
                    "activation not authorized"
                )
            }
            (
                StartupPurpose::SettingsReplay,
                StartupConfirmation::RequestSaved { .. }
                | StartupConfirmation::RequestConflict { .. },
            ) => {
                anyhow::ensure!(
                    matches!(self.journal.authorizations.iter().rfind(|a| a.reaffirm_prefix.is_none()).map(|a|&a.owner),Some(Owner::Transition{operation_id,phase:StepPhase::ResolveRequest,..}) if Some(operation_id)==request),
                    "request has not reached durable resolution"
                );
            }
            _ => anyhow::bail!("startup completion purpose mismatch"),
        }
        let mut pending = std::collections::BTreeMap::new();
        for binding in &self.journal.bindings {
            if let Some(request) = &binding.active_request {
                pending.insert(&request.operation_id, StepPhase::PrepareRequest);
            }
        }
        for a in &self.journal.authorizations {
            if a.reaffirm_prefix.is_some() {
                continue;
            }
            if let Owner::Transition {
                operation_id,
                phase,
                ..
            } = &a.owner
            {
                pending.insert(operation_id, *phase);
            }
        }
        anyhow::ensure!(
            pending.values().all(|p| *p == StepPhase::ResolveRequest),
            "request remains unresolved"
        );
        if let Some(operation) = request {
            let root = self
                .roots
                .iter()
                .find(|r| r.label == "handy")
                .context("control root missing")?;
            let raw: serde_json::Value = serde_json::from_slice(&read_bytes(
                &checked_path(&root.root, Path::new(CONTROL_SETTINGS_PENDING_NAME))?,
                SETTINGS_LIMIT,
            )?)?;
            let status = if matches!(confirmation, StartupConfirmation::RequestSaved { .. }) {
                "saved"
            } else {
                "conflict"
            };
            anyhow::ensure!(
                raw["state"]["phase"] == "resolved"
                    && raw["state"]["operation_id"] == operation.as_str()
                    && raw["state"]["outcome"]["status"] == status,
                "completion does not match the real request outcome"
            );
        }
        verify()?;
        anyhow::ensure!(
            observed_digests(&self.roots, &self.journal.members)? == expected,
            "settings changed during confirmation"
        );
        self.journal.phase = Phase::Completed;
        self.journal.completed = Some(expected);
        if self.journal.purpose == StartupPurpose::FullStartup {
            let mut evidence = self.journal.clone();
            evidence.full_completion = None;
            let relative = PathBuf::from(format!("full-startup-{}.json", self.journal.attempt_id));
            let bytes = serde_json::to_vec_pretty(&evidence)?;
            write_immutable(&checked_path(&self.backup, &relative)?, &bytes)?;
            self.journal.full_completion = Some(Evidence {
                relative,
                sha256: sha(&bytes),
            });
            fault("v3_full_evidence")?;
        }
        self.persist()?;
        fault("v3_completed")
    }
    fn persist(&self) -> Result<()> {
        anyhow::ensure!(
            serde_json::to_vec_pretty(&self.journal)?.len() as u64 <= JOURNAL_LIMIT,
            "startup journal exceeds bound"
        );
        write_json_atomically(&self.backup.join(JOURNAL), &self.journal)
    }
    fn validate(&self) -> Result<()> {
        validate(&self.journal, &self.backup, &self.roots).map(|_| ())
    }
}
impl Drop for StartupTransaction {
    fn drop(&mut self) {
        if matches!(self.journal.phase, Phase::Mutating | Phase::Restoring) {
            log::warn!("Startup settings recovery remains pending");
        }
    }
}

enum Versioned {
    V2(Journal),
    V3(JournalV3),
}
fn read_versioned(path: &Path) -> Result<Option<Versioned>> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
        Ok(_) => {}
    }
    #[derive(Deserialize)]
    struct Version {
        schema_version: u32,
    }
    let bytes = read_bytes(path, JOURNAL_LIMIT)?;
    let version: Version = serde_json::from_slice(&bytes)?;
    // 独立 deny_unknown_fields 类型重新解析同一批原始字节，拒重复字段和跨版本混装。
    match version.schema_version {
        2 => Ok(Some(Versioned::V2(serde_json::from_slice(&bytes)?))),
        3 => Ok(Some(Versioned::V3(serde_json::from_slice(&bytes)?))),
        _ => anyhow::bail!("unsupported startup schema"),
    }
}

/// 第一包只提供受控内核；现 App 调用仍为 schema 2，不能提前开启永久 pending 门禁。
pub(crate) fn prepare_with_state(
    handy: &Path,
    inputia: Option<&Path>,
    backup: &Path,
    lock: &Path,
    legacy: Option<&Path>,
    inspect: impl FnOnce() -> Result<StartupNeed>,
) -> Result<StartupPreparation> {
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
    let journal_path = checked_path(backup, Path::new(JOURNAL))?;
    let existing = read_versioned(&journal_path)
        .map_err(|e| classified(StartupFailureKind::RepairRequired, e))?;
    let (lock, full) = match existing {
        Some(Versioned::V2(journal)) => {
            ensure_legacy_pending_absent(&roots)
                .map_err(|e| classified(StartupFailureKind::RepairRequired, e))?;
            let outcome = validate_journal(&journal, backup, &roots)?;
            let mut old = StartupMigration {
                _lock: Some(lock),
                outcome,
                source_roots: roots.clone(),
                backup_root: backup.into(),
                journal,
            };
            if matches!(old.journal.phase, Phase::Mutating | Phase::Restoring) {
                validate_legacy_present_ownership(&old)
                    .map_err(|e| classified(StartupFailureKind::RepairRequired, e))?;
                recover(&mut old)
                    .map_err(|e| classified(StartupFailureKind::PendingRecovery, e))?;
            }
            open_read(&journal_path)?.sync_all()?;
            sync_dir(backup)?;
            fault("v3_legacy_terminal_synced")?;
            (
                old._lock.take().context("legacy startup lock missing")?,
                None,
            )
        }
        Some(Versioned::V3(journal)) => {
            let outcome = validate(&journal, backup, &roots)
                .map_err(|e| classified(StartupFailureKind::RepairRequired, e))?;
            let mut old = StartupTransaction {
                _lock: Some(lock),
                roots: roots.clone(),
                backup: backup.into(),
                outcome,
                journal,
            };
            if matches!(old.journal.phase, Phase::Mutating | Phase::Restoring) {
                restore::recover(&mut old)
                    .map_err(|e| classified(StartupFailureKind::PendingRecovery, e))?;
            }
            open_read(&journal_path)?.sync_all()?;
            sync_dir(backup)?;
            (
                old._lock.take().context("startup lock missing")?,
                old.journal.full_completion.clone(),
            )
        }
        None => {
            ensure_legacy_pending_absent(&roots)
                .map_err(|e| classified(StartupFailureKind::RepairRequired, e))?;
            anyhow::ensure!(
                !has_backup(backup)?,
                StartupFailure {
                    kind: StartupFailureKind::LegacyRecoveryRequired
                }
            );
            if let Some(path) = legacy {
                if has_backup(path)? {
                    verify_startup_marker(&path.join("complete.json"), &roots)
                        .map_err(|e| classified(StartupFailureKind::LegacyRecoveryRequired, e))?;
                }
            }
            (lock, None)
        }
    };
    let all = members(
        &roots,
        if full.is_none() {
            StartupPurpose::FullStartup
        } else {
            StartupPurpose::SettingsProtocol
        },
    )?;
    let need = inspect().map_err(|e| classified(StartupFailureKind::CleanPreflight, e))?;
    anyhow::ensure!(
        observe_members(&roots, &all)? == all,
        "settings changed during pure inspection"
    );
    if need == StartupNeed::NoWork && full.is_some() {
        return Ok(StartupPreparation::NoWork(StartupObservation {
            members: all,
        }));
    }
    let purpose = if full.is_none() {
        StartupPurpose::FullStartup
    } else {
        match need {
            StartupNeed::NoWork | StartupNeed::FullStartup => StartupPurpose::FullStartup,
            StartupNeed::SettingsProtocol => StartupPurpose::SettingsProtocol,
            StartupNeed::SettingsReplay => StartupPurpose::SettingsReplay,
        }
    };
    new_attempt(lock, roots, backup, purpose, full).map(StartupPreparation::Attempt)
}
fn specs(roots: &[MigrationSourceRoot], purpose: StartupPurpose) -> Result<Vec<Member>> {
    let mut result = vec![];
    for root in roots {
        if purpose != StartupPurpose::FullStartup && root.label != "handy" {
            continue;
        }
        let (domain, document, marker) = pair_spec(&root.label)?;
        let mut names = vec![document, marker];
        if root.label == "handy" {
            names.push(CONTROL_SETTINGS_PENDING_NAME);
        }
        for name in names {
            result.push(Member {
                root_label: root.label.clone(),
                domain: domain.into(),
                name: name.into(),
                original: Original::Absent,
            });
        }
    }
    Ok(result)
}
fn members(roots: &[MigrationSourceRoot], purpose: StartupPurpose) -> Result<Vec<Member>> {
    observe_members(roots, &specs(roots, purpose)?)
}
fn observe_members(roots: &[MigrationSourceRoot], members: &[Member]) -> Result<Vec<Member>> {
    members
        .iter()
        .map(|member| {
            let root = roots
                .iter()
                .find(|r| r.label == member.root_label)
                .context("settings root missing")?;
            let mut member = member.clone();
            let path = checked_path(&root.root, Path::new(&member.name))?;
            if let Ok(metadata) = fs::symlink_metadata(&path) {
                anyhow::ensure!(
                    metadata.len() <= SETTINGS_LIMIT,
                    "settings source exceeds size bound"
                );
            }
            member.original = observe(&path)?;
            if let Some(value) = member.digest() {
                value.validate()?;
            }
            Ok(member)
        })
        .collect()
}
fn observed_digests(
    roots: &[MigrationSourceRoot],
    members: &[Member],
) -> Result<Vec<Option<FileStamp>>> {
    Ok(observe_members(roots, members)?
        .iter()
        .map(Member::digest)
        .collect())
}
fn candidates(
    roots: &[MigrationSourceRoot],
    purpose: StartupPurpose,
) -> Result<Vec<MigrationPathCandidate>> {
    let mut result = if purpose == StartupPurpose::FullStartup {
        roots
            .iter()
            .flat_map(|r| {
                if r.label == "handy" {
                    handy_data_candidates(&r.label)
                } else {
                    inputia_data_candidates(&r.label)
                }
            })
            .collect()
    } else {
        specs(roots, purpose)?
            .into_iter()
            .filter(|m| m.name != CONTROL_SETTINGS_PENDING_NAME)
            .map(|m| MigrationPathCandidate {
                source_root_label: m.root_label,
                relative_path: m.name.into(),
                kind: MigrationPathKind::File,
            })
            .collect::<Vec<_>>()
    };
    result.push(MigrationPathCandidate {
        source_root_label: "handy".into(),
        relative_path: CONTROL_SETTINGS_PENDING_NAME.into(),
        kind: MigrationPathKind::File,
    });
    Ok(result)
}
fn bindings(roots: &[MigrationSourceRoot], members: &[Member]) -> Result<Vec<Binding>> {
    let mut result = vec![];
    for member in members
        .iter()
        .filter(|m| m.name == "settings_store.json" || m.name == "settings.json")
    {
        if member.original == Original::Absent {
            continue;
        }
        let root = roots
            .iter()
            .find(|r| r.label == member.root_label)
            .context("domain root missing")?;
        let raw: serde_json::Value = serde_json::from_slice(&read_bytes(
            &checked_path(&root.root, Path::new(&member.name))?,
            SETTINGS_LIMIT,
        )?)?;
        if let Some(meta) = raw.get("_inputia_store") {
            let store = meta
                .get("store_id")
                .and_then(|v| v.as_str())
                .context("store identity missing")?;
            let ledger = meta
                .get("pending_protocol")
                .map(|v| {
                    v.get("ledger_id")
                        .and_then(|v| v.as_str())
                        .context("ledger identity missing")
                })
                .transpose()?;
            let active_request = if let Some(ledger_id) = ledger {
                let raw: serde_json::Value = serde_json::from_slice(&read_bytes(
                    &checked_path(&root.root, Path::new(CONTROL_SETTINGS_PENDING_NAME))?,
                    SETTINGS_LIMIT,
                )?)?;
                anyhow::ensure!(
                    raw.get("domain").and_then(serde_json::Value::as_str)
                        == Some(member.domain.as_str())
                        && raw.get("store_id").and_then(serde_json::Value::as_str) == Some(store)
                        && raw.get("ledger_id").and_then(serde_json::Value::as_str)
                            == Some(ledger_id),
                    "baseline ledger identity mismatch"
                );
                if raw["state"]["phase"] == "active" {
                    Some(RequestBinding {
                        operation_id: raw["state"]["request"]["request"]["operation_id"]
                            .as_str()
                            .context("active operation missing")?
                            .into(),
                        request_digest: raw["state"]["request_digest"]
                            .as_str()
                            .context("active request digest missing")?
                            .into(),
                    })
                } else {
                    None
                }
            } else {
                None
            };
            result.push(Binding {
                domain: member.domain.clone(),
                store_id: store.into(),
                ledger_id: ledger.map(str::to_owned),
                active_request,
            });
        }
    }
    Ok(result)
}
fn new_attempt(
    lock: StartupLock,
    roots: Vec<MigrationSourceRoot>,
    backup: &Path,
    purpose: StartupPurpose,
    full: Option<Evidence>,
) -> Result<StartupTransaction> {
    let members = members(&roots, purpose)?;
    let bindings = bindings(&roots, &members)?;
    fault("before_backup")?;
    create_dirs_durable(backup)?;
    let outcome =
        prepare_backup_for_migration(&roots, &candidates(&roots, purpose)?, backup, GENERATION)?;
    durable_tree(&outcome.backup_dir)?;
    sync_dir(backup)?;
    fault("backup_created")?;
    anyhow::ensure!(
        observe_members(&roots, &members)? == members,
        "settings changed during snapshot"
    );
    let journal = JournalV3 {
        schema_version: 3,
        migration_id: GENERATION.into(),
        attempt_id: uuid::Uuid::new_v4().to_string(),
        roots_sha256: roots_digest(&roots)?,
        purpose,
        manifest_relative: outcome.manifest_path.strip_prefix(backup)?.into(),
        manifest_sha256: file_fingerprint(&outcome.manifest_path)?.1,
        phase: Phase::Prepared,
        members,
        bindings,
        authorizations: vec![],
        recovery: None,
        completed: None,
        full_completion: full,
    };
    validate(&journal, backup, &roots)?;
    let guard = StartupTransaction {
        _lock: Some(lock),
        roots,
        backup: backup.into(),
        outcome,
        journal,
    };
    guard.persist()?;
    fault("v3_prepared")?;
    Ok(guard)
}
fn write_immutable(path: &Path, bytes: &[u8]) -> Result<()> {
    anyhow::ensure!(
        bytes.len() as u64 <= JOURNAL_LIMIT,
        "completion evidence exceeds size bound"
    );
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    match options.open(path) {
        Ok(mut file) => {
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            anyhow::ensure!(
                read_bytes(path, JOURNAL_LIMIT)? == bytes,
                "immutable completion differs"
            );
            open_read(path)?.sync_all()?;
        }
        Err(e) => return Err(e.into()),
    }
    sync_dir(
        path.parent()
            .context("completion evidence parent missing")?,
    )
}
fn validate(
    j: &JournalV3,
    backup: &Path,
    roots: &[MigrationSourceRoot],
) -> Result<MigrationBackupOutcome> {
    anyhow::ensure!(
        j.schema_version == 3
            && j.migration_id == GENERATION
            && uuid::Uuid::parse_str(&j.attempt_id).is_ok()
            && j.roots_sha256 == roots_digest(roots)?
            && hash_valid(&j.manifest_sha256),
        "startup v3 identity mismatch"
    );
    let expected = specs(roots, j.purpose)?;
    anyhow::ensure!(
        j.members.len() == expected.len()
            && j.members
                .iter()
                .zip(&expected)
                .all(|(a, b)| a.root_label == b.root_label
                    && a.domain == b.domain
                    && a.name == b.name),
        "startup purpose member scope mismatch"
    );
    let states = prefixes(j)?;
    anyhow::ensure!(
        j.phase != Phase::Prepared || j.authorizations.is_empty(),
        "unarmed startup writes"
    );
    anyhow::ensure!(
        (j.phase == Phase::Completed) == j.completed.is_some(),
        "startup completion state mismatch"
    );
    if let Some(completed) = &j.completed {
        anyhow::ensure!(
            states.last() == Some(completed),
            "completed frontier mismatch"
        );
    }
    match (&j.phase, &j.recovery) {
        (Phase::Restoring | Phase::Recovered, Some(recovery)) => anyhow::ensure!(
            recovery.prefix < states.len()
                && recovery.cursor <= j.members.len()
                && (j.phase != Phase::Recovered || recovery.cursor == j.members.len()),
            "invalid restore cursor"
        ),
        (Phase::Restoring | Phase::Recovered, None) => anyhow::bail!("restore ownership missing"),
        (_, Some(_)) => anyhow::bail!("unexpected recovery plan"),
        (_, None) => {}
    }
    validate_relative(&j.manifest_relative)?;
    anyhow::ensure!(
        j.manifest_relative.components().count() == 2
            && j.manifest_relative.file_name() == Some(std::ffi::OsStr::new("manifest.json")),
        "invalid v3 manifest reference"
    );
    let path = checked_path(backup, &j.manifest_relative)?;
    let raw = read_bytes(&path, 16 * 1024 * 1024)?;
    anyhow::ensure!(sha(&raw) == j.manifest_sha256, "v3 manifest differs");
    let manifest: MigrationBackupManifest = serde_json::from_slice(&raw)?;
    anyhow::ensure!(
        manifest.migration_id == GENERATION
            && manifest.status == MigrationBackupStatus::Verified
            && manifest.sqlite_snapshot_format == Some(SqliteSnapshotFormat::VacuumIntoV1),
        "v3 manifest is not verified"
    );
    let outcome = MigrationBackupOutcome {
        backup_dir: path.parent().context("manifest parent missing")?.into(),
        manifest_path: path,
        manifest,
    };
    validate_outcome_paths(&outcome, Some(roots))?;
    let allowed = candidates(roots, j.purpose)?;
    let mut seen = std::collections::BTreeSet::new();
    for entry in &outcome.manifest.entries {
        anyhow::ensure!(
            seen.insert((&entry.source_root_label, &entry.source_relative_path))
                && allowed
                    .iter()
                    .any(|c| c.source_root_label == entry.source_root_label
                        && (c.relative_path == entry.source_relative_path && c.kind == entry.kind
                            || c.kind == MigrationPathKind::Directory
                                && entry.source_relative_path.starts_with(&c.relative_path))),
            "manifest exceeds startup purpose"
        );
    }
    for member in &j.members {
        let matches = outcome
            .manifest
            .entries
            .iter()
            .filter(|e| {
                e.source_root_label == member.root_label
                    && e.source_relative_path == Path::new(&member.name)
            })
            .collect::<Vec<_>>();
        match member.digest() {
            None => anyhow::ensure!(matches.is_empty(), "absent member has payload"),
            Some(d) => anyhow::ensure!(
                matches.len() == 1
                    && matches[0].kind == MigrationPathKind::File
                    && matches[0].sha256 == d.sha256
                    && matches[0].byte_len == d.size,
                "member snapshot mismatch"
            ),
        }
    }
    if let Some(evidence) = &j.full_completion {
        validate_relative(&evidence.relative)?;
        anyhow::ensure!(
            evidence.relative.components().count() == 1
                && evidence
                    .relative
                    .to_string_lossy()
                    .starts_with("full-startup-")
                && hash_valid(&evidence.sha256),
            "invalid full completion reference"
        );
        let raw = read_bytes(&checked_path(backup, &evidence.relative)?, JOURNAL_LIMIT)?;
        anyhow::ensure!(
            sha(&raw) == evidence.sha256,
            "full completion evidence changed"
        );
        let old: JournalV3 = serde_json::from_slice(&raw)?;
        anyhow::ensure!(
            old.purpose == StartupPurpose::FullStartup
                && old.phase == Phase::Completed
                && old.full_completion.is_none(),
            "full completion evidence is not a terminal full migration"
        );
        validate(&old, backup, roots)?;
    }
    Ok(outcome)
}
#[cfg(test)]
mod tests;
