//! 单一后台写入线程；Tauri/Host 通过请求队列访问，不在按键线程等待。

use crate::{
    attachment_store::{
        AttachmentBudget, AttachmentHealth, AttachmentImport, AttachmentKind, AttachmentLease,
        AttachmentMutationGate, AttachmentMutationGuard, AttachmentStore, PauseReason, PinPurpose,
    },
    deletion_lifecycle::{
        DeleteFailure, DeleteRecord, DeleteRequest, DeleteState, DELETE_SCHEMA_VERSION,
    },
    learning::{ApplyContribution, ContributionInput, HistoryTermConfirmation},
    source::SourceTable,
    store::{
        ContentRevision, HistoryQuery, IndexedItem, IntegrationStore, LearnedTermView, TermSnapshot,
    },
    sync::SourcePump,
};
use inputia_core::integration::{
    privacy::{PrivacyContext, PrivacyPolicy},
    terms::HotwordBudget,
};
use rusqlite::{Connection, OpenFlags};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, SyncSender},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

type ServiceResult<T> = Result<T, String>;
type Job = Box<dyn FnOnce(&mut ServiceResult<Worker>) + Send>;

/// GUI 发键边界只读取原子快照，不等待数据库或服务队列。
/// 所有新增的策略/删除写入口必须在写入前调用 Worker::revoke_outputs。
#[derive(Clone)]
pub struct OutputPermit {
    generation: Arc<AtomicU64>,
    expected: u64,
    expires: Instant,
}

#[derive(Default)]
struct SourceWrites {
    gate: Arc<AttachmentMutationGate>,
    active: AtomicUsize,
    generation: AtomicU64,
}

/// 源写入期间阻止新许可；Drop 后仍须由同步屏障消费源事务才可重新输出。
pub struct SourceWriteGuard {
    _attachment: AttachmentMutationGuard,
    source: Arc<SourceWrites>,
}
impl Drop for SourceWriteGuard {
    fn drop(&mut self) {
        self.source.generation.fetch_add(1, Ordering::AcqRel);
        self.source.active.fetch_sub(1, Ordering::AcqRel);
    }
}

impl OutputPermit {
    pub fn check(&self) -> ServiceResult<()> {
        if self.generation.load(Ordering::Acquire) == self.expected && Instant::now() < self.expires
        {
            Ok(())
        } else {
            Err("output permission expired or was revoked".into())
        }
    }
}

struct Worker {
    output_generation: Arc<AtomicU64>,
    source_writes: Arc<SourceWrites>,
    _writer_lease: std::fs::File,
    attachments: AttachmentStore,
    attachment_audit: Option<(u64, u64)>,
    attachment_pause: Option<PauseReason>,
    maintenance_user: Option<inputia_settings::maintenance::UserContext>,
    next_attachment_maintenance: Instant,
    learning_key: [u8; 32],
    store: IntegrationStore,
    sources: Vec<SourcePump>,
    last_error: Option<String>,
    generation: u64,
    deletion_recovery_cursor: Option<String>,
    next_deletion_recovery: Instant,
    deletion_audit_cursor: Option<String>,
    deletion_audit_complete: bool,
    changed: Box<dyn Fn(u64) + Send>,
}

