use crate::{filesystem as fs, *};
use inputia_settings::installation::{valid_uuid, InstallationReceipt, LocatorContext, PRODUCT_ID};
use std::{
    collections::BTreeSet,
    fs::File,
    path::{Path, PathBuf},
};

const JOURNAL_LIMIT: usize = 256 * 1024;
#[derive(Clone, Debug)]
pub struct Updater {
    home: PathBuf,
    uid: u32,
}
pub struct Transaction {
    updater: Updater,
    _lock: File,
    journal: Journal,
}
fn bytes<T: serde::Serialize>(value: &T) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(value)?)
}
fn digest<T: serde::Serialize>(value: &T) -> Result<String> {
    Ok(fs::sha(&bytes(value)?))
}
fn receipt_context(home: &Path, uid: u32, release: &str) -> LocatorContext {
    LocatorContext {
        product_id: PRODUCT_ID.into(),
        release_id: release.into(),
        uid,
        home: home.into(),
    }
}
fn artifact_roles() -> BTreeSet<Role> {
    [Role::Control, Role::Ime, Role::Settings, Role::PairManifest]
        .into_iter()
        .collect()
}
fn component_set() -> BTreeSet<Role> {
    Role::COMPONENTS.into_iter().collect()
}
fn destination(
    receipt: &InstallationReceipt,
    home: &Path,
    uid: u32,
    role: Role,
) -> Result<PathBuf> {
    let located = receipt
        .clone()
        .resolve(&receipt_context(home, uid, &receipt.release_id))
        .map_err(|_| Error::Invalid("receipt identity or paths"))?;
    Ok(match role {
        Role::Control => located.receipt.components.control,
        Role::Ime => located.receipt.components.ime,
        Role::Settings => located.receipt.components.settings,
        Role::PairManifest => located.pair_manifest,
        Role::Receipt => receipt_context(home, uid, &receipt.release_id).receipt_path(),
    })
}
fn slot(destination: &Path, transaction: &str, role: Role, kind: &str) -> Result<PathBuf> {
    Ok(destination
        .parent()
        .ok_or(Error::UnsafePath)?
        .join(format!(".inputia-{transaction}-{}.{kind}", role.label())))
}
/// 原生验签适配器必须把回执绑定到这个精确制品集合，不能只返回一个版本号。
pub fn artifact_set_digest(entries: &[Entry]) -> Result<String> {
    let mut values: Vec<_> = entries
        .iter()
        .filter(|entry| entry.role != Role::Receipt)
        .map(|entry| (entry.role, &entry.new))
        .collect();
    values.sort_by_key(|value| value.0);
    digest(&values)
}

impl Updater {
    /// 接收系统用户目录和实际有效 UID；不接受跨用户执行。
    pub fn new(home: PathBuf, uid: u32) -> Result<Self> {
        fs::canonical(&home)?;
        // 当前实现为每用户执行环境；提权/跨用户安装必须由单独授权的助手实现。
        if uid != unsafe { libc::geteuid() } {
            return Err(Error::PermissionRequired);
        }
        fs::validate_prefix(&home.join(".probe"), uid)?;
        Ok(Self { home, uid })
    }
    /// 独立于被替换 app 的恢复执行环境根目录。
    pub fn root(&self) -> PathBuf {
        self.home
            .join("Library/Application Support/Inputia/Updater")
    }
    /// 两端启动门禁使用的固定维护标记路径。
    pub fn maintenance_path(&self) -> PathBuf {
        self.root().join("maintenance.json")
    }
    fn transaction_dir(&self, id: &str) -> Result<PathBuf> {
        if !valid_uuid(id) {
            return Err(Error::Invalid("transaction UUID"));
        }
        Ok(self.root().join("transactions").join(id))
    }
    fn journal_path(&self, id: &str) -> Result<PathBuf> {
        Ok(self.transaction_dir(id)?.join("journal.json"))
    }
    fn receipt_path(&self) -> PathBuf {
        receipt_context(&self.home, self.uid, "").receipt_path()
    }

