use std::{
    path::Path,
    time::{Duration, Instant},
};

use inputia_handy_runtime::source::{
    SnapshotHeader, SnapshotRecord, SourceError, SourceOutbox, SourceTable,
};
use inputia_handy_runtime::store::{
    item_id, ApplyOutcome, ContentType, HistoryQuery, IntegrationStore, ItemSnapshot, SourceChange,
    SourceKind, SourceOperation, SourceTrust, StoreError,
};
use rusqlite::Connection;

fn snapshot(text: &str) -> ItemSnapshot {
    ItemSnapshot {
        source_kind: SourceKind::Voice,
        content_type: ContentType::Text,
        text: Some(text.to_owned()),
        title: None,
        starred: false,
        pinned: false,
        created_at_ms: 100,
        asset_ref: None,
        source_app: None,
        source_trust: SourceTrust::Unknown,
    }
}

fn change(store: &str, seq: u64, record: &str, revision: u64, text: &str) -> SourceChange {
    SourceChange {
        store_id: store.to_owned(),
        seq,
        event_id: format!("{store}-event-{seq}"),
        record_id: record.to_owned(),
        revision,
        operation: SourceOperation::Upsert,
        policy_epoch: 1,
        payload: Some(snapshot(text)),
    }
}

fn open(path: &Path) -> IntegrationStore {
    let mut store = IntegrationStore::open(path, "test-profile").unwrap();
    store.register_source("history", "voice-store").unwrap();
    store
}

#[cfg(unix)]
#[test]
fn new_database_is_private_and_symlink_cannot_redirect_the_store() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("private.db");
    let store = open(&path);
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let link = temp.path().join("redirect.db");
    symlink(&path, &link).unwrap();
    assert!(IntegrationStore::open(&link, "test-profile").is_err());
    assert_eq!(store.item_count().unwrap(), 0);
}

#[test]
fn physical_source_identity_is_unambiguous_and_profile_is_not_overwritten() {
    assert_ne!(item_id("a:b", "c"), item_id("a", "b:c"));
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("integration.db");
    let mut store = open(&path);
    store
        .register_source("clipboard", "clipboard-store")
        .unwrap();
    store
        .apply_change(&change("voice-store", 1, "1", 1, "same content"))
        .unwrap();
    let mut copied = change("clipboard-store", 1, "1", 1, "same content");
    copied.payload.as_mut().unwrap().source_kind = SourceKind::Clipboard;
    store.apply_change(&copied).unwrap();
    assert_eq!(store.item_count().unwrap(), 2);
    drop(store);
    assert!(matches!(
        IntegrationStore::open(&path, "another-profile"),
        Err(StoreError::ProfileMismatch)
    ));
    assert_eq!(open(&path).item_count().unwrap(), 2);
    let alien = temp.path().join("alien.db");
    Connection::open(&alien)
        .unwrap()
        .execute_batch(
            "CREATE TABLE valuable(data TEXT); INSERT INTO valuable VALUES ('retained');",
        )
        .unwrap();
    assert!(matches!(
        IntegrationStore::open(&alien, "test-profile"),
        Err(StoreError::UnsupportedSchema(0))
    ));
    let value: String = Connection::open(alien)
        .unwrap()
        .query_row("SELECT data FROM valuable", [], |row| row.get(0))
        .unwrap();
    assert_eq!(value, "retained");
}

#[test]
fn retention_authorization_is_current_and_does_not_forge_event_receipts() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = open(&temp.path().join("index.db"));
    let event = change("voice-store", 1, "one", 1, "old retained text");
    store.advance_policy_epoch(1, 2).unwrap();
    assert!(matches!(
        store.apply_change(&event),
        Err(StoreError::EpochMismatch { .. })
    ));
    let mut policy = store.history_retention_policy().unwrap();
    policy.retain_unknown = false;
    assert!(store
        .apply_retained_history(std::slice::from_ref(&event), &policy)
        .is_err());
    let policy = store.history_retention_policy().unwrap();
    store
        .apply_retained_history(std::slice::from_ref(&event), &policy)
        .unwrap();
    assert_eq!(
        store
            .apply_retained_history(std::slice::from_ref(&event), &policy)
            .unwrap(),
        vec![ApplyOutcome::Replay]
    );
    let mut altered = event.clone();
    altered.payload.as_mut().unwrap().text = Some("altered".into());
    assert!(matches!(
        store.apply_retained_history(&[altered], &policy),
        Err(StoreError::EventConflict)
    ));
    let mut future = change("voice-store", 2, "two", 1, "future");
    future.policy_epoch = 3;
    assert!(matches!(
        store.apply_retained_history(&[future], &policy),
        Err(StoreError::EpochMismatch { .. })
    ));
    let mut sensitive = change("voice-store", 2, "two", 1, "sensitive retained fixture");
    sensitive.payload.as_mut().unwrap().source_app = Some("com.1password.1password".into());
    store.apply_retained_history(&[sensitive], &policy).unwrap();
    assert_eq!(store.item_count().unwrap(), 1);
    assert_eq!(store.cursor("voice-store").unwrap(), 2);
}