impl Worker {
    fn record_deletion_attachments(&mut self, request: &DeleteRequest) -> ServiceResult<()> {
        let source = self
            .sources
            .iter()
            .find(|s| s.store_id() == request.store_id)
            .ok_or("attachment source unavailable")?;
        let manifest = source
            .deleted_attachments(
                &request.item_id,
                request.expected_revision,
                &request.operation_id,
            )
            .map_err(|e| e.to_string())?;
        let retained: Vec<_> = self
            .store
            .retained_attachment_references()
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|r| r.store_id == request.store_id && r.record_id == request.record_id)
            .collect();
        self.attachments
            .record_manifest(
                self.store.attachment_connection(),
                request,
                manifest.as_deref(),
                &retained,
            )
            .map_err(|e| e.to_string())
    }
    fn audit_attachments(&mut self) -> ServiceResult<()> {
        let mut refs = Vec::new();
        for source in &mut self.sources {
            refs.extend(source.attachment_references().map_err(|e| e.to_string())?);
        }
        refs.extend(
            self.store
                .retained_attachment_references()
                .map_err(|e| e.to_string())?,
        );
        self.attachments
            .reconcile(self.store.attachment_connection(), &refs)
            .map_err(|e| e.to_string())?;
        self.attachment_audit = Some((
            self.generation,
            self.source_writes.generation.load(Ordering::Acquire),
        ));
        Ok(())
    }
    fn maintain_attachments(&mut self) {
        if Instant::now() < self.next_attachment_maintenance {
            return;
        }
        self.next_attachment_maintenance = Instant::now() + Duration::from_secs(1);
        if self.require_deletions_settled().is_err() || self.last_error.is_some() {
            self.attachment_pause = Some(PauseReason::SourceAuditPending);
            return;
        }
        let Some(exclusive) = self.source_writes.gate.try_gc() else {
            self.attachment_pause = Some(PauseReason::BusyPin);
            return;
        };
        let revision = (
            self.generation,
            self.source_writes.generation.load(Ordering::Acquire),
        );
        if self.attachment_audit != Some(revision) && self.audit_attachments().is_err() {
            self.attachment_pause = Some(PauseReason::SourceAuditPending);
            return;
        }
        let Some(user) = self.maintenance_user.as_ref() else {
            self.attachment_pause = Some(PauseReason::Maintenance);
            return;
        };
        self.attachment_pause = self
            .attachments
            .collect_one(self.store.attachment_connection(), &exclusive, user)
            .err()
            .map(|e| e.reason());
    }
    fn revoke_outputs(&self) {
        self.output_generation.fetch_add(1, Ordering::AcqRel);
    }

    fn require_deletions_settled(&self) -> ServiceResult<()> {
        if !self.deletion_audit_complete {
            return Err("history deletion recovery is verifying prior completion evidence".into());
        }
        if self
            .store
            .has_pending_deletions()
            .map_err(|e| e.to_string())?
        {
            return Err("history deletion recovery is pending; content access is suspended".into());
        }
        Ok(())
    }

    fn deletion_failure(
        &mut self,
        record: &DeleteRecord,
        failure: DeleteFailure,
        reject: bool,
    ) -> ServiceResult<DeleteRecord> {
        self.store
            .transition_deletion(
                &record.request,
                if reject {
                    DeleteState::Rejected
                } else {
                    record.state
                },
                Some(failure),
            )
            .map_err(|e| e.to_string())
    }

    fn resume_deletion(
        &mut self,
        mut record: DeleteRecord,
        sync_budget: usize,
    ) -> ServiceResult<DeleteRecord> {
        if record.state == DeleteState::Rejected {
            return Err(
                "deletion rejected before source commit; review the current revision".into(),
            );
        }
        self.revoke_outputs();
        let request = record.request.clone();
        if !self
            .store
            .deletion_source_is_current(&request)
            .map_err(|e| e.to_string())?
        {
            self.deletion_failure(&record, DeleteFailure::SourceIdentity, false)?;
            return Err("deletion projection source identity changed".into());
        }
        let Some(index) = self.sources.iter().position(|source| {
            source.store_id() == request.store_id
                && source.source_table().logical_name() == request.logical_name
        }) else {
            self.deletion_failure(&record, DeleteFailure::SourceIdentity, false)?;
            return Err("deletion source identity unavailable; original source required".into());
        };
        // 已提交回执优先。回执存在但源记录重新出现时不能再次删除新内容。
        let result = (|| {
            if !self.sources[index].verify_deleted_record(
                &request.record_id,
                request.expected_revision,
                &request.operation_id,
            )? {
                if record.state != DeleteState::Requested {
                    return Err(crate::sync::SyncError::Source(
                        crate::source::SourceError::ChangedAfterDeletion,
                    ));
                }
                for source in &self.sources {
                    // 同一操作 ID 不能在另一源库被用作其他修改；此时本源尚无回执。
                    source.delete_receipt(
                        &request.item_id,
                        request.expected_revision,
                        &request.operation_id,
                    )?;
                }
                self.sources[index].delete_record(
                    &request.record_id,
                    request.expected_revision,
                    &request.operation_id,
                )?;
            }
            if !self.sources[index].verify_deleted_record(
                &request.record_id,
                request.expected_revision,
                &request.operation_id,
            )? {
                return Err(crate::sync::SyncError::Source(
                    crate::source::SourceError::ChangedAfterDeletion,
                ));
            }
            Ok::<(), crate::sync::SyncError>(())
        })();
        if let Err(error) = result {
            use crate::{source::SourceError, sync::SyncError};
            let (failure, reject) = match error {
                SyncError::Source(SourceError::RevisionConflict) => (
                    DeleteFailure::SourceRevision,
                    record.state == DeleteState::Requested,
                ),
                SyncError::Source(SourceError::OperationConflict) => (
                    DeleteFailure::OperationConflict,
                    record.state == DeleteState::Requested,
                ),
                SyncError::Source(SourceError::WrongSource) => {
                    (DeleteFailure::SourceIdentity, false)
                }
                SyncError::Source(SourceError::ChangedAfterDeletion) => {
                    (DeleteFailure::SourceChangedAfterCommit, false)
                }
                _ => (DeleteFailure::SourceUnavailable, false),
            };
            self.deletion_failure(&record, failure, reject)?;
            return Err(format!("deletion pending or rejected: {failure:?}"));
        }
        self.record_deletion_attachments(&request)?;
        if record.state == DeleteState::Requested {
            record = self
                .store
                .transition_deletion(&request, DeleteState::SourceApplied, None)
                .map_err(|e| e.to_string())?;
        }
        for _ in 0..sync_budget {
            if let Err(error) = self.sync_once() {
                self.deletion_failure(&record, DeleteFailure::ProjectionUnavailable, false)?;
                return Err(error);
            }
            if self
                .store
                .deletion_projection_revoked(&request)
                .map_err(|e| e.to_string())?
            {
                // 同步之后再次核验源，避免提交期间源恢复/换库被当作删除完成。
                match self.sources[index].verify_deleted_record(
                    &request.record_id,
                    request.expected_revision,
                    &request.operation_id,
                ) {
                    Ok(true) => {}
                    _ => {
                        self.deletion_failure(
                            &record,
                            DeleteFailure::SourceChangedAfterCommit,
                            false,
                        )?;
                        return Err("source changed during deletion projection".into());
                    }
                }
                self.store
                    .transition_deletion(&request, DeleteState::ProjectionRevoked, None)
                    .map_err(|e| e.to_string())?;
                self.attachments
                    .schedule_deletion(self.store.attachment_connection(), &request)
                    .map_err(|e| e.to_string())?;
                return self
                    .store
                    .deletion_record(&request.operation_id)
                    .map_err(|e| e.to_string())?
                    .ok_or("deletion record missing".into());
            }
        }
        self.deletion_failure(&record, DeleteFailure::ProjectionUnavailable, false)?;
        Err("source deleted but projection recovery is still pending".into())
    }

    fn recover_deletions(&mut self) -> ServiceResult<()> {
        self.audit_deletions()?;
        if Instant::now() < self.next_deletion_recovery {
            return Ok(());
        }
        let mut pending = self
            .store
            .pending_deletions(self.deletion_recovery_cursor.as_deref())
            .map_err(|e| e.to_string())?;
        if pending.is_empty() && self.deletion_recovery_cursor.take().is_some() {
            pending = self
                .store
                .pending_deletions(None)
                .map_err(|e| e.to_string())?;
        }
        // 每轮最多一项、一个同步批次；轮转游标让失败项不能饿死后面的删除。
        let result = if let Some(record) = pending.pop() {
            self.deletion_recovery_cursor = Some(record.request.operation_id.clone());
            self.resume_deletion(record, 1).map(|_| ())
        } else {
            Ok(())
        };
        self.next_deletion_recovery = Instant::now() + Duration::from_millis(250);
        result
    }

    fn audit_deletions(&mut self) -> ServiceResult<()> {
        if self.deletion_audit_complete {
            return Ok(());
        }
        let records = self
            .store
            .deletion_audit_page(self.deletion_audit_cursor.as_deref())
            .map_err(|e| e.to_string())?;
        let finished = records.len() < 32;
        for record in records {
            let request = &record.request;
            if matches!(
                record.state,
                DeleteState::ProjectionRevoked | DeleteState::Rejected
            ) {
                if !self
                    .store
                    .deletion_source_is_current(request)
                    .map_err(|e| e.to_string())?
                {
                    return Err("deletion audit source identity changed".into());
                }
                let source = self
                    .sources
                    .iter_mut()
                    .find(|source| {
                        source.store_id() == request.store_id
                            && source.source_table().logical_name() == request.logical_name
                    })
                    .ok_or("deletion audit source unavailable")?;
                if record.state == DeleteState::ProjectionRevoked {
                    // 只核验已完成证据；绝不把声称完成的记录重新送入源删除。
                    if record.last_failure.is_some()
                        || !source
                            .verify_deleted_record(
                                &request.record_id,
                                request.expected_revision,
                                &request.operation_id,
                            )
                            .map_err(|e| e.to_string())?
                        || !self
                            .store
                            .deletion_projection_revoked(request)
                            .map_err(|e| e.to_string())?
                    {
                        return Err("deletion completion evidence missing or inconsistent".into());
                    }
                    self.record_deletion_attachments(request)?;
                    self.attachments
                        .schedule_deletion(self.store.attachment_connection(), request)
                        .map_err(|e| e.to_string())?;
                } else {
                    if !matches!(
                        record.last_failure,
                        Some(DeleteFailure::SourceRevision | DeleteFailure::OperationConflict)
                    ) {
                        return Err("deletion rejection evidence missing".into());
                    }
                    // 拒绝证据取源事务的持久 CAS 观察，不能靠统一库 phase/失败标签自证。
                    // 后来的正常修订可能恰好达到旧 expected，因此不能事后再比较当前修订。
                    if record.last_failure == Some(DeleteFailure::SourceRevision)
                        && source
                            .delete_receipt(
                                &request.item_id,
                                request.expected_revision,
                                &request.operation_id,
                            )
                            .map_err(|e| e.to_string())?
                            != Some(false)
                    {
                        return Err("deletion source rejection receipt missing".into());
                    }
                    let mut conflicting = false;
                    for source in &self.sources {
                        match source.delete_receipt(
                            &request.item_id,
                            request.expected_revision,
                            &request.operation_id,
                        ) {
                            Ok(None) => {}
                            Ok(Some(false))
                                if source.store_id() == request.store_id
                                    && record.last_failure
                                        == Some(DeleteFailure::SourceRevision) => {}
                            Ok(Some(_)) => {
                                return Err("rejected deletion has a source commit receipt".into())
                            }
                            Err(crate::sync::SyncError::Source(
                                crate::source::SourceError::OperationConflict,
                            )) => conflicting = true,
                            Err(error) => return Err(error.to_string()),
                        }
                    }
                    if record.last_failure == Some(DeleteFailure::OperationConflict) && !conflicting
                    {
                        return Err("deletion conflict receipt missing".into());
                    }
                }
            }
            self.deletion_audit_cursor = Some(record.request.operation_id);
        }
        self.deletion_audit_complete = finished;
        Ok(())
    }

    fn sync_once(&mut self) -> ServiceResult<bool> {
        let mut changed = false;
        let mut failure = None;
        for source in &mut self.sources {
            let generation = self.output_generation.clone();
            match source.sync_batch_before_apply(&mut self.store, || {
                generation.fetch_add(1, Ordering::AcqRel);
            }) {
                Ok(result) => changed |= result.applied_events > 0 || result.restored_records > 0,
                Err(error) => {
                    failure.get_or_insert_with(|| error.to_string());
                }
            }
        }
        if changed {
            self.generation = self.generation.saturating_add(1);
            (self.changed)(self.generation);
        }
        if failure.is_some() {
            self.revoke_outputs();
        }
        failure.map_or(Ok(changed), Err)
    }
}

/// 调用者持有到源记录提交或放弃；超时/取消后的导入不会永久占据本进程 reservation。
pub struct PendingAttachmentImport {
    pub import: AttachmentImport,
    service: Arc<HistoryService>,
}
impl Drop for PendingAttachmentImport {
    fn drop(&mut self) {
        let service = self.service.clone();
        let operation = self.import.operation_id.clone();
        let _ = thread::Builder::new()
            .name("attachment-import-settle".into())
            .spawn(move || {
                let _ = service.finish_attachment_import(operation);
            });
    }
}

