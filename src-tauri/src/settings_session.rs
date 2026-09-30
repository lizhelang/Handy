//! 控制中心设置的进程内协调：读取不等待磁盘，保存始终绑定原始快照。
use super::{document, AppSettings};
use document::{LoadedSettings, PlannedChange, SavedSettings};
use inputia_settings::store::Error;
use serde::Serialize;
use std::{
    path::Path,
    sync::{Arc, Mutex, RwLock},
};

/// 只由当前协调器创建；不进入 JSON、Specta 或日志。
#[derive(Clone)]
pub(crate) struct ReadTicket {
    owner: Arc<()>,
    loaded: Arc<LoadedSettings>,
}

pub(super) struct Coordinator {
    owner: Arc<()>,
    cached: RwLock<Arc<LoadedSettings>>,
    writer: Mutex<WriterState>,
}
#[derive(Default)]
struct WriterState {
    pending: Option<PendingSave>,
    blocked: Option<Error>,
}
struct PendingSave {
    plan: PlannedChange,
    uncertain: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "code", rename_all = "snake_case")]
pub(super) enum Failure {
    Storage { error: Error },
    MissingReadTicket,
    Conflict,
    Pending { operation_id: String },
    OutcomeExpired { operation_id: String },
    WrongOperation,
}
impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Self::Storage { error }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum SaveReport {
    Unchanged,
    Saved {
        operation_id: String,
        commit_revision: String,
        current_revision: String,
        replayed: bool,
        changed_fields: Vec<String>,
    },
}

impl Coordinator {
    pub fn open(path: &Path, home: &Path, uid: u32) -> Result<Self, Failure> {
        Ok(Self {
            owner: Arc::new(()),
            cached: RwLock::new(Arc::new(LoadedSettings::read(path, home, uid)?)),
            writer: Mutex::new(WriterState::default()),
        })
    }

    pub fn read(&self) -> Result<AppSettings, Failure> {
        let loaded = self
            .cached
            .read()
            .map_err(|_| Error::StorageUnavailable)?
            .clone();
        let mut settings = loaded.settings().clone();
        settings.read_ticket = Some(ReadTicket {
            owner: self.owner.clone(),
            loaded,
        });
        Ok(settings)
    }

    pub fn save(&self, settings: &AppSettings) -> Result<SaveReport, Failure> {
        self.save_with(settings, |plan, floor| plan.apply_at_least(floor))
    }

    fn save_with(
        &self,
        settings: &AppSettings,
        apply: impl FnOnce(&PlannedChange, &LoadedSettings) -> Result<SavedSettings, Error>,
    ) -> Result<SaveReport, Failure> {
        let ticket = settings
            .read_ticket
            .as_ref()
            .filter(|ticket| Arc::ptr_eq(&ticket.owner, &self.owner))
            .ok_or(Failure::MissingReadTicket)?;
        let mut writer = self.writer.lock().map_err(|_| Error::StorageUnavailable)?;
        if let Some(pending) = &writer.pending {
            return Err(Failure::Pending {
                operation_id: pending.plan.operation_id().into(),
            });
        }
        if let Some(error) = &writer.blocked {
            return Err(error.clone().into());
        }
        let Some(plan) = ticket.loaded.plan(settings)? else {
            return Ok(SaveReport::Unchanged);
        };
        // 先保留原请求；任何不确定结果都不能使下一次编辑生成新 ID 覆盖它。
        writer.pending = Some(PendingSave {
            plan,
            uncertain: false,
        });
        self.apply_pending(&mut writer, apply)
    }

    pub fn pending_operation(&self) -> Result<Option<String>, Failure> {
        let writer = self.writer.lock().map_err(|_| Error::StorageUnavailable)?;
        Ok(writer
            .pending
            .as_ref()
            .map(|pending| pending.plan.operation_id().into()))
    }

    /// 明确刷新时才访问文件；旧版/错域文件不能取代进程已确认的快照。
    pub fn refresh(&self) -> Result<AppSettings, Failure> {
        let mut writer = self.writer.lock().map_err(|_| Error::StorageUnavailable)?;
        if let Some(pending) = &writer.pending {
            return Err(Failure::Pending {
                operation_id: pending.plan.operation_id().into(),
            });
        }
        let loaded = self
            .cached
            .read()
            .map_err(|_| Error::StorageUnavailable)?
            .clone();
        let current = match loaded.reload() {
            Ok(current) => current,
            Err(error) => {
                writer.blocked = Some(error.clone());
                return Err(error.into());
            }
        };
        if let Err(error) = self.publish(current) {
            writer.blocked = Some(error.clone());
            return Err(error.into());
        }
        writer.blocked = None;
        self.read()
    }