#[test]
fn retained_replay_and_same_revision_snapshot_withdraw_existing_denied_projection() {
    use inputia_handy_runtime::source::{SnapshotHeader, SnapshotRecord};
    for use_snapshot in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let mut store = open(&temp.path().join("index.db"));
        let mut event = change("voice-store", 1, "one", 1, "legacy sensitive fixture");
        event.payload.as_mut().unwrap().source_app = Some("com.1password.1password".into());
        store.apply_change(&event).unwrap();
        assert_eq!(store.item_count().unwrap(), 1);
        store.advance_policy_epoch(1, 2).unwrap();
        let policy = store.history_retention_policy().unwrap();
        if use_snapshot {
            store
                .restore_retained_history(
                    &SnapshotHeader {
                        store_id: "voice-store".into(),
                        through_sequence: 1,
                        policy_epoch: 1,
                    },
                    &[SnapshotRecord {
                        record_id: "one".into(),
                        revision: 1,
                        payload: event.payload.clone(),
                    }],
                    &policy,
                )
                .unwrap();
        } else {
            store
                .apply_retained_history(std::slice::from_ref(&event), &policy)
                .unwrap();
        }
        assert_eq!(store.item_count().unwrap(), 0);
        assert!(store
            .revisions(&item_id("voice-store", "one"))
            .unwrap()
            .is_empty());
        assert_eq!(store.cursor("voice-store").unwrap(), 1);
        let mut retry = event.clone();
        retry.seq = 2;
        retry.event_id = "newer-event".into();
        retry.revision = 2;
        retry.policy_epoch = 2;
        retry.payload.as_mut().unwrap().source_app = None;
        store.apply_retained_history(&[retry], &policy).unwrap();
        assert_eq!(store.item_count().unwrap(), 0);
    }
}

#[test]
fn payload_identity_conflicts_are_rejected_without_advancing_or_mutating() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("integration.db");
    let mut store = open(&path);
    let original = change("voice-store", 1, "1", 1, "original");
    store.apply_change(&original).unwrap();
    assert_eq!(store.apply_change(&original).unwrap(), ApplyOutcome::Replay);
    let mut conflicting = original.clone();
    conflicting.payload.as_mut().unwrap().text = Some("different text".into());
    assert!(matches!(
        store.apply_change(&conflicting),
        Err(StoreError::EventConflict)
    ));
    conflicting = original.clone();
    conflicting.seq = 2;
    assert!(matches!(
        store.apply_change(&conflicting),
        Err(StoreError::EventConflict)
    ));
    conflicting = original.clone();
    conflicting.record_id = "different-record".into();
    assert!(matches!(
        store.apply_change(&conflicting),
        Err(StoreError::EventConflict)
    ));
    assert_eq!(store.cursor("voice-store").unwrap(), 1);
    assert_eq!(
        store
            .get(&item_id("voice-store", "1"))
            .unwrap()
            .unwrap()
            .snapshot
            .text
            .as_deref(),
        Some("original")
    );
    let conn = Connection::open(&path).unwrap();
    let columns: Vec<String> = conn
        .prepare("SELECT name FROM pragma_table_info('integration_events')")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(columns, ["event_id", "store_id", "seq", "digest"]);
    assert_eq!(
        conn.query_row("SELECT length(digest) FROM integration_events", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        32
    );
}

#[test]
fn gaps_unknown_replay_and_batch_failure_leave_consistent_cursor() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("integration.db");
    let mut store = open(&path);
    assert!(matches!(
        store.apply_change(&change("voice-store", 2, "two", 1, "second")),
        Err(StoreError::SequenceGap {
            expected: 1,
            actual: 2
        })
    ));
    let events = vec![
        change("voice-store", 1, "one", 1, "first"),
        change("voice-store", 3, "three", 1, "third"),
    ];
    assert!(store.apply_changes(&events).is_err());
    assert_eq!(store.cursor("voice-store").unwrap(), 0);
    assert_eq!(store.item_count().unwrap(), 0);
    store.apply_change(&events[0]).unwrap();
    let mut old = events[0].clone();
    old.event_id = "new-name-old-sequence".into();
    assert!(matches!(
        store.apply_change(&old),
        Err(StoreError::ReplayUnknown)
    ));
    drop(store);
    let mut store = open(&path);
    store
        .apply_change(&change("voice-store", 2, "two", 1, "second"))
        .unwrap();
    store.apply_change(&events[1]).unwrap();
    assert_eq!(store.cursor("voice-store").unwrap(), 3);
    assert_eq!(store.item_count().unwrap(), 3);
}