pub struct HistoryService {
    output_generation: Arc<AtomicU64>,
    source_writes: Arc<SourceWrites>,
    sender: SyncSender<Job>,
    stopping: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl HistoryService {
    /// root 必须是已完成备份及源表迁移的受管目录；此入口不会创建业务源库。
    pub fn start(
        root: PathBuf,
        profile_id: String,
        changed: impl Fn(u64) + Send + 'static,
    ) -> ServiceResult<Self> {
        Self::start_with_attachment_context(
            root,
            profile_id,
            changed,
            inputia_settings::maintenance::current_user_context().ok(),
        )
    }
    fn start_with_attachment_context(
        root: PathBuf,
        profile_id: String,
        changed: impl Fn(u64) + Send + 'static,
        maintenance_user: Option<inputia_settings::maintenance::UserContext>,
    ) -> ServiceResult<Self> {
        let (sender, receiver) = mpsc::sync_channel::<Job>(32);
        let stopping = Arc::new(AtomicBool::new(false));
        let stop = stopping.clone();
        let output_generation = Arc::new(AtomicU64::new(1));
        let worker_generation = output_generation.clone();
        let source_writes = Arc::new(SourceWrites::default());
        let worker_source_writes = source_writes.clone();
        let thread = thread::Builder::new()
            .name("handy-unified-history".into())
            .spawn(move || {
                let mut worker = (|| {
                    let mut options = std::fs::OpenOptions::new();
                    options.read(true).write(true).create(true).truncate(false);
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::OpenOptionsExt;
                        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
                    }
                    let writer = options
                        .open(root.join("integration-writer.lock"))
                        .map_err(|_| "unable to open integration writer lease".to_owned())?;
                    writer
                        .try_lock()
                        .map_err(|_| "unified history already has a writer".to_owned())?;
                    let mut sources = Vec::new();
                    for (file, table) in [
                        ("history.db", SourceTable::History),
                        ("clipboard.db", SourceTable::Clipboard),
                    ] {
                        let connection = Connection::open_with_flags(
                            root.join(file),
                            OpenFlags::SQLITE_OPEN_READ_WRITE,
                        )
                        .map_err(|_| "source database unavailable".to_owned())?;
                        sources.push(
                            SourcePump::attach(connection, table).map_err(|e| e.to_string())?,
                        );
                    }
                    let mut store =
                        IntegrationStore::open(root.join("integration.db"), &profile_id)
                            .map_err(|e| e.to_string())?;
                    let key = crate::private_key::load_or_create(
                        &root.join("integration-learning.key"),
                        !store.learning_initialized().map_err(|e| e.to_string())?,
                    )
                    .map_err(|_| "learning key unavailable or unsafe".to_owned())?;
                    store.enable_learning(&key).map_err(|e| e.to_string())?;
                    store.initialize_outputs().map_err(|e| e.to_string())?;
                    store
                        .initialize_voice_sessions()
                        .map_err(|e| e.to_string())?;
                    let attachments = AttachmentStore::new(&root).map_err(|e| e.to_string())?;
                    AttachmentStore::initialize(store.attachment_connection())
                        .map_err(|e| e.to_string())?;
                    attachments
                        .recover_imports(store.attachment_connection())
                        .map_err(|e| e.to_string())?;
                    Ok(Worker {
                        attachments,
                        attachment_audit: None,
                        attachment_pause: Some(PauseReason::SourceAuditPending),
                        maintenance_user,
                        next_attachment_maintenance: Instant::now(),
                        output_generation: worker_generation,
                        source_writes: worker_source_writes,
                        _writer_lease: writer,
                        learning_key: key,
                        store,
                        sources,
                        last_error: None,
                        generation: 0,
                        deletion_recovery_cursor: None,
                        next_deletion_recovery: Instant::now(),
                        deletion_audit_cursor: None,
                        deletion_audit_complete: false,
                        changed: Box::new(changed),
                    })
                })();
                while !stop.load(Ordering::Acquire) {
                    if let Ok(state) = &mut worker {
                        // 先恢复删除屏障再接收下一项工作；重启不依赖 UI 重放请求。
                        let recovery = state.recover_deletions().err();
                        state.last_error = recovery.or_else(|| state.sync_once().err());
                        state.maintain_attachments();
                    }
                    match receiver.recv_timeout(Duration::from_millis(100)) {
                        Ok(job) => job(&mut worker),
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
            })
            .map_err(|_| "unable to start history worker".to_owned())?;
        Ok(Self {
            output_generation,
            source_writes,
            sender,
            stopping,
            thread: Some(thread),
        })
    }

    fn call<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Worker) -> ServiceResult<T> + Send + 'static,
    ) -> ServiceResult<T> {
        self.call_metadata(move |worker| {
            worker.require_deletions_settled()?;
            work(worker)
        })
    }

