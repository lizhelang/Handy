//! 真正的启动短期锁：恢复、初始化、激活和原请求重放共享同一个 Files guard。
use super::*;
use crate::data_migration::{v3::*, MigrationSourceRoot};
use inputia_settings::store::{InputSettingsSchema, PendingStatus};

pub(in crate::settings) struct StartupStore {
    pub store: DocumentStore<Schema>,
    _inputia: Option<DocumentStore<InputSettingsSchema>>,
    path: PathBuf,
    home: PathBuf,
    uid: u32,
}
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum Reconciliation {
    Saved {
        operation_id: String,
        commit_revision: String,
    },
    Conflict {
        operation_id: String,
    },
}
pub(in crate::settings) struct PreparedSettings {
    pub loaded: LoadedSettings,
    pub transaction: Option<StartupTransaction>,
    pub reconciliation: Option<Reconciliation>,
}
impl StartupStore {
    /// 调用方必须已持迁移锁；固定顺序为控制中心→IME，避免反向等待。
    pub fn open(root: &Path, inputia: Option<&Path>, home: &Path, uid: u32) -> Result<Self, Error> {
        if inputia.is_some_and(|inputia| inputia == root) {
            return Err(Error::UnsafePath);
        }
        let path = root.join(Schema::FILE_NAME);
        let store = DocumentStore::<Schema>::open(&path, home, uid)?;
        let inputia = inputia
            .map(|root| {
                DocumentStore::<InputSettingsSchema>::open(
                    &root.join(InputSettingsSchema::FILE_NAME),
                    home,
                    uid,
                )
            })
            .transpose()?;
        Ok(Self {
            store,
            _inputia: inputia,
            path,
            home: home.into(),
            uid,
        })
    }
    pub fn inspect(&self) -> Result<StartupNeed, Error> {
        if !entry_exists(&self.path.with_file_name(Schema::MARKER_NAME))? {
            if entry_exists(
                &self
                    .path
                    .with_file_name(Schema::PENDING_NAME.ok_or(Error::RepairRequired)?),
            )? {
                return Err(Error::RepairRequired);
            }
            self.store.preflight()?;
            return Ok(StartupNeed::SettingsProtocol);
        }
        // Active 外部导入只有原预览/回执可以通过，普通 preflight 不得代替此入口。
        self.store.preflight_pending_recovery()?;
        match self.store.pending_status()? {
            PendingStatus::RequiresActivation => Ok(StartupNeed::SettingsProtocol),
            PendingStatus::Active { .. } => Ok(StartupNeed::SettingsReplay),
            PendingStatus::Idle => Ok(StartupNeed::NoWork),
            PendingStatus::Resolved { outcome, .. }
                if outcome == "saved" || outcome == "conflict" =>
            {
                Ok(StartupNeed::NoWork)
            }
            _ => Err(Error::RepairRequired),
        }
    }
    pub fn prepare(self, preparation: StartupPreparation) -> anyhow::Result<PreparedSettings> {
        let mut reconciliation = None;
        let mut transaction = match preparation {
            StartupPreparation::NoWork(observed) => {
                observed.recheck(&[MigrationSourceRoot {
                    label: "handy".into(),
                    root: self.path.parent().ok_or(Error::UnsafePath)?.into(),
                }])?;
                None
            }
            StartupPreparation::Attempt(mut transaction) => {
                transaction.begin_mutations()?;
                match transaction.purpose() {
                    StartupPurpose::FullStartup | StartupPurpose::SettingsProtocol => {
                        // 不能先对 Active import 做普通 read，它会拒绝合法的原外部预览。
                        if self.inspect()? != StartupNeed::SettingsReplay {
                            self.store.read_observed_initialization(&mut |intent| {
                                transaction
                                    .record_settings_initialization(intent)
                                    .map_err(|_| Error::CommitUncertain)
                            })?;
                            if matches!(
                                self.store.pending_status()?,
                                PendingStatus::RequiresActivation
                            ) {
                                self.store.activate_pending_protocol(&mut |intent| {
                                    transaction
                                        .record_protocol_activation(intent)
                                        .map_err(|_| Error::CommitUncertain)
                                })?;
                            }
                        }
                    }
                    StartupPurpose::SettingsReplay => {}
                }
                if let PendingStatus::Active { operation_id } = self.store.pending_status()? {
                    let outcome = self.store.reconcile_pending_observed(&mut |intent| {
                        transaction
                            .record_settings_transition(intent)
                            .map_err(|_| Error::CommitUncertain)
                    })?;
                    reconciliation = Some(match outcome {
                        Some(ApplyResult::Saved {
                            commit_revision, ..
                        }) => Reconciliation::Saved {
                            operation_id,
                            commit_revision,
                        },
                        Some(ApplyResult::Conflict { .. }) => {
                            Reconciliation::Conflict { operation_id }
                        }
                        // Expired/未知从来不能关闭 attempt 或启动业务。
                        Some(ApplyResult::OutcomeExpired { .. }) | None => {
                            anyhow::bail!(Error::CommitUncertain)
                        }
                    });
                }
                match transaction.purpose() {
                    StartupPurpose::FullStartup => Some(transaction),
                    StartupPurpose::SettingsProtocol => {
                        transaction.complete(StartupConfirmation::ProtocolReady, || {
                            ensure_ready(&self.store).map_err(Into::into)
                        })?;
                        None
                    }
                    StartupPurpose::SettingsReplay => {
                        let confirmation =
                            match reconciliation.as_ref().ok_or(Error::RepairRequired)? {
                                Reconciliation::Saved { operation_id, .. } => {
                                    StartupConfirmation::RequestSaved {
                                        operation_id: operation_id.clone(),
                                    }
                                }
                                Reconciliation::Conflict { operation_id } => {
                                    StartupConfirmation::RequestConflict {
                                        operation_id: operation_id.clone(),
                                    }
                                }
                            };
                        transaction.complete(confirmation, || {
                            ensure_ready(&self.store).map_err(Into::into)
                        })?;
                        None
                    }
                }
            }
        };
        // 本次 Files 锁仍在；不能将早先 NoWork 或 read() 返回当作 Ready 授权。
        confirm_ready(&self.store)?;
        let snapshot = self.store.read()?;
        ensure_ready(&self.store)?;
        let loaded = LoadedSettings::from_snapshot(
            snapshot,
            self.path.clone(),
            self.home.clone(),
            self.uid,
        )?;
        // Store 在返回后释放；manager 的保存自行取得同锁并携带原启动 observer。
        Ok(PreparedSettings {
            loaded,
            transaction: transaction.take(),
            reconciliation,
        })
    }
}
fn entry_exists(path: &Path) -> Result<bool, Error> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(Error::StorageUnavailable),
    }
}
pub(super) fn ensure_ready(store: &DocumentStore<Schema>) -> Result<(), Error> {
    match store.pending_status()? {
        PendingStatus::Idle => Ok(()),
        PendingStatus::Resolved { outcome, .. } if outcome == "saved" || outcome == "conflict" => {
            Ok(())
        }
        PendingStatus::Active { .. } => Err(Error::PendingOperation),
        PendingStatus::RequiresActivation | PendingStatus::Disabled => {
            Err(Error::PendingProtocolRequired)
        }
        _ => Err(Error::RepairRequired),
    }
}

/// Resolved 的 rename 可能已完成而同步未知；只补同步，不写任何新请求或执行原生副作用。
pub(super) fn confirm_ready(store: &DocumentStore<Schema>) -> Result<(), Error> {
    ensure_ready(store)?;
    match store.reconcile_pending()? {
        None | Some(ApplyResult::Saved { .. }) | Some(ApplyResult::Conflict { .. }) => {
            ensure_ready(store)
        }
        Some(ApplyResult::OutcomeExpired { .. }) => Err(Error::CommitUncertain),
    }
}