#[test]
fn sqlite_failure_after_item_update_rolls_back_item_event_and_revision() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("integration.db");
    let mut store = open(&path);
    store
        .apply_change(&change("voice-store", 1, "one", 1, "before"))
        .unwrap();
    let fault = Connection::open(&path).unwrap();
    fault.execute_batch("CREATE TRIGGER fail_receipt BEFORE INSERT ON integration_events BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    let update = change("voice-store", 2, "one", 2, "after");
    assert!(matches!(
        store.apply_change(&update),
        Err(StoreError::Sqlite(_))
    ));
    assert_eq!(store.cursor("voice-store").unwrap(), 1);
    assert_eq!(
        store
            .revisions(&item_id("voice-store", "one"))
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        store
            .get(&item_id("voice-store", "one"))
            .unwrap()
            .unwrap()
            .snapshot
            .text
            .as_deref(),
        Some("before")
    );
    fault.execute_batch("DROP TRIGGER fail_receipt").unwrap();
    drop(store);
    let mut store = open(&path);
    assert_eq!(store.apply_change(&update).unwrap(), ApplyOutcome::Applied);
    assert_eq!(
        store
            .revisions(&item_id("voice-store", "one"))
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn revisions_survive_restart_but_metadata_does_not_duplicate_text_versions() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("integration.db");
    let mut store = open(&path);
    let original = change("voice-store", 1, "one", 1, "原始转写");
    store.apply_change(&original).unwrap();
    let mut metadata = change("voice-store", 2, "one", 2, "原始转写");
    let payload = metadata.payload.as_mut().unwrap();
    payload.starred = true;
    payload.pinned = true;
    payload.title = Some("收藏标题".into());
    store.apply_change(&metadata).unwrap();
    assert_eq!(
        store
            .revisions(&item_id("voice-store", "one"))
            .unwrap()
            .len(),
        1
    );
    let mut revised = metadata.clone();
    revised.seq = 3;
    revised.event_id = "voice-store-event-3".into();
    revised.revision = 3;
    revised.payload.as_mut().unwrap().text = Some("纠正后转写".into());
    store.apply_change(&revised).unwrap();
    let mut stale = change("voice-store", 4, "one", 2, "过时文本");
    assert_eq!(
        store.apply_change(&stale).unwrap(),
        ApplyOutcome::StaleSuppressed
    );
    stale.seq = 5;
    stale.event_id = "voice-store-event-5".into();
    stale.revision = 3;
    assert_eq!(
        store.apply_change(&stale).unwrap(),
        ApplyOutcome::StaleSuppressed
    );
    drop(store);
    let store = open(&path);
    let versions = store.revisions(&item_id("voice-store", "one")).unwrap();
    assert_eq!(versions.len(), 2);
    assert_eq!(
        versions.iter().map(|v| v.revision).collect::<Vec<_>>(),
        vec![1, 3]
    );
    assert_eq!(versions[0].snapshot.text.as_deref(), Some("原始转写"));
    assert_eq!(versions[1].snapshot.text.as_deref(), Some("纠正后转写"));
    assert!(versions[1].snapshot.starred && versions[1].snapshot.pinned);
    assert_eq!(versions[1].snapshot.title.as_deref(), Some("收藏标题"));
    assert_eq!(store.cursor("voice-store").unwrap(), 5);
}

#[test]
fn delete_removes_all_revision_bodies_and_cannot_be_revived_by_newer_queue() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("integration.db");
    let mut store = open(&path);
    let original = change("voice-store", 1, "one", 1, "original private content");
    store.apply_change(&original).unwrap();
    store
        .apply_change(&change(
            "voice-store",
            2,
            "one",
            2,
            "revised private content",
        ))
        .unwrap();
    let mut delete = change("voice-store", 3, "one", 3, "");
    delete.operation = SourceOperation::Delete;
    delete.payload = None;
    store.apply_change(&delete).unwrap();
    assert_eq!(store.item_count().unwrap(), 0);
    assert!(store
        .revisions(&item_id("voice-store", "one"))
        .unwrap()
        .is_empty());
    drop(store);
    let mut store = open(&path);
    assert_eq!(store.apply_change(&original).unwrap(), ApplyOutcome::Replay);
    assert_eq!(
        store
            .apply_change(&change("voice-store", 4, "one", 100, "cannot resurrect"))
            .unwrap(),
        ApplyOutcome::DeletedSuppressed
    );
    assert_eq!(store.item_count().unwrap(), 0);
    assert_eq!(store.cursor("voice-store").unwrap(), 4);
    assert!(matches!(
        store.replace_source("history", "voice-store", "new-voice-store"),
        Err(StoreError::ReconciliationRequired)
    ));
    assert!(matches!(
        store.register_source("history", "new-voice-store"),
        Err(StoreError::SourceConflict)
    ));
    // 墓碑只保存来源身份与版本，不将正文作为删除键。
    let conn = Connection::open(&path).unwrap();
    let columns: Vec<String> = conn
        .prepare("SELECT name FROM pragma_table_info('integration_tombstones')")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(columns, ["logical_name", "record_id", "deleted_revision"]);
}