    /// 仅状态、既有回执、取消或恢复入口使用；不得经此方法读取正文或签发许可。
    fn call_metadata<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Worker) -> ServiceResult<T> + Send + 'static,
    ) -> ServiceResult<T> {
        let (sender, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(Box::new(move |worker| {
                let result = match worker {
                    Ok(state) => work(state),
                    Err(error) => Err(error.clone()),
                };
                let _ = sender.send(result);
            }))
            .map_err(|_| "history service is busy or stopped".to_owned())?;
        receiver
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| "history service request timed out".to_owned())?
    }

    /// 导入失败独立于文字提交，调用方应保存 typed 缺附件原因。操作 ID 重试必须保持字节相同。
    pub fn import_attachment(
        &self,
        kind: AttachmentKind,
        operation_id: String,
        bytes: Vec<u8>,
    ) -> Result<AttachmentImport, PauseReason> {
        use sha2::{Digest, Sha256};
        let _guard = self.begin_source_write();
        let digest = format!("{:x}", Sha256::digest(&bytes));
        let size = bytes.len() as u64;
        let (attachments, work) = self
            .call_metadata(move |worker| {
                let work = worker.attachments.prepare_import(
                    worker.store.attachment_connection(),
                    kind,
                    &operation_id,
                    &digest,
                    size,
                );
                Ok((worker.attachments.clone(), work.map_err(|e| e.reason())))
            })
            .map_err(|_| PauseReason::IoFailure)?;
        let published = attachments
            .publish_import(work?, &bytes)
            .map_err(|e| e.reason())?;
        self.call_metadata(move |worker| {
            Ok(worker
                .attachments
                .commit_import(worker.store.attachment_connection(), published)
                .map_err(|e| e.reason()))
        })
        .map_err(|_| PauseReason::IoFailure)?
    }
    pub fn import_attachment_owned(
        self: &Arc<Self>,
        kind: AttachmentKind,
        operation: String,
        bytes: Vec<u8>,
    ) -> Result<PendingAttachmentImport, PauseReason> {
        match self.import_attachment(kind, operation.clone(), bytes) {
            Ok(import) => Ok(PendingAttachmentImport {
                import,
                service: self.clone(),
            }),
            Err(error) => {
                let _ = self.finish_attachment_import(operation);
                Err(error)
            }
        }
    }
    pub fn finish_attachment_import(&self, operation: String) -> ServiceResult<()> {
        self.call_metadata(move |worker| {
            worker
                .attachments
                .finish_import(worker.store.attachment_connection(), &operation)
                .map_err(|e| e.to_string())?;
            worker.attachment_audit = None;
            Ok(())
        })
    }
    pub fn attachment_import_status(
        &self,
        operation: String,
    ) -> ServiceResult<Option<crate::attachment_store::AttachmentImportStatus>> {
        self.call_metadata(move |worker| {
            worker
                .attachments
                .import_status(worker.store.attachment_connection(), &operation)
                .map_err(|e| e.to_string())
        })
    }
    pub fn attachment_deletion_status(
        &self,
        operation: String,
    ) -> ServiceResult<(
        crate::deletion_lifecycle::AttachmentCleanup,
        Option<PauseReason>,
    )> {
        self.call_metadata(move |worker| {
            worker
                .attachments
                .deletion_progress(worker.store.attachment_connection(), &operation)
                .map_err(|e| e.to_string())
        })
    }
    pub fn attachment_health(&self) -> ServiceResult<AttachmentHealth> {
        self.call_metadata(|worker| {
            let mut health = worker
                .attachments
                .health(worker.store.attachment_connection())
                .map_err(|e| e.to_string())?;
            health.paused = health.paused.or(worker.attachment_pause);
            Ok(health)
        })
    }
    pub fn configure_attachment_budget(&self, budget: AttachmentBudget) -> ServiceResult<()> {
        self.call_metadata(move |worker| {
            worker
                .attachments
                .configure(worker.store.attachment_connection(), &budget)
                .map_err(|e| e.to_string())
        })
    }
    pub fn pause_attachment_gc(&self, reason: Option<PauseReason>) -> ServiceResult<()> {
        self.call_metadata(move |worker| {
            worker
                .attachments
                .pause(worker.store.attachment_connection(), reason)
                .map_err(|e| e.to_string())
        })
    }
    pub fn mark_attachment_failure(
        &self,
        table: SourceTable,
        record_id: String,
        reason: PauseReason,
    ) -> ServiceResult<()> {
        self.call_metadata(move |worker| {
            let source = worker
                .sources
                .iter_mut()
                .find(|s| s.source_table() == table)
                .ok_or("attachment source unavailable")?;
            let refs = source.attachment_references().map_err(|e| e.to_string())?;
            let reference = refs
                .iter()
                .find(|r| r.record_id == record_id)
                .ok_or("attachment owner missing")?;
            worker
                .attachments
                .owner_failure(
                    worker.store.attachment_connection(),
                    &reference.store_id,
                    &record_id,
                    reference.revision,
                    Some(reason),
                )
                .map_err(|e| e.to_string())
        })
    }
    /// 按源当前修订获取，租约钉住确切文件身份，不随之后的 owner 修订改变。
    pub fn acquire_source_attachment(
        &self,
        table: SourceTable,
        record_id: String,
        expected_revision: Option<u64>,
        purpose: PinPurpose,
        operation_id: String,
    ) -> ServiceResult<(AttachmentLease, PathBuf, u64)> {
        let acquired = self.call(move |worker| {
            let source = worker
                .sources
                .iter_mut()
                .find(|s| s.source_table() == table)
                .ok_or("attachment source unavailable")?;
            let refs: Vec<_> = source
                .attachment_references()
                .map_err(|e| e.to_string())?
                .into_iter()
                .filter(|r| r.record_id == record_id)
                .collect();
            let first = refs.first().ok_or("attachment owner missing")?;
            let store = first.store_id.clone();
            let revision = first.revision;
            if expected_revision.is_some_and(|expected| expected != revision) {
                return Err("attachment revision changed".into());
            }
            worker
                .attachments
                .reconcile_owner(worker.store.attachment_connection(), &refs)
                .map_err(|e| e.to_string())?;
            let (lease, path) = worker
                .attachments
                .acquire_owner(
                    worker.store.attachment_connection(),
                    &store,
                    &record_id,
                    revision,
                    purpose,
                    &operation_id,
                )
                .map_err(|e| e.to_string())?;
            Ok((lease, path, revision))
        })?;
        if let Err(error) = self.read_attachment(acquired.0.clone()) {
            let _ = self.release_attachment(acquired.0);
            return Err(error);
        }
        Ok(acquired)
    }
    pub fn release_attachment(&self, lease: AttachmentLease) -> ServiceResult<()> {
        self.call_metadata(move |worker| {
            worker
                .attachments
                .release(worker.store.attachment_connection(), &lease)
                .map_err(|e| e.to_string())
        })
    }
    pub fn cancel_attachment_acquire(&self, operation_id: String) -> ServiceResult<()> {
        self.call_metadata(move |worker| {
            worker
                .attachments
                .cancel_acquire(worker.store.attachment_connection(), &operation_id)
                .map_err(|e| e.to_string())
        })
    }
    pub fn read_attachment(&self, lease: AttachmentLease) -> ServiceResult<Vec<u8>> {
        let (attachments, plan) = self.call(move |worker| {
            let plan = worker
                .attachments
                .lease_read_plan(worker.store.attachment_connection(), &lease)
                .map_err(|e| e.to_string())?;
            Ok((worker.attachments.clone(), plan))
        })?;
        attachments.read_plan(plan).map_err(|e| e.to_string())
    }

    pub fn acquire_attachment_maintenance(
        &self,
        operation_id: String,
    ) -> ServiceResult<AttachmentLease> {
        self.call_metadata(move |worker| {
            worker
                .attachments
                .acquire(
                    worker.store.attachment_connection(),
                    None,
                    PinPurpose::Update,
                    &operation_id,
                )
                .map_err(|e| e.to_string())
        })
    }
    /// manager 的单项、清空、保留策略均用同一源 CAS 和耐久删除状态机，隐私过滤不影响删除。
    pub fn delete_source_record(
        &self,
        table: SourceTable,
        record_id: String,
    ) -> ServiceResult<bool> {
        self.call_metadata(move |worker| {
            use sha2::{Digest, Sha256};
            worker.sync_once()?;
            let source = worker
                .sources
                .iter_mut()
                .find(|s| s.source_table() == table)
                .ok_or("deletion source unavailable")?;
            let refs = source.attachment_references().map_err(|e| e.to_string())?;
            let Some(reference) = refs.iter().find(|r| r.record_id == record_id) else {
                let store_id = source.store_id().to_owned();
                if let Some(record) = worker
                    .store
                    .pending_deletion_for_owner(&store_id, &record_id)
                    .map_err(|e| e.to_string())?
                {
                    worker.resume_deletion(record, 100)?;
                    return Ok(true);
                }
                return Ok(false);
            };
            let store_id = source.store_id().to_owned();
            let revision = reference.revision;
            let item_id = crate::store::item_id(&store_id, &record_id);
            let operation_id = format!(
                "manager-delete-{:x}",
                Sha256::digest(format!("{item_id}:{revision}").as_bytes())
            );
            let request = DeleteRequest {
                schema_version: DELETE_SCHEMA_VERSION,
                operation_id,
                item_id,
                store_id,
                logical_name: table.logical_name().into(),
                record_id,
                expected_revision: revision,
            };
            // 捕获所有现存 owner/保留修订引用，随后源事务回执决定真实删除清单。
            let _ = worker
                .attachments
                .reconcile_owner(worker.store.attachment_connection(), &refs);
            let record = worker
                .store
                .prepare_deletion(&request)
                .map_err(|e| e.to_string())?;
            worker.resume_deletion(record, 100)?;
            worker.last_error = None;
            Ok(true)
        })
    }

    /// 同步请求仅供阻塞任务池调用，不能在 GUI 或输入法按键线程调用。
    pub fn query(&self, query: HistoryQuery) -> ServiceResult<Vec<IndexedItem>> {
        self.call(move |worker| {
            if let Some(error) = &worker.last_error {
                return Err(error.clone());
            }
            worker.store.query(&query).map_err(|e| e.to_string())
        })
    }

    pub fn revisions(&self, item_id: String) -> ServiceResult<Vec<ContentRevision>> {
        self.call(move |worker| {
            if let Some(error) = &worker.last_error {
                return Err(error.clone());
            }
            worker.store.revisions(&item_id).map_err(|e| e.to_string())
        })
    }

    pub fn get_item(&self, item_id: String, expected_revision: u64) -> ServiceResult<IndexedItem> {
        self.call(move |worker| {
            if let Some(error) = &worker.last_error {
                return Err(error.clone());
            }
            let item = worker
                .store
                .get(&item_id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "history item no longer exists".to_owned())?;
            if item.revision != expected_revision {
                return Err("history item has a newer revision".into());
            }
            Ok(item)
        })
    }

    /// true 仅确认源记录和统一投影撤销；附件和其他学习域以日志范围为准。
    pub fn delete_item(
        &self,
        item_id: String,
        expected_revision: u64,
        operation_id: String,
    ) -> ServiceResult<bool> {
        self.call_metadata(move |worker| {
            worker.revoke_outputs();
            let existing = worker
                .store
                .deletion_record(&operation_id)
                .map_err(|e| e.to_string())?;
            let record = if let Some(record) = existing {
                if record.request.item_id != item_id
                    || record.request.expected_revision != expected_revision
                {
                    return Err("deletion operation identifier conflict".into());
                }
                record
            } else {
                // 兼容本日志引入前已提交的源回执，同时拒绝其他源重用操作 ID。
                let mut prior_source = None;
                for source in &worker.sources {
                    if source
                        .delete_receipt(&item_id, expected_revision, &operation_id)
                        .map_err(|e| e.to_string())?
                        .is_some()
                    {
                        prior_source = Some((
                            source.store_id().to_owned(),
                            source.source_table().logical_name().to_owned(),
                        ));
                    }
                }
                let (store_id, logical_name, record_id) =
                    if let Some((store_id, logical_name)) = prior_source {
                        let prefix = format!("{}:{}", store_id.len(), store_id);
                        let encoded = item_id
                            .strip_prefix(&prefix)
                            .ok_or("deletion receipt source mismatch")?;
                        let (length, record_id) = encoded
                            .split_once(':')
                            .ok_or("deletion item identity invalid")?;
                        if length.parse::<usize>().ok() != Some(record_id.len())
                            || crate::store::item_id(&store_id, record_id) != item_id
                        {
                            return Err("deletion item identity invalid".into());
                        }
                        (store_id, logical_name, record_id.to_owned())
                    } else {
                        let item = worker
                            .store
                            .get(&item_id)
                            .map_err(|e| e.to_string())?
                            .ok_or("history item no longer exists")?;
                        let source = worker
                            .sources
                            .iter()
                            .find(|source| source.store_id() == item.store_id)
                            .ok_or("history source unavailable")?;
                        (
                            item.store_id,
                            source.source_table().logical_name().to_owned(),
                            item.record_id,
                        )
                    };
                let request = DeleteRequest {
                    schema_version: DELETE_SCHEMA_VERSION,
                    operation_id,
                    item_id,
                    store_id,
                    logical_name,
                    record_id,
                    expected_revision,
                };
                worker
                    .store
                    .prepare_deletion(&request)
                    .map_err(|e| e.to_string())?
            };
            worker.resume_deletion(record, 100)?;
            worker.last_error = None;
            Ok(true)
        })
        .map_err(|error| {
            if error == "history service request timed out" {
                "deletion outcome unknown; retry the same operation_id and arguments".into()
            } else {
                error
            }
        })
    }

    /// 返回可恢复阶段及明确未完成的附件范围，不返回正文或路径。
    pub fn deletion_record(&self, operation_id: String) -> ServiceResult<Option<DeleteRecord>> {
        self.call_metadata(move |worker| {
            worker
                .store
                .deletion_record(&operation_id)
                .map_err(|e| e.to_string())
        })
    }

    pub fn update_item(
        &self,
        item_id: String,
        expected_revision: u64,
        operation_id: String,
        patch: crate::source::HistoryPatch,
    ) -> ServiceResult<u64> {
        self.call(move |worker| {
            worker.revoke_outputs();
            if worker
                .store
                .deletion_record(&operation_id)
                .map_err(|e| e.to_string())?
                .is_some()
            {
                return Err("source operation identifier already belongs to a deletion".into());
            }
            let item = worker
                .store
                .get(&item_id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "history item no longer exists".to_owned())?;
            let source = worker
                .sources
                .iter_mut()
                .find(|source| source.store_id() == item.store_id)
                .ok_or_else(|| "history source unavailable".to_owned())?;
            let result = source
                .update_record(&item.record_id, expected_revision, &operation_id, &patch)
                .map_err(|e| e.to_string())?;
            let revision = result
                .response
                .parse::<u64>()
                .map_err(|_| "invalid source receipt".to_owned())?;
            // 消费到该修订才确认 UI 修改完成；超时重试仍复用相同 operation_id。
            for _ in 0..100 {
                match worker.sync_once() {
                    Ok(_) => {}
                    Err(error) => {
                        worker.last_error = Some(error.clone());
                        return Err(error);
                    }
                }
                if worker
                    .store
                    .get(&item_id)
                    .map_err(|e| e.to_string())?
                    .is_some_and(|current| current.revision >= revision)
                {
                    worker.last_error = None;
                    return Ok(revision);
                }
            }
            Err("source updated but projection is still pending".into())
        })
    }

    pub fn list_terms(&self, limit: u32, offset: u64) -> ServiceResult<Vec<LearnedTermView>> {
        self.call(move |worker| {
            worker
                .store
                .list_terms(limit, offset)
                .map_err(|e| e.to_string())
        })
    }

    /// 逐词本地同意入口；超时后以同一请求重试，仅返回历史处理回执。
    pub fn confirm_history_term(
        &self,
        request: HistoryTermConfirmation,
        guard: impl Fn() -> bool + Send + 'static,
    ) -> ServiceResult<ApplyContribution> {
        self.call(move |worker| {
            if worker
                .store
                .history_term_confirmation_received(&worker.learning_key, &request)
                .map_err(|error| error.to_string())?
            {
                return Ok(ApplyContribution::Replay);
            }
            worker.sync_once()?;
            worker.revoke_outputs();
            let result = worker
                .store
                .confirm_history_term(&worker.learning_key, &request, guard)
                .map_err(|error| error.to_string())?;
            if result == ApplyContribution::Applied {
                worker.generation = worker.generation.saturating_add(1);
                (worker.changed)(worker.generation);
            }
            Ok(result)
        })
    }

    /// 已确认内容的贡献仍由规范库验证源修订、策略和重放；不接收普通输入全文。
    pub fn contribute_term(
        &self,
        input: ContributionInput,
        policy: PrivacyPolicy,
        context: PrivacyContext,
    ) -> ServiceResult<ApplyContribution> {
        self.call(move |worker| {
            worker.revoke_outputs();
            worker.sync_once()?;
            let result = worker
                .store
                .contribute_term(&worker.learning_key, &input, &policy, context)
                .map_err(|error| error.to_string())?;
            if result == ApplyContribution::Applied {
                worker.generation = worker.generation.saturating_add(1);
                (worker.changed)(worker.generation);
            }
            Ok(result)
        })
    }

    /// 回执返回本操作提交时的 epoch，而非当前策略；超时必须复用 operation_id。
    pub fn forget_term(
        &self,
        operation_id: String,
        term: String,
        expected_epoch: u64,
    ) -> ServiceResult<u64> {
        let operation_id = inputia_core::integration::events::Identifier::parse(operation_id)
            .map_err(str::to_owned)?;
        self.call(move |worker| {
            worker.revoke_outputs();
            worker.sync_once()?;
            let epoch = worker
                .store
                .forget_term_with_receipt(
                    &worker.learning_key,
                    &operation_id,
                    &term,
                    expected_epoch,
                )
                .map_err(|error| error.to_string())?;
            worker.generation = worker.generation.saturating_add(1);
            (worker.changed)(worker.generation);
            Ok(epoch)
        })
        .map_err(|error| {
            if error == "history service request timed out" {
                "forget outcome unknown; retry with the same operation_id and arguments".into()
            } else {
                error
            }
        })
    }

    pub fn policy_epoch(&self) -> ServiceResult<u64> {
        self.call_metadata(|worker| worker.store.policy_epoch().map_err(|e| e.to_string()))
    }

    pub fn prepare_voice_request(
        &self,
        request: crate::voice_protocol::VoiceRequest,
        client: String,
        server: String,
        applied_epoch: Option<u64>,
    ) -> ServiceResult<crate::voice_ledger::SessionRecord> {
        self.call_metadata(move |worker| {
            if request.strict_start_identity().is_some() {
                worker.require_deletions_settled()?;
            }
            worker
                .store
                .prepare_voice_request(&request, &client, &server, applied_epoch)
                .map_err(|e| e.to_string())
        })
    }

    pub fn bind_voice_peer(&self, client: String, audit: [u8; 32]) -> ServiceResult<()> {
        self.call_metadata(move |worker| {
            worker
                .store
                .bind_voice_peer(&client, &audit)
                .map_err(|error| error.to_string())
        })
    }

    pub fn voice_terms_version(&self) -> ServiceResult<crate::voice_protocol::VoiceTermsVersion> {
        self.call_metadata(|worker| {
            worker
                .store
                .voice_terms_version()
                .map_err(|error| error.to_string())
        })
    }

    pub fn voice_cancellation_requested(&self, session_id: String) -> ServiceResult<bool> {
        self.call_metadata(move |worker| {
            worker
                .store
                .voice_cancellation_requested(&session_id)
                .map_err(|error| error.to_string())
        })
    }
    pub fn claim_voice_request(
        &self,
        request: crate::voice_protocol::VoiceRequest,
        client: String,
        server: String,
        applied_epoch: Option<u64>,
    ) -> ServiceResult<bool> {
        self.call_metadata(move |worker| {
            if request.strict_start_identity().is_some() {
                worker.require_deletions_settled()?;
            }
            worker
                .store
                .claim_voice_request(&request, &client, &server, applied_epoch)
                .map_err(|e| e.to_string())
        })
    }
    pub fn project_voice_session(
        &self,
        client: String,
        server: String,
        view: crate::voice_protocol::VoiceSessionView,
    ) -> ServiceResult<bool> {
        self.call_metadata(move |worker| {
            worker
                .store
                .project_voice_session(&client, &server, &view)
                .map_err(|e| e.to_string())
        })
    }
    pub fn voice_session(
        &self,
        session_id: String,
    ) -> ServiceResult<Option<crate::voice_ledger::SessionRecord>> {
        self.call_metadata(move |worker| {
            worker
                .store
                .voice_session(&session_id)
                .map_err(|e| e.to_string())
        })
    }
    pub fn prepare_output(
        &self,
        intent: crate::output_ledger::OutputIntent,
    ) -> ServiceResult<crate::output_ledger::OutputRecord> {
        self.call(move |worker| {
            worker
                .store
                .prepare_output(&intent)
                .map_err(|e| e.to_string())
        })
    }

    /// 按本次保存的源记录定位；不通过最新记录或全文搜索猜测身份。
    pub fn prepare_saved_voice_result(
        &self,
        start: crate::voice_protocol::VoiceRequest,
        history_id: i64,
        expected_text: String,
    ) -> ServiceResult<crate::output_ledger::OutputRecord> {
        self.call(move |worker| {
            if history_id <= 0 || expected_text.is_empty() {
                return Err("invalid saved voice result".into());
            }
            worker.sync_once()?;
            let source = worker
                .sources
                .iter()
                .find(|source| source.source_table() == SourceTable::History)
                .ok_or("history source unavailable")?;
            let id = crate::store::item_id(source.store_id(), &history_id.to_string());
            let item = worker
                .store
                .get(&id)
                .map_err(|error| error.to_string())?
                .ok_or("saved voice result is not projected")?;
            if item.snapshot.source_kind != crate::store::SourceKind::Voice
                || item.snapshot.text.as_deref() != Some(expected_text.as_str())
            {
                return Err("saved voice result changed before output preparation".into());
            }
            let session = worker
                .store
                .voice_session(&start.session_id)
                .map_err(|error| error.to_string())?
                .ok_or("voice session missing")?;
            if session.start != start {
                return Err("voice start identity changed".into());
            }
            worker
                .store
                .prepare_voice_result(
                    &start.session_id,
                    &start.client_instance,
                    &start.server_instance,
                    &id,
                    item.revision,
                )
                .map_err(|error| error.to_string())
        })
    }

    /// Legacy 转写也按本次源记录取得稳定输出身份；不得伪造 Host 会话。
    /// 相同源记录只准备一次自动输出。目标、动作或修订不同的重放会被账本拒绝。
    pub fn prepare_saved_platform_result(
        &self,
        history_id: i64,
        expected_text: String,
        target_id: Option<String>,
        expected_policy_epoch: Option<u64>,
        action: crate::output_ledger::OutputAction,
    ) -> ServiceResult<crate::output_ledger::OutputRecord> {
        self.call(move |worker| {
            use crate::output_ledger::{OutputAction, OutputIntent, OutputOutcome, OutputOwner};
            use sha2::{Digest, Sha256};
            if history_id <= 0
                || expected_text.is_empty()
                || !matches!(
                    action,
                    OutputAction::InsertText | OutputAction::CopyPlainText
                )
            {
                return Err("invalid saved platform result".into());
            }
            worker.sync_once()?;
            let source = worker
                .sources
                .iter()
                .find(|source| source.source_table() == SourceTable::History)
                .ok_or("history source unavailable")?;
            let id = crate::store::item_id(source.store_id(), &history_id.to_string());
            let item = worker
                .store
                .get(&id)
                .map_err(|error| error.to_string())?
                .ok_or("saved platform result is not projected")?;
            if item.snapshot.source_kind != crate::store::SourceKind::Voice
                || item.snapshot.text.as_deref() != Some(expected_text.as_str())
            {
                return Err("saved platform result changed before output preparation".into());
            }
            let policy_epoch = worker
                .store
                .policy_epoch()
                .map_err(|error| error.to_string())?;
            let intent = OutputIntent {
                operation_id: format!("platform-{:x}", Sha256::digest(id.as_bytes())),
                item_id: id,
                revision: item.revision,
                target_id,
                owner: OutputOwner::Platform,
                policy_epoch,
                action,
            };
            let record = worker
                .store
                .prepare_output(&intent)
                .map_err(|error| error.to_string())?;
            if record.state == crate::output_ledger::OutputState::Prepared
                && (expected_policy_epoch != Some(policy_epoch)
                    || (action == OutputAction::InsertText && intent.target_id.is_none()))
            {
                return worker
                    .store
                    .finish_output(&intent, OutputOutcome::PendingTarget)
                    .map_err(|error| error.to_string());
            }
            Ok(record)
        })
    }

    /// 只用于本次转写已保存的源记录；有待消费的源变更先完成投影再准备。
    pub fn prepare_voice_result(
        &self,
        session_id: String,
        client: String,
        server: String,
        item_id: String,
        revision: u64,
    ) -> ServiceResult<crate::output_ledger::OutputRecord> {
        self.call(move |worker| {
            worker.sync_once()?;
            worker
                .store
                .prepare_voice_result(&session_id, &client, &server, &item_id, revision)
                .map_err(|error| error.to_string())
        })
    }

    pub fn voice_result(
        &self,
        session_id: String,
    ) -> ServiceResult<Option<crate::output_ledger::OutputRecord>> {
        self.call_metadata(move |worker| {
            worker
                .store
                .voice_result(&session_id)
                .map_err(|error| error.to_string())
        })
    }
    pub fn cancel_voice_result(
        &self,
        session_id: String,
        client: String,
        server: String,
    ) -> ServiceResult<bool> {
        self.call_metadata(move |worker| {
            worker.revoke_outputs();
            worker
                .store
                .cancel_voice_result(&session_id, &client, &server)
                .map_err(|error| error.to_string())
        })
    }

    pub fn claim_output(&self, intent: crate::output_ledger::OutputIntent) -> ServiceResult<bool> {
        self.call(move |worker| {
            worker
                .store
                .claim_output(&intent)
                .map_err(|e| e.to_string())
        })
    }

    /// 取得执行所有权与短期许可在同一后台请求内完成。许可不得跨进程持久化。
    pub fn claim_output_with_permit(
        &self,
        intent: crate::output_ledger::OutputIntent,
    ) -> ServiceResult<Option<OutputPermit>> {
        self.call(move |worker| {
            // 外部 manager 的源事务与投影分别持连接；先验证完整同步屏障，
            // 不能在“已撤销旧许可、尚未投影”的窗口签发另一个旧内容许可。
            let source_generation = worker.source_writes.generation.load(Ordering::Acquire);
            if worker.source_writes.active.load(Ordering::Acquire) != 0 {
                return Err("source mutation is in progress".into());
            }
            let mut caught_up = false;
            for _ in 0..100 {
                if !worker.sync_once()? {
                    caught_up = true;
                    break;
                }
            }
            let expected_generation = worker.output_generation.load(Ordering::Acquire);
            if !caught_up
                || worker.source_writes.active.load(Ordering::Acquire) != 0
                || worker.source_writes.generation.load(Ordering::Acquire) != source_generation
            {
                return Err("source changed during output authorization".into());
            }
            if worker.last_error.is_some() {
                return Err("history synchronization unavailable".into());
            }
            if !worker
                .store
                .claim_output(&intent)
                .map_err(|e| e.to_string())?
            {
                return Ok(None);
            }
            Ok(Some(OutputPermit {
                generation: worker.output_generation.clone(),
                expected: expected_generation,
                expires: Instant::now() + Duration::from_secs(2),
            }))
        })
    }

    /// 必须在任何源正文/元数据/附件变更之前持有此守卫，直到源事务提交。
    pub fn begin_source_write(&self) -> SourceWriteGuard {
        let attachment = self.source_writes.gate.enter();
        self.source_writes.active.fetch_add(1, Ordering::AcqRel);
        self.source_writes.generation.fetch_add(1, Ordering::AcqRel);
        self.output_generation.fetch_add(1, Ordering::AcqRel);
        SourceWriteGuard {
            _attachment: attachment,
            source: self.source_writes.clone(),
        }
    }
    pub fn finish_output(
        &self,
        intent: crate::output_ledger::OutputIntent,
        outcome: crate::output_ledger::OutputOutcome,
    ) -> ServiceResult<crate::output_ledger::OutputRecord> {
        self.call_metadata(move |worker| {
            worker
                .store
                .finish_output(&intent, outcome)
                .map_err(|e| e.to_string())
        })
    }
    pub fn output_record(
        &self,
        id: String,
    ) -> ServiceResult<Option<crate::output_ledger::OutputRecord>> {
        self.call_metadata(move |worker| worker.store.output_record(&id).map_err(|e| e.to_string()))
    }
    /// 启动与显式刷新均只读持久结果，不用重放输出请求恢复 UI。
    pub fn unresolved_output_notices(
        &self,
        cursor: Option<String>,
        limit: u32,
    ) -> ServiceResult<crate::output_ledger::OutputNoticePage> {
        self.call_metadata(move |worker| {
            worker
                .store
                .unresolved_output_notices(cursor.as_deref(), limit)
                .map_err(|error| error.to_string())
        })
    }

    pub fn acknowledge_output_notice(
        &self,
        operation_id: String,
        expected_state: crate::output_ledger::OutputState,
    ) -> ServiceResult<()> {
        self.call_metadata(move |worker| {
            worker
                .store
                .acknowledge_output_notice(&operation_id, expected_state)
                .map_err(|error| error.to_string())
        })
    }

    pub fn indexed_item(&self, id: String) -> ServiceResult<Option<IndexedItem>> {
        self.call(move |worker| {
            worker.sync_once()?;
            worker.store.get(&id).map_err(|e| e.to_string())
        })
    }

    /// 模型调用方提供已捕获的真实目标策略；此方法不自行猜测前台来源。
    pub fn session_hotwords(
        &self,
        policy: PrivacyPolicy,
        context: PrivacyContext,
        explicit: Vec<String>,
        budget: HotwordBudget,
    ) -> ServiceResult<TermSnapshot> {
        self.call(move |worker| {
            worker
                .store
                .term_snapshot(&worker.learning_key, &policy, context, &explicit, budget)
                .map_err(|e| e.to_string())
        })
    }

    pub fn term_snapshot_is_current(&self, snapshot: TermSnapshot) -> ServiceResult<bool> {
        self.call(move |worker| {
            worker
                .store
                .term_snapshot_is_current(&snapshot)
                .map_err(|e| e.to_string())
        })
    }

    /// 测试和显式刷新使用的有界屏障；达到上限报告未追平，不冒充同步完成。
    pub fn synchronize(&self) -> ServiceResult<u64> {
        self.call(|worker| {
            for _ in 0..100 {
                match worker.sync_once() {
                    Ok(false) => {
                        worker.last_error = None;
                        return Ok(worker.generation);
                    }
                    Ok(true) => {}
                    Err(error) => {
                        worker.last_error = Some(error.clone());
                        return Err(error);
                    }
                }
            }
            Err("history backlog requires another synchronization pass".into())
        })
    }
}

