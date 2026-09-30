use inputia_settings::installation::InstallationReceipt;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::PathBuf};

#[derive(Debug)]
pub enum Error {
    Invalid(&'static str),
    UnsafePath,
    PermissionRequired,
    MissingArtifact,
    ArtifactMismatch,
    Busy,
    TransactionReused,
    NeedsRepair(&'static str),
    Adapter(&'static str),
    InjectedCrash,
    Io(std::io::Error),
    Json(serde_json::Error),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for Error {}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        match e.raw_os_error() {
            Some(libc::EACCES | libc::EPERM | libc::EROFS) => Self::PermissionRequired,
            Some(libc::ELOOP | libc::ENOTDIR) => Self::UnsafePath,
            Some(libc::ENOENT) => Self::MissingArtifact,
            _ => Self::Io(e),
        }
    }
}
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Control,
    Ime,
    Settings,
    PairManifest,
    Receipt,
}
impl Role {
    pub const COMPONENTS: [Self; 3] = [Self::Control, Self::Ime, Self::Settings];
    pub fn label(self) -> &'static str {
        match self {
            Self::Control => "control",
            Self::Ime => "ime",
            Self::Settings => "settings",
            Self::PairManifest => "pair",
            Self::Receipt => "receipt",
        }
    }
}

/// 摘要包括类型、相对路径、权限、长度和正文；不跟随链接，不接受多硬链接文件。
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fingerprint {
    pub sha256: String,
    pub bytes: u64,
    pub entries: u64,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Artifact {
    pub role: Role,
    pub source: PathBuf,
    pub expected: Fingerprint,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallRequest {
    pub transaction_id: String,
    pub new_receipt: InstallationReceipt,
    pub artifacts: Vec<Artifact>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub role: Role,
    pub source: Option<PathBuf>,
    pub destination: PathBuf,
    pub stage: PathBuf,
    pub backup: PathBuf,
    pub failed: PathBuf,
    pub old: Option<Fingerprint>,
    pub new: Fingerprint,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreparedPlan {
    pub schema_version: u32,
    pub home: PathBuf,
    pub uid: u32,
    pub request: InstallRequest,
    pub old_receipt: Option<InstallationReceipt>,
    pub old_pair_manifest: Option<FileEvidence>,
    pub entries: Vec<Entry>,
    pub required_free_bytes: u64,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Subject {
    pub transaction_id: String,
    pub plan_sha256: String,
    pub installation_id: String,
    pub new_release_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Prepared,
    Staged,
    Quiesced,
    SnapshotReady,
    Replacing,
    PairVerified,
    Activated,
    Committed,
    WritesReleased,
    RollingBack,
    RolledBack,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Stage { role: Role },
    Backup { role: Role },
    Install { role: Role },
    PreserveFailed { role: Role },
    RestoreOld { role: Role },
    ReleaseWrites,
    ReleaseRollback,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaintenanceMarker {
    pub schema_version: u32,
    pub transaction_id: String,
    pub installation_id: String,
    pub old_release_id: Option<String>,
    pub new_release_id: String,
    pub epoch: String,
    pub plan_sha256: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileEvidence {
    pub path: PathBuf,
    pub fingerprint: Fingerprint,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryEnvironmentReceipt {
    pub subject: Subject,
    pub bootstrap: FileEvidence,
    pub helper: FileEvidence,
    pub recovery_registration_sha256: String,
    pub journal_contract: u32,
}
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationCheck {
    SecurityAllArchitectures,
    HardenedRuntime,
    ReleaseSignature,
    PairSignature,
    BundleMetadata,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationReceipt {
    pub subject: Subject,
    pub roles: BTreeSet<Role>,
    pub artifact_set_sha256: String,
    pub checks: BTreeSet<VerificationCheck>,
    pub evidence_sha256: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentSnapshot {
    pub subject: Subject,
    pub previously_running_roles: BTreeSet<Role>,
    pub previous_input_source: String,
    pub process_snapshot_sha256: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QuiescenceReceipt {
    pub subject: Subject,
    pub epoch: String,
    pub stopped_roles: BTreeSet<Role>,
    pub previously_running_roles: BTreeSet<Role>,
    pub previous_input_source: String,
    pub process_snapshot_sha256: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataDomain {
    History,
    Clipboard,
    Integration,
    InputiaMemory,
    Rime,
    Settings,
    Attachments,
    OutputLedger,
    DeletionLedger,
}
impl DataDomain {
    pub fn all() -> BTreeSet<Self> {
        [
            Self::History,
            Self::Clipboard,
            Self::Integration,
            Self::InputiaMemory,
            Self::Rime,
            Self::Settings,
            Self::Attachments,
            Self::OutputLedger,
            Self::DeletionLedger,
        ]
        .into_iter()
        .collect()
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotReceipt {
    pub subject: Subject,
    pub epoch: String,
    pub covers: BTreeSet<DataDomain>,
    pub files: Vec<FileEvidence>,
    pub consistency_evidence_sha256: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionState {
    Available,
    MicrophoneMissing,
    AccessibilityMissing,
    BothMissing,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PostcheckReceipt {
    pub subject: Subject,
    pub release_id: String,
    pub profile_id: String,
    pub receipt_fingerprint: Fingerprint,
    pub epoch: String,
    pub handshake_and_schema_evidence_sha256: String,
    pub permissions: PermissionState,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RollbackCompatibilityReceipt {
    pub subject: Subject,
    pub epoch: String,
    pub target_release_id: String,
    pub tested_domains: BTreeSet<DataDomain>,
    pub read_write_outbox_privacy_evidence_sha256: String,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputSourceRestoration {
    Restored,
    NeedsUserAction,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestorationReceipt {
    pub subject: Subject,
    pub input_source: InputSourceRestoration,
    pub restored_running_roles: BTreeSet<Role>,
    pub evidence_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VerificationPurpose {
    DownloadedNew,
    StagedNew,
    InstalledNew,
    RollbackOld { release_id: String },
}

/// 生产实现必须核验真实 Security/TIS/内核进程/SQLite 快照结果；合成测试实现不构成产品验收。
pub trait NativeAdapter {
    fn prepare_recovery_environment(
        &mut self,
        subject: &Subject,
        updater_root: &std::path::Path,
    ) -> Result<RecoveryEnvironmentReceipt>;
    fn verify_artifacts(
        &mut self,
        subject: &Subject,
        locations: &[Entry],
        purpose: VerificationPurpose,
    ) -> Result<VerificationReceipt>;
    fn capture_environment(&mut self, subject: &Subject) -> Result<EnvironmentSnapshot>;
    fn quiesce(
        &mut self,
        subject: &Subject,
        marker: &MaintenanceMarker,
        previous: &EnvironmentSnapshot,
    ) -> Result<QuiescenceReceipt>;
    fn assert_quiesced(&mut self, subject: &Subject, marker: &MaintenanceMarker) -> Result<()>;
    fn snapshot(
        &mut self,
        subject: &Subject,
        marker: &MaintenanceMarker,
        directory: &std::path::Path,
    ) -> Result<SnapshotReceipt>;
    fn postcheck(
        &mut self,
        subject: &Subject,
        plan: &PreparedPlan,
        marker: &MaintenanceMarker,
    ) -> Result<PostcheckReceipt>;
    /// 在本轮停写后检查当前数据；静态版本能力不能代替此回执，每次恢复均重新检查。
    fn rollback_compatibility(
        &mut self,
        subject: &Subject,
        old: &InstallationReceipt,
        marker: &MaintenanceMarker,
    ) -> Result<RollbackCompatibilityReceipt>;
    fn restore_environment(
        &mut self,
        subject: &Subject,
        quiescence: &QuiescenceReceipt,
        rolled_back: bool,
    ) -> Result<RestorationReceipt>;
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FaultPoint {
    BeforeMarkerRemoval { rolled_back: bool },
    AfterMarkerRemoval { rolled_back: bool },
    BeforeRename(Action),
    AfterRename(Action),
    BeforeJournal(Phase),
    AfterJournal(Phase),
}
pub trait FaultInjector {
    fn checkpoint(&mut self, point: FaultPoint) -> Result<()>;
}
pub struct NoFaults;
impl FaultInjector for NoFaults {
    fn checkpoint(&mut self, _: FaultPoint) -> Result<()> {
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryPolicy {
    Resume,
    BinaryRollback,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Journal {
    pub schema_version: u32,
    pub subject: Subject,
    pub plan: PreparedPlan,
    pub phase: Phase,
    pub maintenance_epoch: String,
    pub environment: Option<EnvironmentSnapshot>,
    pub intent: Option<Action>,
    pub recovery: Option<RecoveryEnvironmentReceipt>,
    pub quiescence: Option<QuiescenceReceipt>,
    pub snapshot: Option<SnapshotReceipt>,
    pub postcheck: Option<PostcheckReceipt>,
    pub restoration: Option<RestorationReceipt>,
    pub writes_released: bool,
    pub rollback_requested: bool,
    pub rollback_compatibility: Option<RollbackCompatibilityReceipt>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArchiveKind {
    File,
    Directory,
    Symlink,
    Hardlink,
    Special,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArchiveEntry {
    pub path: PathBuf,
    pub kind: ArchiveKind,
    pub unpacked_bytes: u64,
    pub link_target: Option<PathBuf>,
}