    /// 只读预检：不建目录/锁/marker，不触碰程序或原数据。签名与维护凭据仍在 run 时由原生适配器验证。
    pub fn prepare(&self, request: InstallRequest) -> Result<PreparedPlan> {
        self.transaction_dir(&request.transaction_id)?;
        let new = &request.new_receipt;
        if new.scope == inputia_settings::installation::InstallationScope::LegacySingleUser {
            return Err(Error::PermissionRequired);
        }
        let _ = destination(new, &self.home, self.uid, Role::Receipt)?;
        let old_receipt = match fs::read(&self.receipt_path(), self.uid, 16_384) {
            Ok(raw) => Some(
                InstallationReceipt::parse(&raw).map_err(|_| Error::Invalid("existing receipt"))?,
            ),
            Err(Error::MissingArtifact) => None,
            Err(error) => return Err(error),
        };
        if let Some(old) = &old_receipt {
            let _ = destination(old, &self.home, self.uid, Role::Receipt)?;
            let mut expected = old.clone();
            expected.release_id = new.release_id.clone();
            expected.channel = new.channel.clone();
            if &expected != new || old.release_id == new.release_id {
                return Err(Error::Invalid(
                    "update changed installation or reused release",
                ));
            }
        }
        let roles: BTreeSet<_> = request
            .artifacts
            .iter()
            .map(|artifact| artifact.role)
            .collect();
        if roles != artifact_roles() || request.artifacts.len() != 4 {
            return Err(Error::Invalid("exact artifact roles required"));
        }
        let old_pair_manifest = old_receipt
            .as_ref()
            .map(|old| {
                let path = destination(old, &self.home, self.uid, Role::PairManifest)?;
                Ok::<FileEvidence, Error>(FileEvidence {
                    fingerprint: fingerprint(&path, self.uid)?,
                    path,
                })
            })
            .transpose()?;
        let mut entries = Vec::new();
        for role in [
            Role::Control,
            Role::Ime,
            Role::Settings,
            Role::PairManifest,
            Role::Receipt,
        ] {
            let target = destination(new, &self.home, self.uid, role)?;
            fs::validate_prefix(&target, self.uid)?;
            let old = fs::maybe_fingerprint(&target, self.uid)?;
            if role == Role::PairManifest && old.is_some() {
                return Err(Error::Invalid("immutable release path already exists"));
            }
            if old_receipt.is_none() && old.is_some() {
                return Err(Error::Invalid(
                    "unreceipted installation would be overwritten",
                ));
            }
            if old_receipt.is_some() && role != Role::PairManifest && old.is_none() {
                return Err(Error::MissingArtifact);
            }
            let (source, new_fingerprint) = if role == Role::Receipt {
                (None, fs::contents_fingerprint(&bytes(new)?, 0o600))
            } else {
                let artifact = request
                    .artifacts
                    .iter()
                    .find(|artifact| artifact.role == role)
                    .ok_or(Error::Invalid("missing role"))?;
                fs::canonical(&artifact.source)?;
                if artifact.source.starts_with(self.root())
                    || artifact.source.starts_with(
                        self.home
                            .join("Library/Application Support/Inputia/Profiles"),
                    )
                {
                    return Err(Error::UnsafePath);
                }
                for component in Role::COMPONENTS {
                    let installed = destination(new, &self.home, self.uid, component)?;
                    if artifact.source.starts_with(&installed)
                        || installed.starts_with(&artifact.source)
                    {
                        return Err(Error::UnsafePath);
                    }
                }
                if fingerprint(&artifact.source, self.uid)? != artifact.expected {
                    return Err(Error::ArtifactMismatch);
                }
                let new_fingerprint = if role == Role::PairManifest {
                    fs::contents_fingerprint(
                        &fs::read_public(&artifact.source, self.uid, 16_384)?,
                        0o600,
                    )
                } else {
                    artifact.expected.clone()
                };
                (Some(artifact.source.clone()), new_fingerprint)
            };
            let entry = Entry {
                role,
                source,
                stage: slot(&target, &request.transaction_id, role, "stage")?,
                backup: slot(&target, &request.transaction_id, role, "backup")?,
                failed: slot(&target, &request.transaction_id, role, "failed")?,
                destination: target,
                old,
                new: new_fingerprint,
            };
            for path in [&entry.stage, &entry.backup, &entry.failed] {
                if fs::maybe_fingerprint(path, self.uid)?.is_some() {
                    return Err(Error::TransactionReused);
                }
            }
            entries.push(entry);
        }
        let required_free_bytes =
            entries
                .iter()
                .try_fold(16u64 * 1024 * 1024, |total, entry| {
                    total
                        .checked_add(entry.new.bytes)
                        .ok_or(Error::Invalid("space overflow"))
                })?;
        for entry in &entries {
            if fs::free_bytes(&entry.destination, self.uid)? < required_free_bytes {
                return Err(Error::Invalid("insufficient staging space"));
            }
        }
        Ok(PreparedPlan {
            schema_version: 1,
            home: self.home.clone(),
            uid: self.uid,
            request,
            old_receipt,
            old_pair_manifest,
            entries,
            required_free_bytes,
        })
    }
    /// 只读解析日志并重新校验结构、收据路径及阶段绑定。
    pub fn inspect(&self, id: &str) -> Result<Journal> {
        let journal: Journal =
            serde_json::from_slice(&fs::read(&self.journal_path(id)?, self.uid, JOURNAL_LIMIT)?)?;
        self.validate_journal(&journal, id)?;
        Ok(journal)
    }
    fn validate_journal(&self, journal: &Journal, id: &str) -> Result<()> {
        let plan = &journal.plan;
        if journal.schema_version != 1
            || plan.schema_version != 1
            || plan.home != self.home
            || plan.uid != self.uid
            || journal.subject.transaction_id != id
            || plan.request.transaction_id != id
            || !valid_uuid(&journal.maintenance_epoch)
            || journal.subject.plan_sha256 != digest(plan)?
            || journal.subject.installation_id != plan.request.new_receipt.installation_id
            || journal.subject.new_release_id != plan.request.new_receipt.release_id
        {
            return Err(Error::Invalid("journal binding"));
        }
        if ((journal.phase == Phase::WritesReleased
            || journal.intent == Some(Action::ReleaseWrites))
            && !journal.writes_released)
            || (journal.phase == Phase::RolledBack && !journal.rollback_requested)
            || ((journal.writes_released || journal.phase == Phase::Committed)
                && (journal.postcheck.is_none()
                    || journal.snapshot.is_none()
                    || journal.quiescence.is_none()
                    || journal.environment.is_none()))
            || (journal.writes_released && journal.restoration.is_none())
            || ((journal.phase == Phase::RolledBack
                || journal.intent == Some(Action::ReleaseRollback))
                && journal.writes_released
                && journal.rollback_compatibility.is_none())
        {
            return Err(Error::Invalid("journal terminal prerequisites"));
        }
        let subjects = [
            journal.recovery.as_ref().map(|v| &v.subject),
            journal.environment.as_ref().map(|v| &v.subject),
            journal.quiescence.as_ref().map(|v| &v.subject),
            journal.snapshot.as_ref().map(|v| &v.subject),
            journal.restoration.as_ref().map(|v| &v.subject),
            journal.rollback_compatibility.as_ref().map(|v| &v.subject),
        ];
        if subjects
            .into_iter()
            .flatten()
            .any(|subject| subject != &journal.subject)
        {
            return Err(Error::Invalid("journal stage receipt subject"));
        }
        if let Some(check) = &journal.postcheck {
            if check.subject != journal.subject
                || check.release_id != journal.subject.new_release_id
                || check.profile_id != plan.request.new_receipt.profile_id
                || check.receipt_fingerprint
                    != fs::contents_fingerprint(&bytes(&plan.request.new_receipt)?, 0o600)
                || !fs::valid_sha(&check.handshake_and_schema_evidence_sha256)
            {
                return Err(Error::Invalid("journal postcheck subject"));
            }
        }
        if let Some(proof) = &journal.rollback_compatibility {
            if proof.epoch != journal.maintenance_epoch
                || journal.plan.old_receipt.as_ref().map(|v| &v.release_id)
                    != Some(&proof.target_release_id)
                || proof.tested_domains != DataDomain::all()
                || !fs::valid_sha(&proof.read_write_outbox_privacy_evidence_sha256)
            {
                return Err(Error::Invalid("journal rollback compatibility binding"));
            }
        }
        let expected_roles: BTreeSet<_> = [
            Role::Control,
            Role::Ime,
            Role::Settings,
            Role::PairManifest,
            Role::Receipt,
        ]
        .into_iter()
        .collect();
        if plan.entries.len() != 5
            || plan
                .entries
                .iter()
                .map(|entry| entry.role)
                .collect::<BTreeSet<_>>()
                != expected_roles
        {
            return Err(Error::Invalid("journal roles"));
        }
        if plan.request.artifacts.len() != 4
            || plan
                .request
                .artifacts
                .iter()
                .map(|artifact| artifact.role)
                .collect::<BTreeSet<_>>()
                != artifact_roles()
        {
            return Err(Error::Invalid("journal source roles"));
        }
        for entry in &plan.entries {
            if (entry.role == Role::PairManifest && entry.old.is_some())
                || (plan.old_receipt.is_some()
                    && entry.role != Role::PairManifest
                    && entry.old.is_none())
            {
                return Err(Error::Invalid("journal old artifact set"));
            }
            if entry.role == Role::Receipt {
                if entry.source.is_some()
                    || entry.new
                        != fs::contents_fingerprint(&bytes(&plan.request.new_receipt)?, 0o600)
                {
                    return Err(Error::Invalid("journal receipt bytes"));
                }
            } else {
                let artifact = plan
                    .request
                    .artifacts
                    .iter()
                    .find(|artifact| artifact.role == entry.role)
                    .ok_or(Error::Invalid("journal source"))?;
                if entry.source.as_ref() != Some(&artifact.source)
                    || (entry.role != Role::PairManifest && entry.new != artifact.expected)
                {
                    return Err(Error::Invalid("journal artifact mapping"));
                }
                fs::canonical(&artifact.source)?;
            }
            let target = destination(&plan.request.new_receipt, &self.home, self.uid, entry.role)?;
            if target != entry.destination
                || entry.stage != slot(&target, id, entry.role, "stage")?
                || entry.backup != slot(&target, id, entry.role, "backup")?
                || entry.failed != slot(&target, id, entry.role, "failed")?
                || !fs::valid_sha(&entry.new.sha256)
            {
                return Err(Error::Invalid("journal paths"));
            }
        }
        if let Some(old) = &plan.old_receipt {
            let mut expected = old.clone();
            expected.release_id = plan.request.new_receipt.release_id.clone();
            expected.channel = plan.request.new_receipt.channel.clone();
            if expected != plan.request.new_receipt {
                return Err(Error::Invalid("old receipt identity"));
            }
            let pair = plan
                .old_pair_manifest
                .as_ref()
                .ok_or(Error::Invalid("old pair missing"))?;
            if pair.path != destination(old, &self.home, self.uid, Role::PairManifest)? {
                return Err(Error::Invalid("old pair path"));
            }
        } else if plan.old_pair_manifest.is_some()
            || plan.entries.iter().any(|entry| entry.old.is_some())
        {
            return Err(Error::Invalid("new install has old state"));
        }
        Ok(())
    }
    fn acquire(&self) -> Result<File> {
        fs::lock(&self.root().join("update.lock"), self.uid)
    }
    fn no_other_pending(&self, current: Option<&str>) -> Result<()> {
        let root = self.root().join("transactions");
        let entries = match fs::list(&root, self.uid) {
            Ok(entries) => entries,
            Err(Error::MissingArtifact) => return Ok(()),
            Err(e) => return Err(e),
        };
        for id in entries {
            let id = id.to_str().ok_or(Error::UnsafePath)?;
            if Some(id) == current {
                continue;
            }
            let journal = self
                .inspect(id)
                .map_err(|_| Error::NeedsRepair("unrecognized transaction retained"))?;
            let complete = (journal.phase == Phase::WritesReleased
                && journal.writes_released
                && !journal.rollback_requested)
                || (journal.phase == Phase::RolledBack && journal.rollback_requested);
            if !complete || journal.intent.is_some() {
                return Err(Error::Busy);
            }
        }
        Ok(())
    }
    /// 取得进程锁、重新预检并独占建立新事务；不移动安装组件。
    pub fn begin(&self, plan: PreparedPlan) -> Result<Transaction> {
        let lock = self.acquire()?;
        self.no_other_pending(None)?;
        if fs::maybe_fingerprint(&self.maintenance_path(), self.uid)?.is_some() {
            return Err(Error::Busy);
        }
        // prepare 后盘面若改变必须重新明确预检；不能用过期摘要覆盖新文件。
        if self.prepare(plan.request.clone())? != plan {
            return Err(Error::ArtifactMismatch);
        }
        let subject = Subject {
            transaction_id: plan.request.transaction_id.clone(),
            plan_sha256: digest(&plan)?,
            installation_id: plan.request.new_receipt.installation_id.clone(),
            new_release_id: plan.request.new_receipt.release_id.clone(),
        };
        fs::mkdir_new(&self.transaction_dir(&subject.transaction_id)?, self.uid)?;
        let journal = Journal {
            schema_version: 1,
            subject,
            plan,
            phase: Phase::Prepared,
            maintenance_epoch: uuid::Uuid::new_v4().to_string(),
            environment: None,
            intent: None,
            recovery: None,
            quiescence: None,
            snapshot: None,
            postcheck: None,
            restoration: None,
            writes_released: false,
            rollback_requested: false,
            rollback_compatibility: None,
        };
        fs::write_new(
            &self.journal_path(&journal.subject.transaction_id)?,
            self.uid,
            &bytes(&journal)?,
        )?;
        Ok(Transaction {
            updater: self.clone(),
            _lock: lock,
            journal,
        })
    }
    /// 在互斥锁内从日志与当前盘面恢复，绝不自动恢复旧用户数据。
    pub fn recover(
        &self,
        id: &str,
        policy: RecoveryPolicy,
        adapter: &mut impl NativeAdapter,
        faults: &mut impl FaultInjector,
    ) -> Result<Journal> {
        let lock = self.acquire()?;
        self.no_other_pending(Some(id))?;
        let journal = self.inspect(id)?;
        let mut transaction = Transaction {
            updater: self.clone(),
            _lock: lock,
            journal,
        };
        transaction.run(policy, adapter, faults)?;
        Ok(transaction.journal)
    }
}