#[test]
fn stale_delete_does_not_remove_a_newer_version() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = open(&temp.path().join("integration.db"));
    store
        .apply_change(&change("voice-store", 1, "one", 9, "new text"))
        .unwrap();
    let mut delete = change("voice-store", 2, "one", 8, "");
    delete.operation = SourceOperation::Delete;
    delete.payload = None;
    assert_eq!(
        store.apply_change(&delete).unwrap(),
        ApplyOutcome::StaleSuppressed
    );
    assert_eq!(store.item_count().unwrap(), 1);
}

#[test]
fn explicit_empty_source_replacement_retires_old_identity() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = open(&temp.path().join("integration.db"));
    store
        .replace_source("history", "voice-store", "new-voice-store")
        .unwrap();
    assert!(matches!(
        store.apply_change(&change("voice-store", 1, "one", 1, "old instance")),
        Err(StoreError::RetiredStore)
    ));
    store
        .apply_change(&change("new-voice-store", 1, "one", 1, "new instance"))
        .unwrap();
    assert!(matches!(
        store.replace_source("history", "new-voice-store", "third-store"),
        Err(StoreError::ReconciliationRequired)
    ));
    assert_eq!(store.item_count().unwrap(), 1);
}

#[test]
fn epoch_cas_rejects_old_events_without_silently_consuming_them() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("integration.db");
    let mut store = open(&path);
    store.advance_policy_epoch(1, 2).unwrap();
    assert!(matches!(
        store.advance_policy_epoch(1, 3),
        Err(StoreError::EpochMismatch {
            expected: 2,
            actual: 1
        })
    ));
    let mut next = change(
        "voice-store",
        1,
        "one",
        1,
        "requires current policy approval",
    );
    assert!(matches!(
        store.apply_change(&next),
        Err(StoreError::EpochMismatch {
            expected: 2,
            actual: 1
        })
    ));
    assert_eq!(store.cursor("voice-store").unwrap(), 0);
    // 服务必须在最新隐私规则重新审核后才可生成同源位置的授权事件；存储层不自行重授权。
    next.policy_epoch = 2;
    store.apply_change(&next).unwrap();
    drop(store);
    let store = open(&path);
    assert_eq!(store.policy_epoch().unwrap(), 2);
    assert_eq!(store.item_count().unwrap(), 1);
}

