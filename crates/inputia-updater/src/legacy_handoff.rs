//! 旧路径永久fence、归档与一致快照的有界核心。当前没有生产origin发行者，不授权runtime接管。
//! 原子交换关闭路径重开窗口；源快照始终在fence持久且归档实例重新取得无旧FD证明之后。
use crate::{filesystem as fs, guardian::GuardianTransactionAuthority, Subject, Transaction};
use inputia_settings::{
    installation::{InstallationReceipt, LocatedInstallation, LocatorContext},
    memory_domain::{
        FileIdentity, MemoryFileBinding, OwnedMemoryDomainLease, DATABASE_NAME, DOMAIN_DIRECTORY,
        FENCE_RECORD, OLD_DATABASE_NAME,
    },
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    path::{Path, PathBuf},
};

const JOURNAL: &str = ".legacy-memory-handoff.json";
const SOURCE_NAMES: [&str; 4] = [
    OLD_DATABASE_NAME,
    "inputia_memory.db-wal",
    "inputia_memory.db-shm",
    "inputia_memory.db-journal",
];
const MAX_BYTES: u64 = 256 * 1024 * 1024;
#[derive(Debug)]
pub enum HandoffError {
    OriginProofRequired,
    LegacySourceMissing,
    RepairRequired(&'static str),
    BindingMismatch,
    Busy,
    Storage(crate::Error),
    Lease(inputia_settings::memory_domain::LeaseError),
    Sqlite(rusqlite::Error),
    Json(serde_json::Error),
    Io(std::io::Error),
}
impl std::fmt::Display for HandoffError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for HandoffError {}
impl From<crate::Error> for HandoffError {
    fn from(e: crate::Error) -> Self {
        Self::Storage(e)
    }
}
impl From<std::io::Error> for HandoffError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<rusqlite::Error> for HandoffError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}
impl From<serde_json::Error> for HandoffError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}
impl From<inputia_settings::memory_domain::LeaseError> for HandoffError {
    fn from(e: inputia_settings::memory_domain::LeaseError) -> Self {
        Self::Lease(e)
    }
}
type Result<T> = std::result::Result<T, HandoffError>;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    name: String,
    identity: Option<FileIdentity>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Content {
    bytes: u64,
    sha256: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fence {
    schema: u32,
    subject: Subject,
    epoch: String,
    sources: Vec<Source>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    fence: Fence,
    installation_id: String,
    profile_id: String,
    managed_dir: Option<FileIdentity>,
    archive_dir: Option<FileIdentity>,
    staged_fence: Option<FileIdentity>,
    fence_record: Option<FileIdentity>,
    service_lock: Option<FileIdentity>,
    workspace: Option<FileIdentity>,
    staged_database: Option<FileIdentity>,
    frozen_sources: Option<Vec<Option<Content>>>,
    snapshot: Option<Content>,
    ready: bool,
}
/// 仅描述文件层事实，绝不以Ready等价于原生origin授权或完整QuiescenceReceipt。
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct HandoffStatus {
    pub path_fenced: bool,
    pub snapshot_published: bool,
    pub origin_proof_required: bool,
    pub production_connected: bool,
}
/// 不可构造/Clone/Deserialize。发行者必须实际持续约束writer/未知子进程，并在fence后复核归档FD。
/// 暂停租约、JSON QuiescenceReceipt、UUID或数据库缺失都不能发行本类型。
type OriginCheck = dyn FnMut(&[PathBuf], bool) -> Result<()>;
pub struct VerifiedLegacyOrigin {
    subject: Subject,
    epoch: String,
    check: Box<OriginCheck>,
}

/// 固定profile、真实事务OFD和维护epoch绑定；不接受客户端数据库路径。
pub struct LegacyMemoryHandoff {
    authority: GuardianTransactionAuthority,
    location: LocatedInstallation,
    #[cfg(test)]
    fixture: bool,
}
impl LegacyMemoryHandoff {
    /// 学习域只读准备；Transaction负责建立维护标记，再重读其真实上下文的收据，不信任传入location。
    pub fn prepare(transaction: &Transaction) -> Result<Self> {
        let authority = transaction.guardian_authority()?;
        let plan = &transaction.journal().plan;
        let path = plan
            .home
            .join("Library/Application Support/Inputia/installation.json");
        let receipt: InstallationReceipt =
            serde_json::from_slice(&fs::read(&path, plan.uid, 16_384)?)?;
        if receipt != plan.request.new_receipt && plan.old_receipt.as_ref() != Some(&receipt) {
            return Err(HandoffError::BindingMismatch);
        }
        let location = receipt
            .clone()
            .resolve(&LocatorContext {
                product_id: "com.inputia".into(),
                release_id: receipt.release_id.clone(),
                uid: plan.uid,
                home: plan.home.clone(),
            })
            .map_err(|_| HandoffError::BindingMismatch)?;
        if location.receipt.installation_id != authority.subject.installation_id {
            return Err(HandoffError::BindingMismatch);
        }
        Ok(Self {
            authority,
            location,
            #[cfg(test)]
            fixture: false,
        })
    }
    /// 目前原生退出/FD集合发行者尚未接入；显式失败，不从既有暂停/TIS观察降级发行。
    pub fn origin_authority(&self) -> Result<VerifiedLegacyOrigin> {
        Err(HandoffError::OriginProofRequired)
    }
    fn root(&self) -> &Path {
        &self.location.inputia_root
    }
    fn managed(&self) -> PathBuf {
        self.root().join(DOMAIN_DIRECTORY)
    }
    fn archive(&self) -> PathBuf {
        self.managed().join("legacy-source")
    }
    fn staged_fence(&self) -> PathBuf {
        self.managed().join("fence-stage")
    }
    fn uid(&self) -> u32 {
        self.location.receipt.uid
    }
    fn authorize(&self) -> Result<()> {
        #[cfg(test)]
        if self.fixture {
            return Ok(());
        }
        crate::native_quiescence::actual_marker(&self.authority.subject, &self.authority.marker)
            .map_err(|_| HandoffError::BindingMismatch)?;
        Ok(())
    }
    fn save(&self, journal: &Journal) -> Result<()> {
        self.authorize()?;
        let raw = serde_json::to_vec(journal)?;
        if raw.len() > 32_768 {
            return Err(HandoffError::RepairRequired("journal_budget"));
        }
        fs::atomic_write(&self.root().join(JOURNAL), self.uid(), &raw)?;
        Ok(())
    }
    fn load(&self) -> Result<Option<Journal>> {
        let raw = match fs::read(&self.root().join(JOURNAL), self.uid(), 32_768) {
            Ok(raw) => raw,
            Err(crate::Error::MissingArtifact) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let value: Journal = serde_json::from_slice(&raw)?;
        if value.fence.schema != 1
            || value.fence.subject != self.authority.subject
            || value.fence.epoch != self.authority.marker.epoch
            || value.installation_id != self.location.receipt.installation_id
            || value.profile_id != self.location.receipt.profile_id
            || value.fence.sources.len() != SOURCE_NAMES.len()
            || value
                .fence
                .sources
                .iter()
                .zip(SOURCE_NAMES)
                .any(|(s, name)| s.name != name)
            || value.fence.sources[0].identity.is_none()
        {
            return Err(HandoffError::BindingMismatch);
        }
        Ok(Some(value))
    }
    fn initialize(&self) -> Result<Journal> {
        if identity(&self.managed(), self.uid(), true)?.is_some() {
            return Err(HandoffError::RepairRequired("unclaimed_managed_directory"));
        }
        let sources = SOURCE_NAMES
            .into_iter()
            .map(|name| {
                Ok(Source {
                    name: name.into(),
                    identity: identity(&self.root().join(name), self.uid(), false)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        if sources[0].identity.is_none() {
            return Err(HandoffError::LegacySourceMissing);
        }
        let value = Journal {
            fence: Fence {
                schema: 1,
                subject: self.authority.subject.clone(),
                epoch: self.authority.marker.epoch.clone(),
                sources,
            },
            installation_id: self.location.receipt.installation_id.clone(),
            profile_id: self.location.receipt.profile_id.clone(),
            managed_dir: None,
            archive_dir: None,
            staged_fence: None,
            fence_record: None,
            service_lock: None,
            workspace: None,
            staged_database: None,
            frozen_sources: None,
            snapshot: None,
            ready: false,
        };
        fs::write_new(
            &self.root().join(JOURNAL),
            self.uid(),
            &serde_json::to_vec(&value)?,
        )?;
        Ok(value)
    }
    /// 文件层完成后仍由调用者持有origin能力；返回的租约仅是合作文件排他，不是runtime-ready。
    pub fn advance(&self, origin: &mut VerifiedLegacyOrigin) -> Result<OwnedMemoryDomainLease> {
        self.drive(origin, &mut |_| Ok(()))
    }
    fn drive(
        &self,
        origin: &mut VerifiedLegacyOrigin,
        fault: &mut dyn FnMut(&str) -> Result<()>,
    ) -> Result<OwnedMemoryDomainLease> {
        if origin.subject != self.authority.subject || origin.epoch != self.authority.marker.epoch {
            return Err(HandoffError::BindingMismatch);
        }
        self.authorize()?;
        // 每次重新开独立OFD，防同Transaction重复advance共享flock而绕过排他。
        let _operation_lock =
            fs::lock(&self.root().join(".legacy-memory-handoff.lock"), self.uid())?;
        let mut journal = match self.load()? {
            Some(value) => value,
            None => self.initialize()?,
        };
        fault("journal")?;
        let pre_paths = SOURCE_NAMES
            .iter()
            .map(|n| self.root().join(n))
            .collect::<Vec<_>>();
        (origin.check)(&pre_paths, false)?;
        self.authorize()?;
        let mut field = journal.managed_dir;
        self.claim_directory(&self.managed(), &mut field)?;
        journal.managed_dir = field;
        self.save(&journal)?;
        let mut field = journal.archive_dir;
        self.claim_directory(&self.archive(), &mut field)?;
        journal.archive_dir = field;
        self.save(&journal)?;
        if journal.staged_fence.is_none() {
            let mut field = None;
            self.claim_directory(&self.staged_fence(), &mut field)?;
            journal.staged_fence = field;
            self.save(&journal)?;
        }
        let old = self.root().join(OLD_DATABASE_NAME);
        let fenced = identity(&old, self.uid(), true);
        let is_fenced = matches!(fenced, Ok(Some(id)) if Some(id) == journal.staged_fence);
        let fence_path = if is_fenced {
            old.clone()
        } else {
            self.staged_fence()
        };
        self.check_identity(&fence_path, journal.staged_fence, true)?;
        let fence_record = fence_path.join(FENCE_RECORD);
        if journal.fence_record.is_none() {
            if identity(&fence_record, self.uid(), false)?.is_some() {
                return Err(HandoffError::RepairRequired("unclaimed_fence_record"));
            }
            fs::write_new(
                &fence_record,
                self.uid(),
                &serde_json::to_vec(&journal.fence)?,
            )?;
            journal.fence_record = identity(&fence_record, self.uid(), false)?;
            self.save(&journal)?;
        }
        self.check_identity(&fence_record, journal.fence_record, false)?;
        if serde_json::from_slice::<Fence>(&fs::read(&fence_record, self.uid(), 16_384)?)?
            != journal.fence
        {
            return Err(HandoffError::RepairRequired("fence_evidence_changed"));
        }
        if !is_fenced {
            self.check_identity(&old, journal.fence.sources[0].identity, false)?;
            self.authorize()?;
            rename_effect(&old, &self.staged_fence(), self.uid(), true)?;
        }
        // 包括交换已生效但fsync/回执失败的重入：必须重新确认准确文件和两个父目录耐久。
        confirm_move(
            &self.staged_fence(),
            &old,
            self.uid(),
            "exchange_sync",
            fault,
        )?;
        fault("fence")?;
        // 永不自动撤fence；即使origin随后失败也保持旧路径无法SQLite打开。
        self.check_identity(&old, journal.staged_fence, true)?;
        for (index, source) in journal.fence.sources.iter().enumerate() {
            let from = if index == 0 {
                self.staged_fence()
            } else {
                self.root().join(&source.name)
            };
            let to = self.archive().join(&source.name);
            let from_id = identity(&from, self.uid(), false)?;
            let to_id = identity(&to, self.uid(), false)?;
            match source.identity {
                Some(expected) if from_id == Some(expected) && to_id.is_none() => {
                    self.authorize()?;
                    rename_effect(&from, &to, self.uid(), false)?;
                }
                Some(expected) if from_id.is_none() && to_id == Some(expected) => {}
                None if from_id.is_none() && to_id.is_none() => {}
                _ => return Err(HandoffError::RepairRequired("source_instance_changed")),
            }
            confirm_move(
                &from,
                &to,
                self.uid(),
                &format!("archive_sync_{index}"),
                fault,
            )?;
            fault(&format!("archive_{index}"))?;
        }
        let archived_paths = journal
            .fence
            .sources
            .iter()
            .filter(|s| s.identity.is_some())
            .map(|s| self.archive().join(&s.name))
            .collect::<Vec<_>>();
        (origin.check)(&archived_paths, true)?;
        self.authorize()?;
        self.audit_sources(&journal)?;
        let frozen = journal
            .fence
            .sources
            .iter()
            .map(|s| {
                s.identity
                    .map(|_| content(&self.archive().join(&s.name), self.uid()))
                    .transpose()
            })
            .collect::<Result<Vec<_>>>()?;
        if frozen.iter().flatten().map(|v| v.bytes).sum::<u64>() > MAX_BYTES {
            return Err(HandoffError::RepairRequired("source_budget"));
        }
        if journal
            .frozen_sources
            .as_ref()
            .is_some_and(|expected| expected != &frozen)
        {
            return Err(HandoffError::RepairRequired("frozen_source_changed"));
        }
        journal.frozen_sources = Some(frozen);
        self.save(&journal)?;
        fault("origin")?;
        let workspace = self.managed().join("snapshot-source");
        let staged = self.managed().join("snapshot.sqlite");
        let target = self.managed().join(DATABASE_NAME);
        if journal.snapshot.is_none() {
            let mut field = journal.workspace;
            self.claim_directory(&workspace, &mut field)?;
            journal.workspace = field;
            self.save(&journal)?;
            for (source, expected) in journal.fence.sources.iter().zip(
                journal
                    .frozen_sources
                    .as_ref()
                    .ok_or(HandoffError::BindingMismatch)?,
            ) {
                if let Some(expected) = expected {
                    let to = workspace.join(&source.name);
                    if identity(&to, self.uid(), false)?.is_none() {
                        fs::copy(&self.archive().join(&source.name), &to, self.uid())?;
                    }
                    if &content(&to, self.uid())? != expected {
                        return Err(HandoffError::RepairRequired("snapshot_workspace_changed"));
                    }
                }
            }
            if journal.staged_database.is_none() {
                if identity(&staged, self.uid(), false)?.is_some()
                    || identity(&target, self.uid(), false)?.is_some()
                {
                    return Err(HandoffError::RepairRequired("unclaimed_target"));
                }
                fs::write_new(&staged, self.uid(), b"")?;
                journal.staged_database = identity(&staged, self.uid(), false)?;
                self.save(&journal)?;
            }
            self.check_identity(&staged, journal.staged_database, false)?;
            fault("before_sqlite")?;
            // SQLite会主动消费/删除热journal；任何未登记sidecar都必须在open前拒绝并保留。
            for (source, expected) in journal.fence.sources.iter().zip(
                journal
                    .frozen_sources
                    .as_ref()
                    .ok_or(HandoffError::BindingMismatch)?,
            ) {
                let path = workspace.join(&source.name);
                match expected {
                    Some(expected) if &content(&path, self.uid())? == expected => {}
                    None if identity(&path, self.uid(), false)?.is_none() => {}
                    _ => return Err(HandoffError::RepairRequired("unclaimed_workspace_sidecar")),
                }
            }
            absent_sidecars(&staged, self.uid())?;
            self.authorize()?;
            snapshot_sqlite(&workspace.join(OLD_DATABASE_NAME), &staged)?;
            self.check_identity(&staged, journal.staged_database, false)?;
            fs::sync_file(&staged, self.uid())?;
            journal.snapshot = Some(content(&staged, self.uid())?);
            self.save(&journal)?;
        }
        fault("snapshot")?;
        match (
            identity(&staged, self.uid(), false)?,
            identity(&target, self.uid(), false)?,
        ) {
            (Some(id), None) if Some(id) == journal.staged_database => {
                self.authorize()?;
                rename_effect(&staged, &target, self.uid(), false)?;
            }
            (None, Some(id)) if Some(id) == journal.staged_database => {}
            _ => return Err(HandoffError::RepairRequired("target_instance_changed")),
        }
        confirm_move(&staged, &target, self.uid(), "publish_sync", fault)?;
        absent_sidecars(&target, self.uid())?;
        fault("published")?;
        if journal.snapshot.as_ref() != Some(&content(&target, self.uid())?) {
            return Err(HandoffError::RepairRequired("snapshot_content_changed"));
        }
        let lock_path = self.managed().join("service.lock");
        if journal.service_lock.is_none() {
            if identity(&lock_path, self.uid(), false)?.is_some() {
                return Err(HandoffError::RepairRequired("unclaimed_service_lock"));
            }
            fs::write_new(&lock_path, self.uid(), b"")?;
            journal.service_lock = identity(&lock_path, self.uid(), false)?;
            self.save(&journal)?;
        }
        self.check_identity(&lock_path, journal.service_lock, false)?;
        // 验证/backup都可能很慢；返回前再次核原生授权和完整盘面，日志不能自己充当证据。
        (origin.check)(&archived_paths, true)?;
        self.authorize()?;
        self.audit_sources(&journal)?;
        journal.ready = true;
        self.save(&journal)?;
        fault("ready")?;
        let lease = OwnedMemoryDomainLease::acquire(self.root(), self.uid(), &binding(&journal)?)?;
        (origin.check)(&archived_paths, true)?;
        self.authorize()?;
        lease.assert_current()?;
        Ok(lease)
    }
    fn claim_directory(&self, path: &Path, value: &mut Option<FileIdentity>) -> Result<()> {
        if value.is_none() {
            if identity(path, self.uid(), true)?.is_some() {
                return Err(HandoffError::RepairRequired("unclaimed_directory"));
            }
            self.authorize()?;
            fs::mkdir_new(path, self.uid())?;
            *value = identity(path, self.uid(), true)?;
        }
        self.check_identity(path, *value, true)
    }
    fn check_identity(
        &self,
        path: &Path,
        expected: Option<FileIdentity>,
        directory: bool,
    ) -> Result<()> {
        if expected.is_none() || identity(path, self.uid(), directory)? != expected {
            return Err(HandoffError::RepairRequired("file_identity_changed"));
        }
        Ok(())
    }
    fn audit_sources(&self, journal: &Journal) -> Result<()> {
        self.check_identity(&self.managed(), journal.managed_dir, true)?;
        self.check_identity(&self.archive(), journal.archive_dir, true)?;
        self.check_identity(
            &self.root().join(OLD_DATABASE_NAME),
            journal.staged_fence,
            true,
        )?;
        for (index, source) in journal.fence.sources.iter().enumerate() {
            if identity(&self.archive().join(&source.name), self.uid(), false)? != source.identity
                || (index > 0
                    && identity(&self.root().join(&source.name), self.uid(), false)?.is_some())
            {
                return Err(HandoffError::RepairRequired("source_set_changed"));
            }
        }
        Ok(())
    }
    /// 元数据诊断从实际目录/目标实例核算；不泄露正文，不将日志ready暴露成生产完成。
    pub fn status(&self) -> Result<HandoffStatus> {
        let journal = self.load()?;
        let (mut path_fenced, mut snapshot_published) = (false, false);
        if let Some(journal) = journal {
            path_fenced = matches!(identity(&self.root().join(OLD_DATABASE_NAME), self.uid(), true), Ok(Some(id)) if Some(id) == journal.staged_fence);
            snapshot_published = journal.staged_database.is_some()
                && identity(&self.managed().join(DATABASE_NAME), self.uid(), false)?
                    == journal.staged_database;
        }
        Ok(HandoffStatus {
            path_fenced,
            snapshot_published,
            origin_proof_required: true,
            production_connected: false,
        })
    }
}
fn binding(journal: &Journal) -> Result<MemoryFileBinding> {
    let required = |value: Option<FileIdentity>| {
        value.ok_or(HandoffError::RepairRequired("incomplete_file_evidence"))
    };
    Ok(MemoryFileBinding {
        database: required(journal.staged_database)?,
        fence: required(journal.staged_fence)?,
        fence_record: required(journal.fence_record)?,
        service_lock: required(journal.service_lock)?,
    })
}
fn identity(path: &Path, uid: u32, directory: bool) -> Result<Option<FileIdentity>> {
    let (parent, name) = match fs::parent(path, uid, false) {
        Ok(v) => v,
        Err(crate::Error::MissingArtifact) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let file = match fs::open_child(&parent, &name, directory) {
        Ok(v) => v,
        Err(crate::Error::MissingArtifact) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let md = file.metadata()?;
    if md.uid() != uid
        || md.mode() & 0o077 != 0
        || (directory && !md.is_dir())
        || (!directory && (!md.is_file() || md.nlink() != 1))
    {
        return Err(HandoffError::RepairRequired("unsafe_file"));
    }
    Ok(Some(FileIdentity::of(&file)?))
}
fn content(path: &Path, uid: u32) -> Result<Content> {
    let before =
        identity(path, uid, false)?.ok_or(HandoffError::RepairRequired("source_missing"))?;
    let (parent, name) = fs::parent(path, uid, false)?;
    let mut file = fs::open_child(&parent, &name, false)?;
    if FileIdentity::of(&file)? != before {
        return Err(HandoffError::RepairRequired("source_replaced"));
    }
    let md = file.metadata()?;
    if md.len() > MAX_BYTES {
        return Err(HandoffError::RepairRequired("source_budget"));
    }
    let mut digest = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0u8; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        if bytes > MAX_BYTES {
            return Err(HandoffError::RepairRequired("source_budget"));
        }
        digest.update(&buffer[..count]);
    }
    let after = file.metadata()?;
    if bytes != md.len()
        || md.len() != after.len()
        || md.mtime() != after.mtime()
        || md.mtime_nsec() != after.mtime_nsec()
        || identity(path, uid, false)? != Some(before)
    {
        return Err(HandoffError::RepairRequired("source_changed"));
    }
    Ok(Content {
        bytes,
        sha256: format!("{:x}", digest.finalize()),
    })
}
// 效应与耐久确认分开：确认在正常和盘面重入路径上相同执行，不能由rename结果推断fsync完成。
fn rename_effect(left: &Path, right: &Path, uid: u32, swap: bool) -> Result<()> {
    let (a, an) = fs::parent(left, uid, false)?;
    let (b, bn) = fs::parent(right, uid, false)?;
    #[cfg(target_os = "macos")]
    let result = unsafe {
        libc::renameatx_np(
            a.as_raw_fd(),
            an.as_ptr(),
            b.as_raw_fd(),
            bn.as_ptr(),
            if swap {
                libc::RENAME_SWAP
            } else {
                libc::RENAME_EXCL
            },
        )
    };
    #[cfg(target_os = "linux")]
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            a.as_raw_fd(),
            an.as_ptr(),
            b.as_raw_fd(),
            bn.as_ptr(),
            if swap { 2u32 } else { 1u32 },
        ) as i32
    };
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    return Err(HandoffError::RepairRequired("atomic_exchange_unsupported"));
    if result != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
fn confirm_move(
    source: &Path,
    destination: &Path,
    uid: u32,
    point: &str,
    fault: &mut dyn FnMut(&str) -> Result<()>,
) -> Result<()> {
    let (from, _) = fs::parent(source, uid, false)?;
    let (to, leaf) = fs::parent(destination, uid, false)?;
    match fs::open_child(&to, &leaf, false) {
        Ok(file) => file.sync_all()?,
        Err(crate::Error::MissingArtifact) => {} // 原集合明确absent的sidecar仍要确认目录状态。
        Err(e) => return Err(e.into()),
    }
    fault(point)?;
    from.sync_all()?;
    to.sync_all()?;
    Ok(())
}
fn absent_sidecars(database: &Path, uid: u32) -> Result<()> {
    for suffix in ["-wal", "-shm", "-journal"] {
        let sidecar = PathBuf::from(format!("{}{suffix}", database.display()));
        if identity(&sidecar, uid, false)?.is_some() {
            return Err(HandoffError::RepairRequired("unclaimed_target_sidecar"));
        }
    }
    Ok(())
}
fn snapshot_sqlite(source_path: &Path, target_path: &Path) -> Result<()> {
    let source = rusqlite::Connection::open_with_flags(
        source_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    source.busy_timeout(std::time::Duration::from_millis(100))?;
    let mut target = rusqlite::Connection::open_with_flags(
        target_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    target.busy_timeout(std::time::Duration::from_millis(100))?;
    {
        let backup = rusqlite::backup::Backup::new(&source, &mut target)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match backup.step(128)? {
                rusqlite::backup::StepResult::Done => break,
                rusqlite::backup::StepResult::More if std::time::Instant::now() < deadline => {}
                _ => return Err(HandoffError::Busy),
            }
        }
    }
    target.pragma_update(None, "journal_mode", "DELETE")?;
    let check: String = target.query_row("PRAGMA integrity_check(1)", [], |row| row.get(0))?;
    if check != "ok" {
        return Err(HandoffError::RepairRequired("snapshot_integrity"));
    }
    target.close().map_err(|(_, e)| HandoffError::Sqlite(e))?;
    source.close().map_err(|(_, e)| HandoffError::Sqlite(e))?;
    for suffix in ["-wal", "-shm", "-journal"] {
        if PathBuf::from(format!("{}{suffix}", target_path.display())).try_exists()? {
            return Err(HandoffError::RepairRequired("snapshot_sidecar_remaining"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