impl Transaction {
    /// 为guardian复制本事务同一flock描述符；仅close释放引用，禁止任何侧LOCK_UN。
    /// 这不是完整停写回执，也不授权跳过真实marker、TIS或数据域交接门禁。
    pub fn guardian_authority(&self) -> Result<crate::guardian::GuardianTransactionAuthority> {
        self.ensure_marker()?;
        Ok(crate::guardian::GuardianTransactionAuthority {
            lock: self._lock.try_clone()?,
            subject: self.journal.subject.clone(),
            marker: self.marker(),
        })
    }
    /// 当前事务只读状态；成功状态不代替真实设备或业务验收。
    pub fn journal(&self) -> &Journal {
        &self.journal
    }
    fn save(&self, faults: &mut impl FaultInjector) -> Result<()> {
        faults.checkpoint(FaultPoint::BeforeJournal(self.journal.phase.clone()))?;
        let raw = bytes(&self.journal)?;
        if raw.len() > JOURNAL_LIMIT {
            return Err(Error::Invalid("journal evidence size budget"));
        }
        fs::atomic_write(
            &self
                .updater
                .journal_path(&self.journal.subject.transaction_id)?,
            self.updater.uid,
            &raw,
        )?;
        faults.checkpoint(FaultPoint::AfterJournal(self.journal.phase.clone()))
    }
    fn marker(&self) -> MaintenanceMarker {
        MaintenanceMarker {
            schema_version: 1,
            transaction_id: self.journal.subject.transaction_id.clone(),
            installation_id: self.journal.subject.installation_id.clone(),
            old_release_id: self
                .journal
                .plan
                .old_receipt
                .as_ref()
                .map(|r| r.release_id.clone()),
            new_release_id: self.journal.subject.new_release_id.clone(),
            epoch: self.journal.maintenance_epoch.clone(),
            plan_sha256: self.journal.subject.plan_sha256.clone(),
        }
    }
    fn ensure_marker(&self) -> Result<()> {
        let expected = bytes(&self.marker())?;
        match fs::read(&self.updater.maintenance_path(), self.updater.uid, 4096) {
            Ok(actual) if actual == expected => Ok(()),
            Ok(_) => Err(Error::NeedsRepair("foreign maintenance marker")),
            Err(Error::MissingArtifact) => fs::write_new(
                &self.updater.maintenance_path(),
                self.updater.uid,
                &expected,
            ),
            Err(e) => Err(e),
        }
    }
    fn check_marker(&self) -> Result<()> {
        if fs::read(&self.updater.maintenance_path(), self.updater.uid, 4096)?
            != bytes(&self.marker())?
        {
            return Err(Error::NeedsRepair("maintenance marker changed"));
        }
        Ok(())
    }
    fn subject(&self, value: &Subject) -> Result<()> {
        if value != &self.journal.subject {
            Err(Error::Adapter("receipt subject mismatch"))
        } else {
            Ok(())
        }
    }
    fn recovery_environment(
        &mut self,
        adapter: &mut impl NativeAdapter,
        faults: &mut impl FaultInjector,
    ) -> Result<()> {
        let receipt =
            adapter.prepare_recovery_environment(&self.journal.subject, &self.updater.root())?;
        self.subject(&receipt.subject)?;
        if receipt.journal_contract != 1
            || !fs::valid_sha(&receipt.recovery_registration_sha256)
            || receipt.bootstrap.path == receipt.helper.path
        {
            return Err(Error::Adapter("external recovery proof"));
        }
        for file in [&receipt.bootstrap, &receipt.helper] {
            if !file.path.starts_with(self.updater.root().join("versions"))
                && file.path != self.updater.root().join("bootstrap")
            {
                return Err(Error::Adapter("recovery must be outside replacement paths"));
            }
            if fingerprint(&file.path, self.updater.uid)? != file.fingerprint {
                return Err(Error::ArtifactMismatch);
            }
            fs::sync_file(&file.path, self.updater.uid)?;
        }
        self.journal.recovery = Some(receipt);
        self.save(faults)
    }
    fn verify(
        &self,
        adapter: &mut impl NativeAdapter,
        entries: &[Entry],
        purpose: VerificationPurpose,
    ) -> Result<()> {
        let receipt = adapter.verify_artifacts(&self.journal.subject, entries, purpose)?;
        self.subject(&receipt.subject)?;
        let checks = [
            VerificationCheck::SecurityAllArchitectures,
            VerificationCheck::HardenedRuntime,
            VerificationCheck::ReleaseSignature,
            VerificationCheck::PairSignature,
            VerificationCheck::BundleMetadata,
        ]
        .into_iter()
        .collect();
        if receipt.roles != artifact_roles()
            || receipt.checks != checks
            || receipt.artifact_set_sha256 != artifact_set_digest(entries)?
            || !fs::valid_sha(&receipt.evidence_sha256)
        {
            return Err(Error::Adapter("incomplete artifact verification"));
        }
        Ok(())
    }
    fn stage(&mut self, faults: &mut impl FaultInjector) -> Result<()> {
        for entry in self.journal.plan.entries.clone() {
            let target = fs::maybe_fingerprint(&entry.destination, self.updater.uid)?;
            let stage = fs::maybe_fingerprint(&entry.stage, self.updater.uid)?;
            let installed_new = target.as_ref() == Some(&entry.new)
                && (entry.old.as_ref() != Some(&entry.new)
                    || fs::maybe_fingerprint(&entry.backup, self.updater.uid)? == entry.old);
            if stage.as_ref() == Some(&entry.new) || installed_new {
                continue;
            }
            if stage.is_some() {
                return Err(Error::NeedsRepair("unrecognized staging artifact"));
            }
            let temporary = entry.stage.with_extension("copy");
            match fs::maybe_fingerprint(&temporary, self.updater.uid)? {
                Some(actual) if actual == entry.new => {}
                Some(_) => return Err(Error::NeedsRepair("partial staging copy retained")),
                None => {
                    if let Some(source) = &entry.source {
                        let expected = self
                            .journal
                            .plan
                            .request
                            .artifacts
                            .iter()
                            .find(|artifact| artifact.role == entry.role)
                            .ok_or(Error::Invalid("source role"))?;
                        if fingerprint(source, self.updater.uid)? != expected.expected {
                            return Err(Error::ArtifactMismatch);
                        }
                        fs::copy(source, &temporary, self.updater.uid)?;
                        if entry.role == Role::PairManifest {
                            fs::private_mode(&temporary, self.updater.uid)?;
                        }
                    } else {
                        fs::write_new(
                            &temporary,
                            self.updater.uid,
                            &bytes(&self.journal.plan.request.new_receipt)?,
                        )?;
                    }
                    if fingerprint(&temporary, self.updater.uid)? != entry.new {
                        return Err(Error::ArtifactMismatch);
                    }
                }
            }
            self.move_artifact(
                Action::Stage { role: entry.role },
                &temporary,
                &entry.stage,
                &entry.new,
                faults,
            )?;
        }
        self.journal.phase = Phase::Staged;
        self.save(faults)
    }
    fn move_artifact(
        &mut self,
        action: Action,
        source: &Path,
        target: &Path,
        expected: &Fingerprint,
        faults: &mut impl FaultInjector,
    ) -> Result<()> {
        if fingerprint(source, self.updater.uid)? != *expected {
            return Err(Error::ArtifactMismatch);
        }
        if fs::maybe_fingerprint(target, self.updater.uid)?.is_some() {
            return Err(Error::NeedsRepair("rename target already occupied"));
        }
        self.journal.intent = Some(action.clone());
        self.save(faults)?;
        faults.checkpoint(FaultPoint::BeforeRename(action.clone()))?;
        fs::rename(source, target, self.updater.uid, false)?;
        faults.checkpoint(FaultPoint::AfterRename(action))?;
        if fingerprint(target, self.updater.uid)? != *expected {
            return Err(Error::ArtifactMismatch);
        }
        self.journal.intent = None;
        self.save(faults)
    }
    fn located_new(&self) -> Result<Vec<Entry>> {
        self.journal
            .plan
            .entries
            .iter()
            .filter(|e| e.role != Role::Receipt)
            .map(|entry| {
                let mut result = entry.clone();
                result.stage = if fs::maybe_fingerprint(&entry.destination, self.updater.uid)?
                    .as_ref()
                    == Some(&entry.new)
                {
                    entry.destination.clone()
                } else {
                    entry.stage.clone()
                };
                if fingerprint(&result.stage, self.updater.uid)? != entry.new {
                    return Err(Error::ArtifactMismatch);
                }
                Ok(result)
            })
            .collect()
    }
    fn quiesce(
        &mut self,
        adapter: &mut impl NativeAdapter,
        faults: &mut impl FaultInjector,
    ) -> Result<()> {
        if self.journal.environment.is_none() {
            let snapshot = adapter.capture_environment(&self.journal.subject)?;
            self.subject(&snapshot.subject)?;
            if !snapshot
                .previously_running_roles
                .is_subset(&component_set())
                || snapshot.previous_input_source.is_empty()
                || snapshot.previous_input_source.len() > 256
                || snapshot.previous_input_source.chars().any(char::is_control)
                || !fs::valid_sha(&snapshot.process_snapshot_sha256)
            {
                return Err(Error::Adapter("invalid process/input-source snapshot"));
            }
            self.journal.environment = Some(snapshot);
            self.save(faults)?;
        }
        self.ensure_marker()?;
        let previous = self
            .journal
            .environment
            .as_ref()
            .ok_or(Error::Adapter("missing process snapshot"))?;
        let receipt = adapter.quiesce(&self.journal.subject, &self.marker(), previous)?;
        self.subject(&receipt.subject)?;
        if receipt.epoch != self.journal.maintenance_epoch
            || receipt.stopped_roles != component_set()
            || receipt.previously_running_roles != previous.previously_running_roles
            || receipt.previous_input_source != previous.previous_input_source
            || !fs::valid_sha(&receipt.process_snapshot_sha256)
        {
            return Err(Error::Adapter("quiescence not confirmed"));
        }
        self.check_marker()?;
        adapter.assert_quiesced(&self.journal.subject, &self.marker())?;
        self.journal.quiescence = Some(receipt);
        self.journal.phase = Phase::Quiesced;
        self.save(faults)
    }
    fn assert_quiesced(&self, adapter: &mut impl NativeAdapter) -> Result<()> {
        self.check_marker()?;
        adapter.assert_quiesced(&self.journal.subject, &self.marker())
    }
    fn snapshot(
        &mut self,
        adapter: &mut impl NativeAdapter,
        faults: &mut impl FaultInjector,
    ) -> Result<()> {
        let directory = self
            .updater
            .transaction_dir(&self.journal.subject.transaction_id)?
            .join("snapshot");
        fs::ensure_dir(&directory, self.updater.uid)?;
        let receipt = match &self.journal.snapshot {
            Some(receipt) => receipt.clone(),
            None => adapter.snapshot(&self.journal.subject, &self.marker(), &directory)?,
        };
        self.subject(&receipt.subject)?;
        if receipt.epoch != self.journal.maintenance_epoch
            || receipt.covers != DataDomain::all()
            || receipt.files.is_empty()
            || receipt.files.len() > 256
            || receipt
                .files
                .iter()
                .map(|file| &file.path)
                .collect::<BTreeSet<_>>()
                .len()
                != receipt.files.len()
            || !fs::valid_sha(&receipt.consistency_evidence_sha256)
        {
            return Err(Error::Adapter("consistent snapshot incomplete"));
        }
        for file in &receipt.files {
            if !file.path.starts_with(&directory)
                || fingerprint(&file.path, self.updater.uid)? != file.fingerprint
            {
                return Err(Error::ArtifactMismatch);
            }
            fs::sync_file(&file.path, self.updater.uid)?;
        }
        self.journal.snapshot = Some(receipt);
        self.journal.phase = Phase::SnapshotReady;
        self.save(faults)
    }
    fn replace_entry(
        &mut self,
        entry: &Entry,
        adapter: &mut impl NativeAdapter,
        faults: &mut impl FaultInjector,
    ) -> Result<()> {
        self.assert_quiesced(adapter)?;
        let target = fs::maybe_fingerprint(&entry.destination, self.updater.uid)?;
        let backup = fs::maybe_fingerprint(&entry.backup, self.updater.uid)?;
        let staged = fs::maybe_fingerprint(&entry.stage, self.updater.uid)?;
        if fs::maybe_fingerprint(&entry.failed, self.updater.uid)?.is_some() {
            return Err(Error::NeedsRepair(
                "rollback artifacts require rollback recovery",
            ));
        }
        if target.as_ref() == Some(&entry.new) && backup == entry.old && staged.is_none() {
            return Ok(());
        }
        if staged.as_ref() != Some(&entry.new) {
            return Err(Error::NeedsRepair("new artifact missing or changed"));
        }
        if let Some(old) = &entry.old {
            if target.as_ref() == Some(old) && backup.is_none() {
                self.move_artifact(
                    Action::Backup { role: entry.role },
                    &entry.destination,
                    &entry.backup,
                    old,
                    faults,
                )?;
            } else if target.is_some() || backup.as_ref() != Some(old) {
                return Err(Error::NeedsRepair("old artifact set does not match"));
            }
        } else if target.is_some() || backup.is_some() {
            return Err(Error::NeedsRepair("new installation path occupied"));
        }
        self.assert_quiesced(adapter)?;
        self.move_artifact(
            Action::Install { role: entry.role },
            &entry.stage,
            &entry.destination,
            &entry.new,
            faults,
        )
    }
    fn audit_final(&self, new: bool) -> Result<()> {
        for entry in &self.journal.plan.entries {
            let expected = if new {
                Some(entry.new.clone())
            } else {
                entry.old.clone()
            };
            if fs::maybe_fingerprint(&entry.destination, self.updater.uid)? != expected {
                return Err(Error::NeedsRepair("final installation changed"));
            }
            if !new && fs::maybe_fingerprint(&entry.backup, self.updater.uid)?.is_some() {
                return Err(Error::NeedsRepair("rollback backup still unresolved"));
            }
            if let Some(failed) = fs::maybe_fingerprint(&entry.failed, self.updater.uid)? {
                if failed != entry.new {
                    return Err(Error::NeedsRepair("failed artifact changed"));
                }
            }
            if new && fs::maybe_fingerprint(&entry.backup, self.updater.uid)? != entry.old {
                return Err(Error::NeedsRepair("verified rollback backup missing"));
            }
        }
        Ok(())
    }
    fn release(
        &mut self,
        rolled_back: bool,
        adapter: &mut impl NativeAdapter,
        faults: &mut impl FaultInjector,
    ) -> Result<()> {
        self.check_marker()?;
        let quiescence = self
            .journal
            .quiescence
            .as_ref()
            .ok_or(Error::Adapter("missing quiescence"))?;
        let restoration =
            adapter.restore_environment(&self.journal.subject, quiescence, rolled_back)?;
        self.subject(&restoration.subject)?;
        if !restoration
            .restored_running_roles
            .is_subset(&quiescence.previously_running_roles)
            || !fs::valid_sha(&restoration.evidence_sha256)
        {
            return Err(Error::Adapter("unexpected service restart"));
        }
        self.journal.restoration = Some(restoration);
        self.journal.intent = Some(if rolled_back {
            Action::ReleaseRollback
        } else {
            Action::ReleaseWrites
        });
        if !rolled_back {
            self.journal.writes_released = true;
        }
        // 先耐久记录写入可能开放，崩溃后绝不能把新数据当成未使用状态。
        self.save(faults)?;
        faults.checkpoint(FaultPoint::BeforeMarkerRemoval { rolled_back })?;
        fs::unlink(&self.updater.maintenance_path(), self.updater.uid)?;
        faults.checkpoint(FaultPoint::AfterMarkerRemoval { rolled_back })?;
        self.journal.intent = None;
        self.journal.phase = if rolled_back {
            Phase::RolledBack
        } else {
            Phase::WritesReleased
        };
        self.save(faults)
    }
    fn validate_postcheck(&self, check: &PostcheckReceipt) -> Result<()> {
        self.subject(&check.subject)?;
        let expected = self
            .journal
            .plan
            .entries
            .iter()
            .find(|entry| entry.role == Role::Receipt)
            .ok_or(Error::Invalid("receipt role"))?;
        if check.release_id != self.journal.subject.new_release_id
            || check.profile_id != self.journal.plan.request.new_receipt.profile_id
            || check.receipt_fingerprint != expected.new
            || check.epoch != self.journal.maintenance_epoch
            || !fs::valid_sha(&check.handshake_and_schema_evidence_sha256)
        {
            return Err(Error::Adapter("postcheck binding"));
        }
        Ok(())
    }
    /// 执行或续跑事务。错误保留日志、备份和维护门禁，供下一次恢复。
    pub fn run(
        &mut self,
        policy: RecoveryPolicy,
        adapter: &mut impl NativeAdapter,
        faults: &mut impl FaultInjector,
    ) -> Result<()> {
        self.updater
            .validate_journal(&self.journal, &self.journal.subject.transaction_id)?;
        self.recovery_environment(adapter, faults)?;
        // 回滚套已落盘、环境回执耐久，且 marker 已移除但最终日志尚未来得及写入。
        if self.journal.rollback_requested
            && self.journal.intent == Some(Action::ReleaseRollback)
            && matches!(
                fs::read(&self.updater.maintenance_path(), self.updater.uid, 4096),
                Err(Error::MissingArtifact)
            )
        {
            self.audit_final(false)?;
            if self.journal.quiescence.is_none() || self.journal.restoration.is_none() {
                return Err(Error::Invalid("rollback release prerequisites"));
            }
            if let Some(old) = &self.journal.plan.old_receipt {
                self.verify(
                    adapter,
                    &self.old_entries()?,
                    VerificationPurpose::RollbackOld {
                        release_id: old.release_id.clone(),
                    },
                )?;
            }
            self.journal.phase = Phase::RolledBack;
            self.journal.intent = None;
            return self.save(faults);
        }
        if !self.journal.writes_released && self.journal.phase != Phase::RolledBack {
            let absent = matches!(
                fs::read(&self.updater.maintenance_path(), self.updater.uid, 4096),
                Err(Error::MissingArtifact)
            );
            if absent {
                let mut moved = self.journal.quiescence.is_some();
                for entry in &self.journal.plan.entries {
                    moved |= fs::maybe_fingerprint(&entry.backup, self.updater.uid)?.is_some();
                    moved |= entry.old.as_ref() != Some(&entry.new)
                        && fs::maybe_fingerprint(&entry.destination, self.updater.uid)?.as_ref()
                            == Some(&entry.new);
                }
                if moved {
                    return Err(Error::NeedsRepair(
                        "maintenance marker missing after replacement or quiescence",
                    ));
                }
            }
        }
        if policy == RecoveryPolicy::BinaryRollback
            || self.journal.rollback_requested
            || self.journal.phase == Phase::RollingBack
            || self.journal.phase == Phase::RolledBack
        {
            return self.rollback(adapter, faults);
        }
        if self.journal.writes_released || self.journal.phase == Phase::WritesReleased {
            self.audit_final(true)?;
            self.verify(
                adapter,
                &self.journal.plan.entries,
                VerificationPurpose::InstalledNew,
            )?;
            match fs::read(&self.updater.maintenance_path(), self.updater.uid, 4096) {
                Ok(_) => {
                    self.assert_quiesced(adapter)?;
                    let check = adapter.postcheck(
                        &self.journal.subject,
                        &self.journal.plan,
                        &self.marker(),
                    )?;
                    self.validate_postcheck(&check)?;
                    self.journal.postcheck = Some(check);
                    self.save(faults)?;
                    fs::unlink(&self.updater.maintenance_path(), self.updater.uid)?;
                }
                Err(Error::MissingArtifact) => {}
                Err(e) => return Err(e),
            }
            self.journal.phase = Phase::WritesReleased;
            self.journal.intent = None;
            return self.save(faults);
        }
        if let Some(old) = &self.journal.plan.old_receipt {
            self.verify(
                adapter,
                &self.old_entries()?,
                VerificationPurpose::RollbackOld {
                    release_id: old.release_id.clone(),
                },
            )?;
        }
        if self.located_new().is_err() {
            let mut sources = Vec::new();
            for entry in self
                .journal
                .plan
                .entries
                .iter()
                .filter(|entry| entry.role != Role::Receipt)
            {
                let artifact = self
                    .journal
                    .plan
                    .request
                    .artifacts
                    .iter()
                    .find(|artifact| artifact.role == entry.role)
                    .ok_or(Error::Invalid("source role"))?;
                if fingerprint(&artifact.source, self.updater.uid)? != artifact.expected {
                    return Err(Error::ArtifactMismatch);
                }
                let mut source = entry.clone();
                source.stage = artifact.source.clone();
                source.new = artifact.expected.clone();
                sources.push(source);
            }
            self.verify(adapter, &sources, VerificationPurpose::DownloadedNew)?;
        }
        self.stage(faults)?;
        self.verify(
            adapter,
            &self.located_new()?,
            VerificationPurpose::StagedNew,
        )?;
        self.quiesce(adapter, faults)?;
        self.snapshot(adapter, faults)?;
        self.journal.phase = Phase::Replacing;
        self.save(faults)?;
        for entry in self
            .journal
            .plan
            .entries
            .clone()
            .into_iter()
            .filter(|entry| entry.role != Role::Receipt)
        {
            self.replace_entry(&entry, adapter, faults)?;
        }
        self.verify(
            adapter,
            &self.journal.plan.entries,
            VerificationPurpose::InstalledNew,
        )?;
        self.journal.phase = Phase::PairVerified;
        self.save(faults)?;
        let receipt_entry = self
            .journal
            .plan
            .entries
            .iter()
            .find(|entry| entry.role == Role::Receipt)
            .cloned()
            .ok_or(Error::Invalid("receipt role"))?;
        self.replace_entry(&receipt_entry, adapter, faults)?;
        self.journal.phase = Phase::Activated;
        self.save(faults)?;
        self.assert_quiesced(adapter)?;
        let postcheck =
            adapter.postcheck(&self.journal.subject, &self.journal.plan, &self.marker())?;
        self.subject(&postcheck.subject)?;
        self.validate_postcheck(&postcheck)?;
        self.check_marker()?;
        self.audit_final(true)?;
        self.journal.postcheck = Some(postcheck);
        self.journal.phase = Phase::Committed;
        self.save(faults)?;
        self.release(false, adapter, faults)
    }
    fn old_entries(&self) -> Result<Vec<Entry>> {
        let mut result = Vec::new();
        for entry in self
            .journal
            .plan
            .entries
            .iter()
            .filter(|entry| Role::COMPONENTS.contains(&entry.role))
        {
            if let Some(old) = &entry.old {
                let mut located = entry.clone();
                located.new = old.clone();
                located.stage = if fs::maybe_fingerprint(&entry.backup, self.updater.uid)?.as_ref()
                    == Some(old)
                {
                    entry.backup.clone()
                } else {
                    entry.destination.clone()
                };
                located.destination = located.stage.clone();
                if fingerprint(&located.stage, self.updater.uid)? != *old {
                    return Err(Error::NeedsRepair("old trusted set unavailable"));
                }
                result.push(located);
            }
        }
        if let Some(pair) = &self.journal.plan.old_pair_manifest {
            if fingerprint(&pair.path, self.updater.uid)? != pair.fingerprint {
                return Err(Error::NeedsRepair("old pair manifest changed"));
            }
            let mut entry = self
                .journal
                .plan
                .entries
                .iter()
                .find(|entry| entry.role == Role::PairManifest)
                .cloned()
                .ok_or(Error::Invalid("pair role"))?;
            entry.new = pair.fingerprint.clone();
            entry.stage = pair.path.clone();
            entry.destination = pair.path.clone();
            result.push(entry);
        }
        Ok(result)
    }
    fn rollback(
        &mut self,
        adapter: &mut impl NativeAdapter,
        faults: &mut impl FaultInjector,
    ) -> Result<()> {
        if self.journal.phase == Phase::RolledBack {
            self.audit_final(false)?;
            if self.journal.plan.old_receipt.is_some() {
                self.verify(
                    adapter,
                    &self.old_entries()?,
                    VerificationPurpose::RollbackOld {
                        release_id: self
                            .journal
                            .plan
                            .old_receipt
                            .as_ref()
                            .ok_or(Error::Invalid("old receipt"))?
                            .release_id
                            .clone(),
                    },
                )?;
            }
            return Ok(());
        }
        if !self.journal.rollback_requested {
            self.journal.rollback_requested = true;
            // 新写入开放后的回滚使用新维护 epoch，不能复用先前停止回执。
            if self.journal.writes_released {
                match fs::read(&self.updater.maintenance_path(), self.updater.uid, 4096) {
                    Ok(_) => self.check_marker()?,
                    Err(Error::MissingArtifact) => {
                        self.journal.maintenance_epoch = uuid::Uuid::new_v4().to_string()
                    }
                    Err(e) => return Err(e),
                }
            }
            self.save(faults)?;
        }
        if self.journal.plan.old_receipt.is_some() {
            self.verify(
                adapter,
                &self.old_entries()?,
                VerificationPurpose::RollbackOld {
                    release_id: self
                        .journal
                        .plan
                        .old_receipt
                        .as_ref()
                        .ok_or(Error::Invalid("old receipt"))?
                        .release_id
                        .clone(),
                },
            )?;
        }
        self.quiesce(adapter, faults)?;
        if self.journal.writes_released {
            let old = self
                .journal
                .plan
                .old_receipt
                .as_ref()
                .ok_or(Error::NeedsRepair(
                    "new installation already accepted writes",
                ))?;
            let proof =
                adapter.rollback_compatibility(&self.journal.subject, old, &self.marker())?;
            self.subject(&proof.subject)?;
            self.assert_quiesced(adapter)?;
            if proof.epoch != self.journal.maintenance_epoch
                || proof.target_release_id != old.release_id
                || proof.tested_domains != DataDomain::all()
                || !fs::valid_sha(&proof.read_write_outbox_privacy_evidence_sha256)
            {
                return Err(Error::Adapter("rollback read/write/privacy compatibility"));
            }
            self.journal.rollback_compatibility = Some(proof);
            self.save(faults)?;
        }
        self.journal.phase = Phase::RollingBack;
        self.save(faults)?;
        for entry in self.journal.plan.entries.clone() {
            self.assert_quiesced(adapter)?;
            let target = fs::maybe_fingerprint(&entry.destination, self.updater.uid)?;
            let failed = fs::maybe_fingerprint(&entry.failed, self.updater.uid)?;
            let backup = fs::maybe_fingerprint(&entry.backup, self.updater.uid)?;
            if failed.is_some() && failed.as_ref() != Some(&entry.new) {
                return Err(Error::NeedsRepair("unknown failed artifact retained"));
            }
            if target == entry.old && backup.is_none() {
                continue;
            }
            if target.as_ref() == Some(&entry.new) {
                if failed.is_some() {
                    return Err(Error::NeedsRepair("duplicate failed target"));
                }
                self.move_artifact(
                    Action::PreserveFailed { role: entry.role },
                    &entry.destination,
                    &entry.failed,
                    &entry.new,
                    faults,
                )?;
            } else if target.is_some() {
                return Err(Error::NeedsRepair("foreign target blocks rollback"));
            }
            if let Some(old) = &entry.old {
                if backup.as_ref() != Some(old) {
                    return Err(Error::NeedsRepair("rollback backup missing"));
                }
                self.move_artifact(
                    Action::RestoreOld { role: entry.role },
                    &entry.backup,
                    &entry.destination,
                    old,
                    faults,
                )?;
            } else if backup.is_some() {
                return Err(Error::NeedsRepair("unexpected backup"));
            }
        }
        self.audit_final(false)?;
        if self.journal.plan.old_receipt.is_some() {
            self.verify(
                adapter,
                &self.old_entries()?,
                VerificationPurpose::RollbackOld {
                    release_id: self
                        .journal
                        .plan
                        .old_receipt
                        .as_ref()
                        .ok_or(Error::Invalid("old receipt"))?
                        .release_id
                        .clone(),
                },
            )?;
        }
        self.journal.phase = Phase::RollingBack;
        self.save(faults)?;
        self.release(true, adapter, faults)
    }
}