#[test]
fn queries_use_current_text_type_source_literal_search_and_stable_pagination() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = open(&temp.path().join("integration.db"));
    let mut one = change("voice-store", 1, "one", 1, "first 100% match");
    one.payload.as_mut().unwrap().starred = true;
    store.apply_change(&one).unwrap();
    let mut two = change("voice-store", 2, "two", 1, "second");
    two.payload.as_mut().unwrap().title = Some("这是标题".into());
    two.payload.as_mut().unwrap().pinned = true;
    store.apply_change(&two).unwrap();
    let mut file = change("voice-store", 3, "file", 1, "");
    let payload = file.payload.as_mut().unwrap();
    payload.text = None;
    payload.content_type = ContentType::Files;
    payload.source_kind = SourceKind::Clipboard;
    payload.asset_ref = Some("managed-files-1".into());
    store.apply_change(&file).unwrap();
    assert_eq!(
        store
            .query(&HistoryQuery {
                search: Some("%".into()),
                ..HistoryQuery::default()
            })
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        store
            .query(&HistoryQuery {
                search: Some("标题".into()),
                ..HistoryQuery::default()
            })
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        store
            .query(&HistoryQuery {
                starred_only: true,
                ..HistoryQuery::default()
            })
            .unwrap()[0]
            .record_id,
        "one"
    );
    let files = store
        .query(&HistoryQuery {
            source_kind: Some(SourceKind::Clipboard),
            content_type: Some(ContentType::Files),
            ..HistoryQuery::default()
        })
        .unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(
        files[0].snapshot.asset_ref.as_deref(),
        Some("managed-files-1")
    );
    assert!(files[0].snapshot.text.is_none());
    let first = store
        .query(&HistoryQuery {
            limit: 1,
            ..HistoryQuery::default()
        })
        .unwrap();
    let second = store
        .query(&HistoryQuery {
            limit: 1,
            offset: 1,
            ..HistoryQuery::default()
        })
        .unwrap();
    assert_eq!(first[0].record_id, "two");
    assert_ne!(first[0].item_id, second[0].item_id);
    assert!(store
        .query(&HistoryQuery {
            limit: 501,
            ..HistoryQuery::default()
        })
        .is_err());
}

#[test]
fn concurrent_writer_wait_is_bounded_and_retry_recovers() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("integration.db");
    let mut store = open(&path);
    let blocker = Connection::open(&path).unwrap();
    blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
    let started = Instant::now();
    assert!(matches!(
        store.apply_change(&change("voice-store", 1, "one", 1, "text")),
        Err(StoreError::Sqlite(_))
    ));
    assert!(started.elapsed() < Duration::from_secs(2));
    blocker.execute_batch("ROLLBACK").unwrap();
    store
        .apply_change(&change("voice-store", 1, "one", 1, "text"))
        .unwrap();
    assert_eq!(store.cursor("voice-store").unwrap(), 1);
}

#[test]
fn fifty_thousand_mixed_records_replay_three_times_without_growth_and_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("integration.db");
    let mut store = open(&path);
    store
        .register_source("clipboard", "clipboard-store")
        .unwrap();
    let events: Vec<_> = (0..50_000_u64)
        .map(|index| {
            let source = if index % 2 == 0 {
                "voice-store"
            } else {
                "clipboard-store"
            };
            let seq = index / 2 + 1;
            let mut event = change(
                source,
                seq,
                &seq.to_string(),
                1,
                &format!("合成记录 {index} project-term"),
            );
            let payload = event.payload.as_mut().unwrap();
            payload.created_at_ms = index as i64;
            payload.source_kind = if index % 2 == 0 {
                SourceKind::Voice
            } else {
                SourceKind::Clipboard
            };
            payload.starred = index % 17 == 0;
            payload.pinned = index % 113 == 0;
            if index % 19 == 0 {
                payload.content_type = ContentType::Files;
                payload.asset_ref = Some(format!("managed-synthetic-{index}"));
                payload.text = None;
            }
            event
        })
        .collect();
    let start = Instant::now();
    for batch in events.chunks(1_000) {
        assert!(store
            .apply_changes(batch)
            .unwrap()
            .iter()
            .all(|result| *result == ApplyOutcome::Applied));
    }
    eprintln!(
        "baseline_store_50000_import_ms={}",
        start.elapsed().as_millis()
    );
    assert_eq!(store.item_count().unwrap(), 50_000);
    for _ in 0..3 {
        for batch in events.chunks(1_000) {
            assert!(store
                .apply_changes(batch)
                .unwrap()
                .iter()
                .all(|result| *result == ApplyOutcome::Replay));
        }
        assert_eq!(store.item_count().unwrap(), 50_000);
    }
    drop(store);
    let store = open(&path);
    assert_eq!(store.cursor("voice-store").unwrap(), 25_000);
    assert_eq!(store.cursor("clipboard-store").unwrap(), 25_000);
    assert_eq!(store.item_count().unwrap(), 50_000);
    assert_eq!(
        store
            .query(&HistoryQuery {
                search: Some("合成记录 49999".into()),
                ..HistoryQuery::default()
            })
            .unwrap()
            .len(),
        1
    );
    let conn = Connection::open(path).unwrap();
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM integration_events", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        50_000
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM integration_revisions", [], |row| row
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
}

