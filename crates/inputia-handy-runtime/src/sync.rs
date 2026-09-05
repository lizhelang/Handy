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

    /// 每次处理最多 2000 事件，允许外层服务公平处理 UI 和停止请求。
    pub fn sync_batch(&mut self, store: &mut IntegrationStore) -> Result<SyncReport, SyncError> {
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
                store.restore_retained_history(&header, &records, &policy)?;
                self.outbox
                    .acknowledge(&mut self.connection, header.through_sequence)?;
                self.outbox.advance_policy(&self.connection, policy.epoch)?;
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
            store.apply_retained_history(&events, &policy)?;
            self.outbox
                .acknowledge(&mut self.connection, through_sequence)?;
        }
        self.outbox
            .advance_policy(&self.connection, store.policy_epoch()?)?;
        Ok(SyncReport {
            applied_events: events.len(),
            restored_records: 0,
            through_sequence,
        })
    }
}
