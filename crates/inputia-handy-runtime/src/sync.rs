//! 后台同步泵：规范库提交成功后才确认源 outbox，索引丢失走一致性快照。

use crate::{
    source::{SourceError, SourceOutbox, SourceTable},
    store::{IntegrationStore, StoreError},
};
use rusqlite::Connection;
use std::fmt;

#[derive(Debug)]
pub enum SyncError {
    Source(SourceError),
    Store(StoreError),
}
impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(error) => write!(f, "source sync: {error}"),
            Self::Store(error) => write!(f, "projection sync: {error}"),
        }
    }
}
impl std::error::Error for SyncError {}
impl From<SourceError> for SyncError {
    fn from(error: SourceError) -> Self {
        Self::Source(error)
    }
}
impl From<StoreError> for SyncError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub applied_events: usize,
    pub restored_records: usize,
    pub through_sequence: u64,
}

pub struct SourcePump {
    connection: Connection,
    outbox: SourceOutbox,
    source: SourceTable,
}

impl SourcePump {
    /// 仅在已备份、已完成源表迁移的数据库上调用；不从输入法按键路径调用。
    pub fn attach(mut connection: Connection, source: SourceTable) -> Result<Self, SyncError> {
        let outbox = SourceOutbox::install(&mut connection, source)?;
        Ok(Self {
            connection,
            outbox,
            source,
        })
    }

    pub fn store_id(&self) -> &str {
        self.outbox.store_id()
    }

    pub fn source_table(&self) -> SourceTable {
        self.source
    }

    pub fn attachment_references(
        &mut self,
    ) -> Result<Vec<crate::attachment_store::AttachmentReference>, SyncError> {
        self.verify_file_identity()?;
        let refs = self
            .outbox
            .attachment_references(&mut self.connection, self.source)?;
        self.verify_file_identity()?;
        Ok(refs)
    }
    pub fn deleted_attachments(
        &self,
        item_id: &str,
        revision: u64,
        operation_id: &str,
    ) -> Result<Option<Vec<crate::source::DeletedAttachment>>, SyncError> {
        self.verify_file_identity()?;
        Ok(self
            .outbox
            .deleted_attachments(&self.connection, item_id, revision, operation_id)?)
    }

    /// SQLite 连接可能仍指向已被替换/移走的旧 inode；禁止把它当当前源继续写入。
    fn verify_file_identity(&self) -> Result<(), SyncError> {
        #[cfg(unix)]
        if let Some(path) = self.connection.path().filter(|path| !path.is_empty()) {
            let metadata = std::fs::symlink_metadata(path).map_err(|_| SourceError::WrongSource)?;
            if !metadata.file_type().is_file() {
                return Err(SourceError::WrongSource.into());
            }
            let mut moved: std::ffi::c_int = 0;
            // SAFETY: 连接在本线程独占；参数是 SQLite FCNTL_HAS_MOVED 要求的可写 int。
            let status = unsafe {
                rusqlite::ffi::sqlite3_file_control(
                    self.connection.handle(),
                    c"main".as_ptr(),
                    rusqlite::ffi::SQLITE_FCNTL_HAS_MOVED,
                    (&mut moved as *mut std::ffi::c_int).cast(),
                )
            };
            if status != rusqlite::ffi::SQLITE_OK || moved != 0 {
                return Err(SourceError::WrongSource.into());
            }
        }
        Ok(())
    }

    pub fn update_record(
        &mut self,
        record_id: &str,
        expected_revision: u64,
        operation_id: &str,
        patch: &crate::source::HistoryPatch,
    ) -> Result<crate::source::MutationResult, SyncError> {
        self.verify_file_identity()?;
        let result = self.outbox.update_record(
            &mut self.connection,
            self.source,
            record_id,
            expected_revision,
            operation_id,
            patch,
        )?;
        self.verify_file_identity()?;
        Ok(result)
    }

    pub fn delete_record(
        &mut self,
        record_id: &str,
        revision: u64,
        operation_id: &str,
    ) -> Result<crate::source::MutationResult, SyncError> {
        self.verify_file_identity()?;
        let result = self.outbox.delete_record(
            &mut self.connection,
            self.source,
            record_id,
            revision,
            operation_id,
        );
        self.verify_file_identity()?;
        Ok(result?)
    }

    pub fn delete_receipt(
        &self,
        item_id: &str,
        revision: u64,
        operation_id: &str,
    ) -> Result<Option<bool>, SyncError> {
        self.verify_file_identity()?;
        Ok(self
            .outbox
            .delete_receipt(&self.connection, item_id, revision, operation_id)?)
    }

    pub fn verify_deleted_record(
        &mut self,
        record_id: &str,
        revision: u64,
        operation_id: &str,
    ) -> Result<bool, SyncError> {
        self.verify_file_identity()?;
        let result = self.outbox.verify_deleted_record(
            &mut self.connection,
            self.source,
            record_id,
            revision,
            operation_id,
        )?;
        self.verify_file_identity()?;
        Ok(result)
    }

    /// 每次处理最多 2000 事件，允许外层服务公平处理 UI 和停止请求。
    pub fn sync_batch(&mut self, store: &mut IntegrationStore) -> Result<SyncReport, SyncError> {
        self.sync_batch_before_apply(store, || {})
    }

    /// 服务在投影写入前撤销已有输出许可；空批次不会无故打断准备中的插入。
    pub fn sync_batch_before_apply(
        &mut self,
        store: &mut IntegrationStore,
        mut before_apply: impl FnMut(),
    ) -> Result<SyncReport, SyncError> {
        self.verify_file_identity()?;
        store.register_source(self.source.logical_name(), self.outbox.store_id())?;
        let cursor = store.cursor(self.outbox.store_id())?;
        let events = match self.outbox.read_batch(&self.connection, cursor, 2_000) {
            Ok(events) => events,
            Err(SourceError::SnapshotRequired { .. }) => {
                let (header, records) =
                    self.outbox
                        .with_snapshot(&mut self.connection, self.source, |view| {
                            let mut records = Vec::new();
                            let mut after = None;
                            loop {
                                let page = view.read_page(after.as_deref(), 2_000)?;
                                if page.is_empty() {
                                    break;
                                }
                                after = page.last().map(|r| r.record_id.clone());
                                records.extend(page);
                            }
                            Ok((view.header.clone(), records))
                        })?;
                let policy = store.history_retention_policy()?;
                before_apply();
                self.verify_file_identity()?;
                store.restore_retained_history(&header, &records, &policy)?;
                self.outbox
                    .acknowledge(&mut self.connection, header.through_sequence)?;
                self.outbox.advance_policy(&self.connection, policy.epoch)?;
                self.verify_file_identity()?;
                return Ok(SyncReport {
                    applied_events: 0,
                    restored_records: records.len(),
                    through_sequence: header.through_sequence,
                });
            }
            Err(error) => return Err(error.into()),
        };
        let through_sequence = events.last().map_or(cursor, |event| event.seq);
        if !events.is_empty() {
            let policy = store.history_retention_policy()?;
            before_apply();
            self.verify_file_identity()?;
            store.apply_retained_history(&events, &policy)?;
            self.outbox
                .acknowledge(&mut self.connection, through_sequence)?;
        }
        self.outbox
            .advance_policy(&self.connection, store.policy_epoch()?)?;
        self.verify_file_identity()?;
        Ok(SyncReport {
            applied_events: events.len(),
            restored_records: 0,
            through_sequence,
        })
    }
}