fn source_fixture(path: &Path) -> (Connection, SourceOutbox) {
    let mut conn = Connection::open(path).unwrap();
    conn.execute_batch("CREATE TABLE transcription_history (
        id INTEGER PRIMARY KEY AUTOINCREMENT, file_name TEXT NOT NULL, timestamp INTEGER NOT NULL,
        saved BOOLEAN NOT NULL DEFAULT 0,title TEXT NOT NULL,transcription_text TEXT NOT NULL,
        post_processed_text TEXT,post_process_prompt TEXT,post_process_requested BOOLEAN DEFAULT 0);").unwrap();
    let source = SourceOutbox::install(&mut conn, SourceTable::History).unwrap();
    (conn, source)
}

fn source_insert(conn: &Connection, text: &str) {
    conn.execute("INSERT INTO transcription_history(file_name,timestamp,title,transcription_text) VALUES('fixture.wav',1,'fixture',?1)", [text]).unwrap();
}

fn full_snapshot(
    source: &SourceOutbox,
    conn: &mut Connection,
) -> (SnapshotHeader, Vec<SnapshotRecord>) {
    source
        .with_snapshot(conn, SourceTable::History, |view| {
            let mut records: Vec<SnapshotRecord> = Vec::new();
            loop {
                let page =
                    view.read_page(records.last().map(|record| record.record_id.as_str()), 1)?;
                if page.is_empty() {
                    break;
                }
                records.extend(page);
            }
            Ok((view.header.clone(), records))
        })
        .unwrap()
}

#[test]
fn acknowledged_source_recovers_lost_index_without_new_events_then_resumes_incrementally() {
    let temp = tempfile::tempdir().unwrap();
    let (mut conn, source) = source_fixture(&temp.path().join("history.db"));
    source_insert(&conn, "first fixture");
    source_insert(&conn, "second fixture");
    let path = temp.path().join("original-index.db");
    let mut original = IntegrationStore::open(&path, "test-profile").unwrap();
    original
        .register_source("history", source.store_id())
        .unwrap();
    let events = source.read_batch(&conn, 0, 100).unwrap();
    original.apply_changes(&events).unwrap();
    source.acknowledge(&mut conn, 2).unwrap();
    drop(original);
    // 独立空库代表索引丢失；不删源数据库或使用真实用户数据。
    let recovered_path = temp.path().join("recovered-index.db");
    let mut recovered = IntegrationStore::open(&recovered_path, "test-profile").unwrap();
    recovered
        .register_source("history", source.store_id())
        .unwrap();
    assert!(matches!(
        source.read_batch(&conn, 0, 100),
        Err(SourceError::SnapshotRequired {
            acknowledged_sequence: 2
        })
    ));
    let (header, records) = full_snapshot(&source, &mut conn);
    assert_eq!(records.len(), 2);
    let result = recovered
        .restore_source_snapshot(&header, &records)
        .unwrap();
    assert_eq!(result.applied, 2);
    assert_eq!(result.through_sequence, 2);
    assert_eq!(recovered.item_count().unwrap(), 2);
    assert!(source.read_batch(&conn, 2, 100).unwrap().is_empty());
    for _ in 0..3 {
        let result = recovered
            .restore_source_snapshot(&header, &records)
            .unwrap();
        assert_eq!(result.applied, 0);
        assert_eq!(result.stale_suppressed, 2);
    }
    drop(recovered);
    let mut recovered = IntegrationStore::open(&recovered_path, "test-profile").unwrap();
    assert_eq!(
        recovered.apply_change(&events[0]).unwrap(),
        ApplyOutcome::StaleSuppressed
    );
    let audit = Connection::open(&recovered_path).unwrap();
    assert_eq!(
        audit
            .query_row("SELECT COUNT(*) FROM integration_events", [], |row| row
                .get::<_, u64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        audit
            .query_row("SELECT COUNT(*) FROM integration_revisions", [], |row| row
                .get::<_, u64>(
                0
            ))
            .unwrap(),
        0
    );
    source_insert(&conn, "post recovery fixture");
    let next = source
        .read_batch(&conn, recovered.cursor(source.store_id()).unwrap(), 100)
        .unwrap();
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].seq, 3);
    assert_eq!(
        recovered.apply_change(&next[0]).unwrap(),
        ApplyOutcome::Applied
    );
    assert_eq!(
        recovered.apply_change(&next[0]).unwrap(),
        ApplyOutcome::Replay
    );
    assert_eq!(recovered.cursor(source.store_id()).unwrap(), 3);
    assert_eq!(recovered.item_count().unwrap(), 3);
}