    fn publish(&self, current: LoadedSettings) -> Result<(), Error> {
        let mut cached = self.cached.write().map_err(|_| Error::StorageUnavailable)?;
        if !current.follows(&cached) {
            return Err(Error::RepairRequired);
        }
        *cached = Arc::new(current);
        Ok(())
    }

    pub fn retry(&self, operation_id: &str) -> Result<SaveReport, Failure> {
        let mut writer = self.writer.lock().map_err(|_| Error::StorageUnavailable)?;
        if !writer
            .pending
            .as_ref()
            .is_some_and(|pending| pending.plan.operation_id() == operation_id)
        {
            return Err(Failure::WrongOperation);
        }
        self.apply_pending(&mut writer, |plan, floor| plan.apply_at_least(floor))
    }

    fn apply_pending(
        &self,
        writer: &mut WriterState,
        apply: impl FnOnce(&PlannedChange, &LoadedSettings) -> Result<SavedSettings, Error>,
    ) -> Result<SaveReport, Failure> {
        let pending = writer.pending.as_mut().ok_or(Failure::WrongOperation)?;
        let operation_id = pending.plan.operation_id().to_owned();
        let changed_fields = pending.plan.changed_fields();
        let floor = self
            .cached
            .read()
            .map_err(|_| Error::StorageUnavailable)?
            .clone();
        match apply(&pending.plan, &floor) {
            Ok(SavedSettings::Saved {
                current,
                commit_revision,
                replayed,
            }) => {
                let current_revision = current.revision().to_owned();
                // 持久成功后若发布缓存失败仍保留原请求，不能让调用者误以为未写。
                self.publish(current).map_err(|error| {
                    pending.uncertain = true;
                    writer.blocked = Some(error);
                    Failure::Pending {
                        operation_id: operation_id.clone(),
                    }
                })?;
                writer.pending = None;
                writer.blocked = None;
                Ok(SaveReport::Saved {
                    operation_id,
                    commit_revision,
                    current_revision,
                    replayed,
                    changed_fields,
                })
            }
            Ok(SavedSettings::Conflict { current }) => {
                if let Err(error) = self.publish(current) {
                    writer.blocked = Some(error.clone());
                    if pending.uncertain {
                        return Err(Failure::Pending { operation_id });
                    }
                    writer.pending = None;
                    return Err(error.into());
                }
                writer.pending = None;
                Err(Failure::Conflict)
            }
            Ok(SavedSettings::OutcomeExpired { current }) => {
                pending.uncertain = true;
                if let Err(error) = self.publish(current) {
                    writer.blocked = Some(error);
                    return Err(Failure::Pending { operation_id });
                }
                Err(Failure::OutcomeExpired { operation_id })
            }
            Err(error) => {
                if error == Error::RepairRequired {
                    writer.blocked = Some(error.clone());
                }
                if pending.uncertain || error == Error::CommitUncertain {
                    pending.uncertain = true;
                    Err(Failure::Pending { operation_id })
                } else {
                    writer.pending = None;
                    Err(error.into())
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{get_default_settings, Theme, SETTINGS_STORE_PATH};
    fn fixture() -> (tempfile::TempDir, Coordinator) {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().canonicalize().unwrap();
        let coordinator = Coordinator::open(&home.join(SETTINGS_STORE_PATH), &home, unsafe {
            libc::geteuid()
        })
        .unwrap();
        (temp, coordinator)
    }

    #[test]
    fn stale_read_conflicts_without_overwriting_a_different_field() {
        let (_temp, coordinator) = fixture();
        let mut first = coordinator.read().unwrap();
        let mut stale = coordinator.read().unwrap();
        first.theme = Theme::Dark;
        assert!(matches!(
            coordinator.save(&first),
            Ok(SaveReport::Saved { .. })
        ));
        stale.audio_feedback = true;
        assert_eq!(coordinator.save(&stale), Err(Failure::Conflict));
        let current = coordinator.read().unwrap();
        assert!(matches!(current.theme, Theme::Dark));
        assert!(!current.audio_feedback);
    }

    #[test]
    fn read_ticket_cannot_be_supplied_by_json_or_another_coordinator() {
        let (_temp, coordinator) = fixture();
        assert_eq!(
            coordinator.save(&get_default_settings()),
            Err(Failure::MissingReadTicket)
        );
        let settings = coordinator.read().unwrap();
        let value = serde_json::to_value(&settings).unwrap();
        assert!(value.get("read_ticket").is_none());
        let decoded = serde_json::from_value(value).unwrap();
        assert_eq!(coordinator.save(&decoded), Err(Failure::MissingReadTicket));
        let (_other_temp, other) = fixture();
        assert_eq!(other.save(&settings), Err(Failure::MissingReadTicket));
    }

    #[test]
    fn lost_commit_confirmation_keeps_exact_request_and_blocks_new_writes() {
        let (_temp, coordinator) = fixture();
        let mut settings = coordinator.read().unwrap();
        settings.theme = Theme::Dark;
        let Err(Failure::Pending { operation_id }) =
            coordinator.save_with(&settings, |plan, floor| {
                plan.apply_at_least(floor)?;
                Err(Error::CommitUncertain)
            })
        else {
            panic!("lost confirmation must remain pending")
        };
        let mut newer = coordinator.read().unwrap();
        newer.audio_feedback = true;
        assert_eq!(
            coordinator.save(&newer),
            Err(Failure::Pending {
                operation_id: operation_id.clone()
            })
        );
        assert_eq!(coordinator.retry("wrong"), Err(Failure::WrongOperation));
        assert_eq!(
            coordinator.pending_operation().unwrap(),
            Some(operation_id.clone())
        );
        let SaveReport::Saved {
            commit_revision,
            replayed,
            operation_id: saved_id,
            ..
        } = coordinator.retry(&operation_id).unwrap()
        else {
            panic!("must recover")
        };
        assert_eq!(saved_id, operation_id);
        assert_eq!(commit_revision, "1");
        assert!(replayed);
        assert_eq!(coordinator.pending_operation().unwrap(), None);
        assert!(matches!(coordinator.read().unwrap().theme, Theme::Dark));
        assert!(!coordinator.read().unwrap().audio_feedback);
    }

    #[test]
    fn definite_failure_releases_the_request_without_publishing_unsaved_values() {
        let (_temp, coordinator) = fixture();
        let mut settings = coordinator.read().unwrap();
        settings.theme = Theme::Dark;
        assert_eq!(
            coordinator.save_with(&settings, |_, _| Err(Error::StorageUnavailable)),
            Err(Failure::Storage {
                error: Error::StorageUnavailable
            })
        );
        assert_eq!(coordinator.pending_operation().unwrap(), None);
        assert!(matches!(coordinator.read().unwrap().theme, Theme::System));
        assert!(matches!(
            coordinator.save(&settings),
            Ok(SaveReport::Saved { .. })
        ));
    }

    #[test]
    fn cached_reader_does_not_wait_for_the_writer_filesystem_operation() {
        use std::{sync::mpsc, time::Duration};
        let (_temp, coordinator) = fixture();
        let coordinator = Arc::new(coordinator);
        let worker_coordinator = coordinator.clone();
        let (started_tx, started_rx) = mpsc::channel();
        let (continue_tx, continue_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let mut settings = worker_coordinator.read().unwrap();
            settings.theme = Theme::Dark;
            worker_coordinator.save_with(&settings, |plan, floor| {
                started_tx.send(()).unwrap();
                continue_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                plan.apply_at_least(floor)
            })
        });
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        // 若 read 错误复用 writer 锁，本断言不能先于释放写者完成。
        assert!(matches!(coordinator.read().unwrap().theme, Theme::System));
        continue_tx.send(()).unwrap();
        assert!(matches!(
            worker.join().unwrap(),
            Ok(SaveReport::Saved { .. })
        ));
        assert!(matches!(coordinator.read().unwrap().theme, Theme::Dark));
    }

    #[test]
    fn refresh_rejects_file_rollback_and_keeps_the_last_confirmed_cache() {
        let (temp, coordinator) = fixture();
        let path = temp.path().join(SETTINGS_STORE_PATH);
        let mut first = coordinator.read().unwrap();
        first.theme = Theme::Dark;
        coordinator.save(&first).unwrap();
        let old_bytes = std::fs::read(&path).unwrap();
        let mut second = coordinator.read().unwrap();
        second.audio_feedback = true;
        coordinator.save(&second).unwrap();
        let latest_bytes = std::fs::read(&path).unwrap();
        std::fs::write(&path, &old_bytes).unwrap();
        assert!(matches!(
            coordinator.refresh(),
            Err(Failure::Storage {
                error: Error::RepairRequired
            })
        ));
        let mut cached = coordinator.read().unwrap();
        assert!(cached.audio_feedback);
        cached.theme = Theme::Light;
        assert_eq!(
            coordinator.save(&cached),
            Err(Failure::Storage {
                error: Error::RepairRequired
            })
        );
        assert_eq!(std::fs::read(&path).unwrap(), old_bytes);
        std::fs::write(&path, latest_bytes).unwrap();
        assert!(coordinator.refresh().unwrap().audio_feedback);
        assert!(matches!(
            coordinator.save(&cached),
            Ok(SaveReport::Saved { .. })
        ));
    }

    #[test]
    fn uncertain_operation_survives_replacement_of_the_whole_store() {
        let (temp, coordinator) = fixture();
        let mut edited = coordinator.read().unwrap();
        edited.theme = Theme::Dark;
        let Err(Failure::Pending { operation_id }) =
            coordinator.save_with(&edited, |plan, floor| {
                plan.apply_at_least(floor)?;
                Err(Error::CommitUncertain)
            })
        else {
            panic!("must retain original operation")
        };
        let (other_temp, _other) = fixture();
        let marker = ".inputia-control-settings-initialized.json";
        let names = [SETTINGS_STORE_PATH, marker];
        let original: Vec<_> = names
            .iter()
            .map(|name| std::fs::read(temp.path().join(name)).unwrap())
            .collect();
        for name in names {
            std::fs::write(
                temp.path().join(name),
                std::fs::read(other_temp.path().join(name)).unwrap(),
            )
            .unwrap();
        }
        assert_eq!(
            coordinator.retry(&operation_id),
            Err(Failure::Pending {
                operation_id: operation_id.clone()
            })
        );
        assert_eq!(
            coordinator.pending_operation().unwrap(),
            Some(operation_id.clone())
        );
        assert!(matches!(coordinator.read().unwrap().theme, Theme::System));
        for (name, bytes) in names.into_iter().zip(original) {
            std::fs::write(temp.path().join(name), bytes).unwrap();
        }
        assert!(matches!(
            coordinator.retry(&operation_id),
            Ok(SaveReport::Saved { replayed: true, .. })
        ));
    }

    #[test]
    fn replay_reports_original_commit_separately_from_a_newer_current_version() {
        let (temp, coordinator) = fixture();
        let mut edited = coordinator.read().unwrap();
        edited.theme = Theme::Dark;
        let Err(Failure::Pending { operation_id }) =
            coordinator.save_with(&edited, |plan, floor| {
                plan.apply_at_least(floor)?;
                Err(Error::CommitUncertain)
            })
        else {
            panic!("must retain original operation")
        };
        let home = temp.path().canonicalize().unwrap();
        let other = Coordinator::open(&home.join(SETTINGS_STORE_PATH), &home, unsafe {
            libc::geteuid()
        })
        .unwrap();
        let mut newer = other.read().unwrap();
        newer.audio_feedback = true;
        other.save(&newer).unwrap();
        let SaveReport::Saved {
            commit_revision,
            current_revision,
            replayed,
            changed_fields,
            ..
        } = coordinator.retry(&operation_id).unwrap()
        else {
            panic!("must recover receipt")
        };
        assert_eq!(commit_revision, "1");
        assert_eq!(current_revision, "2");
        assert!(replayed);
        assert_eq!(changed_fields, ["theme"]);
        assert!(coordinator.read().unwrap().audio_feedback);
    }

    #[test]
    fn old_ticket_cannot_write_into_a_file_restored_below_the_confirmed_floor() {
        let (temp, coordinator) = fixture();
        let path = temp.path().join(SETTINGS_STORE_PATH);
        let original = std::fs::read(&path).unwrap();
        let mut stale = coordinator.read().unwrap();
        let mut current = coordinator.read().unwrap();
        current.theme = Theme::Dark;
        coordinator.save(&current).unwrap();
        std::fs::write(&path, &original).unwrap();
        stale.audio_feedback = true;
        assert_eq!(
            coordinator.save(&stale),
            Err(Failure::Storage {
                error: Error::RepairRequired
            })
        );
        assert_eq!(std::fs::read(&path).unwrap(), original);
        assert!(matches!(coordinator.read().unwrap().theme, Theme::Dark));
    }

    #[test]
    fn active_coordinator_does_not_reinitialize_missing_or_flat_settings() {
        for flat_document in [false, true] {
            for attempt_save in [false, true] {
                let (temp, coordinator) = fixture();
                let path = temp.path().join(SETTINGS_STORE_PATH);
                let marker = temp
                    .path()
                    .join(".inputia-control-settings-initialized.json");
                std::fs::remove_file(&path).unwrap();
                std::fs::remove_file(&marker).unwrap();
                let flat = br#"{"settings":{"theme":"dark"}}"#;
                if flat_document {
                    std::fs::write(&path, flat).unwrap();
                }
                if attempt_save {
                    let mut edited = coordinator.read().unwrap();
                    edited.audio_feedback = true;
                    assert_eq!(
                        coordinator.save(&edited),
                        Err(Failure::Storage {
                            error: Error::RepairRequired
                        })
                    );
                } else {
                    assert!(matches!(
                        coordinator.refresh(),
                        Err(Failure::Storage {
                            error: Error::RepairRequired
                        })
                    ));
                }
                assert!(!marker.exists());
                if flat_document {
                    assert_eq!(std::fs::read(&path).unwrap(), flat);
                } else {
                    assert!(!path.exists());
                }
            }
        }
    }
}