impl Drop for HistoryService {
    fn drop(&mut self) {
        self.output_generation.fetch_add(1, Ordering::AcqRel);
        self.stopping.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod output_permit_tests {
    use super::*;

    #[test]
    fn confirmation_receipt_survives_source_sync_failure() {
        use crate::store::{
            ContentType, ItemSnapshot, SourceChange, SourceKind, SourceOperation, SourceTrust,
        };
        use inputia_core::integration::events::Identifier;
        let root = tempfile::tempdir().unwrap();
        Connection::open(root.path().join("history.db")).unwrap().execute_batch("CREATE TABLE transcription_history(id INTEGER PRIMARY KEY,file_name TEXT,timestamp INTEGER,saved INTEGER,title TEXT,transcription_text TEXT,post_processed_text TEXT);").unwrap();
        let clipboard = Connection::open(root.path().join("clipboard.db")).unwrap();
        clipboard.execute_batch("CREATE TABLE clipboard_history(id INTEGER PRIMARY KEY,content_type TEXT,full_text TEXT,title TEXT,is_favorite INTEGER,is_pinned INTEGER,created_at INTEGER,image_path TEXT,source_app TEXT);").unwrap();
        let service = HistoryService::start(root.path().into(), "fixture".into(), |_| {}).unwrap();
        service
            .call(|worker| {
                worker
                    .store
                    .register_source("fixture", "fixture")
                    .map_err(|e| e.to_string())?;
                worker
                    .store
                    .apply_change(&SourceChange {
                        store_id: "fixture".into(),
                        seq: 1,
                        event_id: "fixture-1".into(),
                        record_id: "1".into(),
                        revision: 1,
                        operation: SourceOperation::Upsert,
                        policy_epoch: 1,
                        payload: Some(ItemSnapshot {
                            source_kind: SourceKind::Voice,
                            content_type: ContentType::Text,
                            text: Some("Inputia".into()),
                            title: None,
                            starred: false,
                            pinned: false,
                            created_at_ms: 1,
                            asset_ref: None,
                            source_app: None,
                            source_trust: SourceTrust::Verified,
                        }),
                    })
                    .map_err(|e| e.to_string())?;
                Ok(())
            })
            .unwrap();
        let request = HistoryTermConfirmation {
            operation_id: Identifier::parse("confirm-sync").unwrap(),
            item_id: crate::store::item_id("fixture", "1"),
            expected_revision: 1,
            term: "Inputia".into(),
        };
        assert_eq!(
            service
                .confirm_history_term(request.clone(), || true)
                .unwrap(),
            ApplyContribution::Applied
        );
        service
            .forget_term("forget-sync".into(), "Inputia".into(), 1)
            .unwrap();
        clipboard
            .execute_batch("DROP TABLE unified_source_outbox;")
            .unwrap();
        assert!(service.synchronize().is_err());
        assert_eq!(
            service.confirm_history_term(request, || false).unwrap(),
            ApplyContribution::Replay
        );
        assert!(service.list_terms(100, 0).unwrap().is_empty());
    }

    #[test]
    fn timed_out_forget_retries_original_receipt_after_restart() {
        let root = tempfile::tempdir().unwrap();
        Connection::open(root.path().join("history.db")).unwrap().execute_batch("CREATE TABLE transcription_history(id INTEGER PRIMARY KEY,file_name TEXT,timestamp INTEGER,saved INTEGER,title TEXT,transcription_text TEXT,post_processed_text TEXT);").unwrap();
        Connection::open(root.path().join("clipboard.db")).unwrap().execute_batch("CREATE TABLE clipboard_history(id INTEGER PRIMARY KEY,content_type TEXT,full_text TEXT,title TEXT,is_favorite INTEGER,is_pinned INTEGER,created_at INTEGER,image_path TEXT,source_app TEXT);").unwrap();
        let (release_tx, release_rx) = mpsc::channel();
        let once = AtomicBool::new(true);
        let service = HistoryService::start(root.path().into(), "fixture".into(), move |_| {
            if once.swap(false, Ordering::AcqRel) {
                release_rx.recv().unwrap();
            }
        })
        .unwrap();
        assert_eq!(service.policy_epoch().unwrap(), 1);
        let result = service.forget_term("forget-timeout".into(), "Inputia".into(), 1);
        release_tx.send(()).unwrap();
        assert!(result.unwrap_err().contains("outcome unknown"));
        assert_eq!(
            service
                .forget_term("forget-timeout".into(), "Inputia".into(), 1)
                .unwrap(),
            2
        );
        assert_eq!(service.policy_epoch().unwrap(), 2);
        drop(service);
        let restarted =
            HistoryService::start(root.path().into(), "fixture".into(), |_| {}).unwrap();
        assert_eq!(
            restarted
                .forget_term("forget-timeout".into(), "Inputia".into(), 1)
                .unwrap(),
            2
        );
        assert_eq!(restarted.policy_epoch().unwrap(), 2);
    }

    #[test]
    fn confirmed_term_queue_replays_once_and_forget_revokes_snapshot() {
        use crate::store::{
            ContentType, ItemSnapshot, SourceChange, SourceKind, SourceOperation,
            SourceTrust as ItemTrust,
        };
        use inputia_core::integration::{
            events::{Identifier, SourceRecord},
            privacy::{HistoryMode, SourceTrust},
            terms::TermEvidence,
        };
        let root = tempfile::tempdir().unwrap();
        Connection::open(root.path().join("history.db")).unwrap().execute_batch("CREATE TABLE transcription_history(id INTEGER PRIMARY KEY,file_name TEXT,timestamp INTEGER,saved INTEGER,title TEXT,transcription_text TEXT,post_processed_text TEXT);").unwrap();
        Connection::open(root.path().join("clipboard.db")).unwrap().execute_batch("CREATE TABLE clipboard_history(id INTEGER PRIMARY KEY,content_type TEXT,full_text TEXT,title TEXT,is_favorite INTEGER,is_pinned INTEGER,created_at INTEGER,image_path TEXT,source_app TEXT);").unwrap();
        let service = HistoryService::start(root.path().into(), "fixture".into(), |_| {}).unwrap();
        service
            .call(|worker| {
                worker
                    .store
                    .register_source("fixture", "fixture")
                    .map_err(|e| e.to_string())?;
                worker
                    .store
                    .apply_change(&SourceChange {
                        store_id: "fixture".into(),
                        seq: 1,
                        event_id: "fixture-1".into(),
                        record_id: "1".into(),
                        revision: 1,
                        operation: SourceOperation::Upsert,
                        policy_epoch: 1,
                        payload: Some(ItemSnapshot {
                            source_kind: SourceKind::Voice,
                            content_type: ContentType::Text,
                            text: Some("Inputia".into()),
                            title: None,
                            starred: false,
                            pinned: false,
                            created_at_ms: 1,
                            asset_ref: None,
                            source_app: None,
                            source_trust: ItemTrust::Verified,
                        }),
                    })
                    .map_err(|e| e.to_string())?;
                Ok(())
            })
            .unwrap();
        let policy = PrivacyPolicy {
            epoch: 1,
            history_enabled: true,
            history_mode: HistoryMode::Normal,
            learning_enabled: true,
            remote_learning_terms_enabled: false,
        };
        let context = PrivacyContext {
            source_trust: SourceTrust::Verified,
            source_sensitive: false,
            target_known: true,
            target_sensitive: false,
            secure_input: false,
            transient_or_concealed: false,
        };
        let contribution = || ContributionInput {
            contribution_id: Identifier::parse("confirmed-1").unwrap(),
            source: SourceRecord {
                store_id: Identifier::parse("fixture").unwrap(),
                record_id: Identifier::parse("1").unwrap(),
            },
            source_revision: 1,
            policy_epoch: 1,
            term: "Inputia".into(),
            evidence: TermEvidence::ConfirmedCorrection,
            explicit_relearn: false,
        };
        assert_eq!(
            service
                .contribute_term(contribution(), policy.clone(), context)
                .unwrap(),
            ApplyContribution::Applied
        );
        assert_eq!(
            service
                .contribute_term(contribution(), policy.clone(), context)
                .unwrap(),
            ApplyContribution::Replay
        );
        assert_eq!(service.list_terms(10, 0).unwrap()[0].contributions, 1);
        let snapshot = service
            .session_hotwords(policy.clone(), context, vec![], HotwordBudget::default())
            .unwrap();
        assert!(service.term_snapshot_is_current(snapshot.clone()).unwrap());
        assert_eq!(
            service
                .forget_term("forget-1".into(), "Inputia".into(), 1)
                .unwrap(),
            2
        );
        assert!(!service.term_snapshot_is_current(snapshot).unwrap());
        assert!(service.list_terms(10, 0).unwrap().is_empty());
        assert_eq!(
            service
                .forget_term("forget-1".into(), "Inputia".into(), 1)
                .unwrap(),
            2
        );
        assert!(service
            .forget_term("forget-1".into(), "Other".into(), 1)
            .is_err());
        assert!(service
            .forget_term("forget-2".into(), "Inputia".into(), 1)
            .is_err());
        assert!(service
            .contribute_term(contribution(), policy, context)
            .is_err());
        assert_eq!(service.policy_epoch().unwrap(), 2);
    }

    #[test]
    fn delayed_voice_claim_times_out_without_granting_caller_execution_or_reclaim() {
        use crate::voice_protocol::*;
        let root = tempfile::tempdir().unwrap();
        let history = Connection::open(root.path().join("history.db")).unwrap();
        history.execute_batch("CREATE TABLE transcription_history(id INTEGER PRIMARY KEY,file_name TEXT,timestamp INTEGER,saved INTEGER,title TEXT,transcription_text TEXT,post_processed_text TEXT);").unwrap();
        Connection::open(root.path().join("clipboard.db")).unwrap().execute_batch("CREATE TABLE clipboard_history(id INTEGER PRIMARY KEY,content_type TEXT,full_text TEXT,title TEXT,is_favorite INTEGER,is_pinned INTEGER,created_at INTEGER,image_path TEXT,source_app TEXT);").unwrap();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let once = AtomicBool::new(true);
        let service = HistoryService::start(root.path().into(), "fixture".into(), move |_| {
            if once.swap(false, Ordering::AcqRel) {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            }
        })
        .unwrap();
        let request = VoiceRequest {
            request_id: "start".into(),
            session_id: "voice".into(),
            client_instance: "host".into(),
            server_instance: "server".into(),
            policy_epoch: 1,
            command: VoiceCommand::Start {
                target: HostTargetToken {
                    target_id: "target".into(),
                    host_instance: "host".into(),
                    controller_id: "controller".into(),
                    activation_generation: 1,
                    field_id: None,
                    selection_generation: 0,
                    composition_generation: 0,
                    source_app: None,
                },
                post_process: false,
                terms: VoiceTermsVersion {
                    policy_epoch: 1,
                    learning_generation: 0,
                },
            },
        };
        service
            .prepare_voice_request(request.clone(), "host".into(), "server".into(), Some(1))
            .unwrap();
        history
            .execute_batch(
                "INSERT INTO transcription_history(id,file_name,timestamp,saved,title,transcription_text,post_processed_text) VALUES(1,NULL,1,0,NULL,'synthetic',NULL);",
            )
            .unwrap();
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let result =
            service.claim_voice_request(request.clone(), "host".into(), "server".into(), Some(1));
        assert!(result.is_err(), "超时不能返回执行资格");
        release_tx.send(()).unwrap();
        // 队列随后可能提交已消费claim；重试只能查询事实，不再次启动。
        assert!(!service
            .claim_voice_request(request, "host".into(), "server".into(), Some(1))
            .unwrap());
        assert_eq!(
            service
                .voice_session("voice".into())
                .unwrap()
                .unwrap()
                .view
                .phase,
            VoicePhase::Preparing
        );
    }

    #[test]
    fn stale_and_expired_permits_fail_without_a_service_call() {
        let generation = Arc::new(AtomicU64::new(1));
        let mut permit = OutputPermit {
            generation: generation.clone(),
            expected: 1,
            expires: Instant::now() + Duration::from_secs(2),
        };
        assert!(permit.check().is_ok());
        generation.fetch_add(1, Ordering::AcqRel);
        assert!(permit.check().is_err());
        permit.expected = 2;
        assert!(permit.check().is_ok());
        permit.expires = Instant::now();
        assert!(permit.check().is_err());
    }
}

#[cfg(all(test, unix))]
mod attachment_pipeline_tests {
    use super::*;
    fn wav() -> Vec<u8> {
        let mut output = std::io::Cursor::new(Vec::new());
        {
            let mut writer = hound::WavWriter::new(
                &mut output,
                hound::WavSpec {
                    channels: 1,
                    sample_rate: 16000,
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                },
            )
            .unwrap();
            writer.write_sample(123i16).unwrap();
            writer.finalize().unwrap();
        }
        output.into_inner()
    }
    fn fixture() -> (tempfile::TempDir, Connection, Arc<HistoryService>) {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().canonicalize().unwrap();
        let conn = Connection::open(home.join("history.db")).unwrap();
        conn.execute_batch("CREATE TABLE transcription_history(id INTEGER PRIMARY KEY AUTOINCREMENT,file_name TEXT,timestamp INTEGER,saved INTEGER,title TEXT,transcription_text TEXT,post_processed_text TEXT);").unwrap();
        Connection::open(home.join("clipboard.db")).unwrap().execute_batch("CREATE TABLE clipboard_history(id INTEGER PRIMARY KEY AUTOINCREMENT,content_type TEXT,full_text TEXT,title TEXT,is_favorite INTEGER,is_pinned INTEGER,created_at INTEGER,image_path TEXT,source_app TEXT);").unwrap();
        let service = Arc::new(
            HistoryService::start_with_attachment_context(
                home.clone(),
                "fixture".into(),
                |_| {},
                Some(inputia_settings::maintenance::UserContext {
                    home,
                    uid: unsafe { libc::geteuid() },
                }),
            )
            .unwrap(),
        );
        service.synchronize().unwrap();
        (root, conn, service)
    }
    fn maintain(service: &HistoryService) {
        service
            .call_metadata(|worker| {
                worker.next_attachment_maintenance = Instant::now();
                worker.maintain_attachments();
                Ok(())
            })
            .unwrap();
    }
    #[test]
    fn real_source_commit_play_delete_release_and_gc_form_one_pipeline() {
        let (root, conn, service) = fixture();
        let pending = service
            .import_attachment_owned(AttachmentKind::Recording, "recording".into(), wav())
            .unwrap();
        let path = root
            .path()
            .join("recordings")
            .join(&pending.import.file_name);
        {
            let _guard = service.begin_source_write();
            conn.execute(
                "INSERT INTO transcription_history(id,file_name,timestamp,saved,title,transcription_text,post_processed_text) VALUES(1,?1,1,0,'title','saved text',NULL)",
                [&pending.import.file_name],
            )
            .unwrap();
        }
        service
            .finish_attachment_import("recording".into())
            .unwrap();
        drop(pending);
        service.synchronize().unwrap();
        maintain(&service);
        let (lease, _, revision) = service
            .acquire_source_attachment(
                SourceTable::History,
                "1".into(),
                None,
                PinPurpose::Active,
                "play".into(),
            )
            .unwrap();
        assert_eq!(service.read_attachment(lease.clone()).unwrap(), wav());
        assert_eq!(revision, 1);
        service
            .delete_source_record(SourceTable::History, "1".into())
            .unwrap();
        maintain(&service);
        assert!(path.exists());
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM transcription_history", [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            0
        );
        service.release_attachment(lease.clone()).unwrap();
        service.release_attachment(lease).unwrap();
        maintain(&service);
        assert!(!path.exists());
        assert_eq!(service.attachment_health().unwrap().pending_gc, 0);
    }
    #[test]
    fn source_revision_change_does_not_retarget_existing_audio_lease() {
        let (_root, conn, service) = fixture();
        let pending = service
            .import_attachment_owned(AttachmentKind::Recording, "one".into(), wav())
            .unwrap();
        {
            let _guard = service.begin_source_write();
            conn.execute(
                "INSERT INTO transcription_history(id,file_name,timestamp,saved,title,transcription_text,post_processed_text) VALUES(1,?1,1,0,'title','one',NULL)",
                [&pending.import.file_name],
            )
            .unwrap();
        }
        let (lease, _, revision) = service
            .acquire_source_attachment(
                SourceTable::History,
                "1".into(),
                Some(1),
                PinPurpose::Active,
                "audio".into(),
            )
            .unwrap();
        {
            let _guard = service.begin_source_write();
            conn.execute(
                "UPDATE transcription_history SET transcription_text='two' WHERE id=1",
                [],
            )
            .unwrap();
        }
        assert_eq!(revision, 1);
        assert!(service
            .acquire_source_attachment(
                SourceTable::History,
                "1".into(),
                Some(1),
                PinPurpose::Active,
                "stale".into()
            )
            .is_err());
        assert_eq!(service.read_attachment(lease.clone()).unwrap(), wav());
        service.release_attachment(lease).unwrap();
    }
    #[test]
    fn cancelled_source_save_and_missing_wav_have_visible_recoverable_outcomes() {
        let (root, conn, service) = fixture();
        let pending = service
            .import_attachment_owned(AttachmentKind::Recording, "cancelled".into(), wav())
            .unwrap();
        let path = root
            .path()
            .join("recordings")
            .join(&pending.import.file_name);
        service
            .finish_attachment_import("cancelled".into())
            .unwrap();
        drop(pending);
        maintain(&service);
        assert!(!path.exists());
        {
            let _guard = service.begin_source_write();
            conn.execute("INSERT INTO transcription_history(id,file_name,timestamp,saved,title,transcription_text,post_processed_text) VALUES(1,'',1,0,'title','successful recognition',NULL)",[]).unwrap();
        }
        service
            .mark_attachment_failure(
                SourceTable::History,
                "1".into(),
                PauseReason::InsufficientSpace,
            )
            .unwrap();
        service.synchronize().unwrap();
        assert_eq!(
            service.query(HistoryQuery::default()).unwrap()[0]
                .snapshot
                .text
                .as_deref(),
            Some("successful recognition")
        );
        assert!(service
            .acquire_source_attachment(
                SourceTable::History,
                "1".into(),
                None,
                PinPurpose::Active,
                "missing".into()
            )
            .is_err());
        let reason = service
            .call_metadata(|worker| {
                let store = worker.sources[0].store_id().to_owned();
                worker
                    .attachments
                    .owner_status(worker.store.attachment_connection(), &store, "1")
                    .map_err(|e| e.to_string())
            })
            .unwrap();
        assert_eq!(reason, Some(PauseReason::InsufficientSpace));
    }
    #[test]
    fn source_write_guard_allows_cleanup_callback_without_gc_deadlock() {
        let (_root, conn, service) = fixture();
        let _guard = service.begin_source_write();
        conn.execute(
            "INSERT INTO transcription_history(id,file_name,timestamp,saved,title,transcription_text,post_processed_text) VALUES(1,'',1,0,'title','text',NULL)",
            [],
        )
        .unwrap();
        assert!(service
            .delete_source_record(SourceTable::History, "1".into())
            .unwrap());
        assert_eq!(service.attachment_health().unwrap().pending_gc, 0);
    }
    #[test]
    fn clipboard_png_uses_same_owner_lease_and_durable_gc_path() {
        let (root, _history, service) = fixture();
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[1, 2, 3, 255]).unwrap();
        }
        let pending = service
            .import_attachment_owned(AttachmentKind::Image, "image".into(), png.clone())
            .unwrap();
        let path = root
            .path()
            .join("clipboard_images")
            .join(&pending.import.file_name);
        let clipboard = Connection::open(root.path().join("clipboard.db")).unwrap();
        {
            let _guard = service.begin_source_write();
            clipboard.execute("INSERT INTO clipboard_history(id,content_type,full_text,title,is_favorite,is_pinned,created_at,image_path,source_app) VALUES(1,'image',NULL,'pixel',0,0,1,?1,NULL)",[&pending.import.file_name]).unwrap();
        }
        service.finish_attachment_import("image".into()).unwrap();
        drop(pending);
        let (lease, _, _) = service
            .acquire_source_attachment(
                SourceTable::Clipboard,
                "1".into(),
                None,
                PinPurpose::Active,
                "image-read".into(),
            )
            .unwrap();
        assert_eq!(service.read_attachment(lease.clone()).unwrap(), png);
        service
            .delete_source_record(SourceTable::Clipboard, "1".into())
            .unwrap();
        maintain(&service);
        assert!(path.exists());
        service.release_attachment(lease).unwrap();
        maintain(&service);
        assert!(!path.exists());
    }
}