#[test]
fn source_snapshot_propagates_deletes_and_prunes_missing_records_without_resurrection() {
    let temp = tempfile::tempdir().unwrap();
    let (mut conn, source) = source_fixture(&temp.path().join("history.db"));
    source_insert(&conn, "deleted fixture");
    source_insert(&conn, "surviving fixture");
    let index_path = temp.path().join("index.db");
    let mut index = IntegrationStore::open(&index_path, "test-profile").unwrap();
    index.register_source("history", source.store_id()).unwrap();
    let original_events = source.read_batch(&conn, 0, 100).unwrap();
    index.apply_changes(&original_events).unwrap();
    source.acknowledge(&mut conn, 2).unwrap();
    conn.execute("DELETE FROM transcription_history WHERE id=1", [])
        .unwrap();
    // 模拟旧索引中的额外记录；完整源快照必须删除它而非仅追加当前记录。
    index
        .apply_change(&change(source.store_id(), 3, "orphan", 7, "orphan fixture"))
        .unwrap();
    let (header, records) = full_snapshot(&source, &mut conn);
    assert_eq!(records[0].payload, None);
    let restored = index.restore_source_snapshot(&header, &records).unwrap();
    assert_eq!(restored.applied, 1);
    assert_eq!(restored.missing_deleted, 1);
    assert_eq!(index.item_count().unwrap(), 1);
    assert!(index
        .revisions(&item_id(source.store_id(), "1"))
        .unwrap()
        .is_empty());
    let mut old_snapshot = records.clone();
    old_snapshot[0].payload = original_events[0].payload.clone();
    old_snapshot[0].revision = 1;
    old_snapshot.push(SnapshotRecord {
        record_id: "orphan".into(),
        revision: 100,
        payload: Some(snapshot("still deleted")),
    });
    assert_eq!(
        index
            .restore_source_snapshot(&header, &old_snapshot)
            .unwrap()
            .deleted_suppressed,
        2
    );
    assert_eq!(index.item_count().unwrap(), 1);
    drop(index);
    let mut index = IntegrationStore::open(&index_path, "test-profile").unwrap();
    assert_eq!(
        index
            .apply_change(&change(source.store_id(), 4, "1", 100, "old queue"))
            .unwrap(),
        ApplyOutcome::DeletedSuppressed
    );
    assert_eq!(
        index
            .apply_change(&change(source.store_id(), 5, "orphan", 101, "old queue"))
            .unwrap(),
        ApplyOutcome::DeletedSuppressed
    );
    assert_eq!(index.item_count().unwrap(), 1);
}

fn test_header(through_sequence: u64) -> SnapshotHeader {
    SnapshotHeader {
        store_id: "voice-store".into(),
        through_sequence,
        policy_epoch: 1,
    }
}

#[test]
fn snapshot_floor_keeps_known_receipt_checks_and_unknown_events_have_no_side_effects() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("index.db");
    let mut store = open(&path);
    let original = change("voice-store", 1, "one", 1, "original");
    store.apply_change(&original).unwrap();
    store
        .restore_source_snapshot(
            &test_header(3),
            &[SnapshotRecord {
                record_id: "one".into(),
                revision: 1,
                payload: original.payload.clone(),
            }],
        )
        .unwrap();
    assert_eq!(store.apply_change(&original).unwrap(), ApplyOutcome::Replay);
    let mut conflict = original.clone();
    conflict.payload = Some(snapshot("mismatching old receipt"));
    assert!(matches!(
        store.apply_change(&conflict),
        Err(StoreError::EventConflict)
    ));
    conflict = original.clone();
    conflict.event_id = "other-event-same-sequence".into();
    assert!(matches!(
        store.apply_change(&conflict),
        Err(StoreError::EventConflict)
    ));
    let mut unknown = change("voice-store", 2, "ghost", 999, "must not become visible");
    assert_eq!(
        store.apply_change(&unknown).unwrap(),
        ApplyOutcome::StaleSuppressed
    );
    unknown.seq = 3;
    unknown.operation = SourceOperation::Delete;
    unknown.payload = None;
    unknown.record_id = "one".into();
    assert_eq!(
        store.apply_change(&unknown).unwrap(),
        ApplyOutcome::StaleSuppressed
    );
    assert_eq!(store.item_count().unwrap(), 1);
    assert_eq!(store.cursor("voice-store").unwrap(), 3);
    let db = Connection::open(&path).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM integration_events", [], |row| row
            .get::<_, u64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM integration_tombstones", [], |row| row
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
    drop(store);
    let mut store = open(&path);
    assert_eq!(
        store.apply_change(&unknown).unwrap(),
        ApplyOutcome::StaleSuppressed
    );
    assert_eq!(
        store
            .apply_change(&change("voice-store", 4, "two", 1, "next"))
            .unwrap(),
        ApplyOutcome::Applied
    );
}

#[test]
fn snapshot_rejects_backward_watermark_epoch_and_duplicate_records_preserves_newer_revision() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("index.db");
    let mut store = open(&path);
    store
        .apply_change(&change("voice-store", 1, "one", 10, "newest"))
        .unwrap();
    let stale = SnapshotRecord {
        record_id: "one".into(),
        revision: 9,
        payload: Some(snapshot("old")),
    };
    assert_eq!(
        store
            .restore_source_snapshot(&test_header(2), std::slice::from_ref(&stale))
            .unwrap()
            .stale_suppressed,
        1
    );
    let stale_delete = SnapshotRecord {
        payload: None,
        ..stale.clone()
    };
    assert_eq!(
        store
            .restore_source_snapshot(&test_header(2), &[stale_delete])
            .unwrap()
            .stale_suppressed,
        1
    );
    assert_eq!(
        store
            .get(&item_id("voice-store", "one"))
            .unwrap()
            .unwrap()
            .snapshot
            .text
            .as_deref(),
        Some("newest")
    );
    assert!(matches!(
        store.restore_source_snapshot(&test_header(1), &[]),
        Err(StoreError::SnapshotRegression {
            current: 2,
            actual: 1
        })
    ));
    assert!(matches!(
        store.restore_source_snapshot(&test_header(3), &[stale.clone(), stale]),
        Err(StoreError::Invalid(_))
    ));
    assert_eq!(store.cursor("voice-store").unwrap(), 2);
    store.advance_policy_epoch(1, 2).unwrap();
    assert!(matches!(
        store.restore_source_snapshot(&test_header(3), &[]),
        Err(StoreError::EpochMismatch {
            expected: 2,
            actual: 1
        })
    ));
    assert_eq!(store.item_count().unwrap(), 1);
    assert_eq!(store.cursor("voice-store").unwrap(), 2);
    let db = Connection::open(&path).unwrap();
    let columns: Vec<String> = db
        .prepare("SELECT name FROM pragma_table_info('integration_instances')")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(columns, ["store_id", "logical_name", "active", "cursor"]);
}

#[test]
fn snapshot_sqlite_failure_rolls_back_updated_items_deletions_revisions_cursor_and_floor() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("index.db");
    let mut store = open(&path);
    store
        .apply_change(&change("voice-store", 1, "one", 1, "before"))
        .unwrap();
    store
        .apply_change(&change("voice-store", 2, "missing", 1, "keep on rollback"))
        .unwrap();
    let db = Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_snapshot_floor BEFORE INSERT ON integration_meta WHEN NEW.key LIKE 'snapshot_floor:%' BEGIN SELECT RAISE(ABORT,'injected snapshot failure'); END;").unwrap();
    let records = vec![
        SnapshotRecord {
            record_id: "one".into(),
            revision: 2,
            payload: Some(snapshot("after")),
        },
        SnapshotRecord {
            record_id: "explicit-delete".into(),
            revision: 1,
            payload: None,
        },
    ];
    assert!(matches!(
        store.restore_source_snapshot(&test_header(5), &records),
        Err(StoreError::Sqlite(_))
    ));
    assert_eq!(store.item_count().unwrap(), 2);
    assert_eq!(store.cursor("voice-store").unwrap(), 2);
    assert_eq!(
        store
            .get(&item_id("voice-store", "one"))
            .unwrap()
            .unwrap()
            .snapshot
            .text
            .as_deref(),
        Some("before")
    );
    assert_eq!(
        store
            .revisions(&item_id("voice-store", "one"))
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM integration_tombstones", [], |row| row
            .get::<_, u64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM integration_meta WHERE key LIKE 'snapshot_floor:%'",
            [],
            |row| row.get::<_, u64>(0)
        )
        .unwrap(),
        0
    );
    db.execute_batch("DROP TRIGGER fail_snapshot_floor")
        .unwrap();
    drop(store);
    let mut store = open(&path);
    let result = store
        .restore_source_snapshot(&test_header(5), &records)
        .unwrap();
    assert_eq!(result.applied, 2);
    assert_eq!(result.missing_deleted, 1);
    assert_eq!(store.cursor("voice-store").unwrap(), 5);
    assert_eq!(store.item_count().unwrap(), 1);
    assert_eq!(
        store
            .revisions(&item_id("voice-store", "one"))
            .unwrap()
            .len(),
        2
    );
}
