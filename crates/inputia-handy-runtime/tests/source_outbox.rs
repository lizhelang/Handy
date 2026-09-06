use inputia_handy_runtime::source::{SourceError, SourceOutbox, SourceTable};
use inputia_handy_runtime::store::{ContentType, SourceOperation, SourceTrust};
use rusqlite::Connection;

fn history(conn: &Connection) {
    conn.execute_batch("CREATE TABLE transcription_history (
        id INTEGER PRIMARY KEY AUTOINCREMENT, file_name TEXT NOT NULL, timestamp INTEGER NOT NULL,
        saved BOOLEAN NOT NULL DEFAULT 0,title TEXT NOT NULL,transcription_text TEXT NOT NULL,
        post_processed_text TEXT,post_process_prompt TEXT,post_process_requested BOOLEAN DEFAULT 0);").unwrap();
}

fn insert(conn: &Connection, text: &str) {
    conn.execute("INSERT INTO transcription_history(file_name,timestamp,title,transcription_text) VALUES('fixture.wav',1,'fixture',?1)", [text]).unwrap();
}

#[test]
fn successful_voice_without_wav_is_retained_and_replayed_without_fake_attachment() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("history.db");
    let mut conn = Connection::open(&path).unwrap();
    history(&conn);
    let outbox = SourceOutbox::install(&mut conn, SourceTable::History).unwrap();
    conn.execute("INSERT INTO transcription_history(file_name,timestamp,title,transcription_text,post_processed_text) VALUES('',1,'fixture','原始转写','成功文字')", []).unwrap();
    let first = outbox.read_batch(&conn, 0, 100).unwrap();
    assert_eq!(first.len(), 1);
    let payload = first[0].payload.as_ref().unwrap();
    assert_eq!(payload.text.as_deref(), Some("成功文字"));
    assert_eq!(payload.asset_ref, None);
    assert_eq!(
        payload.source_kind,
        inputia_handy_runtime::store::SourceKind::Voice
    );
    drop(conn);
    let mut conn = Connection::open(&path).unwrap();
    let reopened = SourceOutbox::install(&mut conn, SourceTable::History).unwrap();
    assert_eq!(reopened.read_batch(&conn, 0, 100).unwrap(), first);
    assert_eq!(
        conn.query_row(
            "SELECT transcription_text FROM transcription_history",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "原始转写"
    );
    conn.execute("DELETE FROM transcription_history", [])
        .unwrap();
    let events = reopened.read_batch(&conn, first[0].seq, 100).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].operation, SourceOperation::Delete);
}

#[test]
fn source_mutation_and_outbox_rollback_together() {
    let mut conn = Connection::open_in_memory().unwrap();
    history(&conn);
    let outbox = SourceOutbox::install(&mut conn, SourceTable::History).unwrap();
    {
        let tx = conn.transaction().unwrap();
        insert(&tx, "discarded fixture");
        // 不提交源事务。
    }
    assert!(outbox.read_batch(&conn, 0, 100).unwrap().is_empty());
    insert(&conn, "retained fixture");
    let events = outbox.read_batch(&conn, 0, 100).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].seq, 1);
    assert_eq!(
        events[0].payload.as_ref().unwrap().text.as_deref(),
        Some("retained fixture")
    );
}

#[test]
fn bootstrap_is_latest_first_complete_and_repeatable() {
    let mut conn = Connection::open_in_memory().unwrap();
    history(&conn);
    let tx = conn.transaction().unwrap();
    for _ in 0..5_010 {
        insert(&tx, "synthetic fixture");
    }
    tx.commit().unwrap();
    let outbox = SourceOutbox::install(&mut conn, SourceTable::History).unwrap();
    let first = outbox.read_batch(&conn, 0, 2_000).unwrap();
    assert_eq!(first[0].record_id, "5010");
    let second = outbox
        .read_batch(&conn, first.last().unwrap().seq, 2_000)
        .unwrap();
    let third = outbox
        .read_batch(&conn, second.last().unwrap().seq, 2_000)
        .unwrap();
    assert_eq!(first.len() + second.len() + third.len(), 5_010);
    let again = SourceOutbox::install(&mut conn, SourceTable::History).unwrap();
    assert_eq!(outbox.store_id(), again.store_id());
    assert_eq!(first, again.read_batch(&conn, 0, 2_000).unwrap());
}

#[test]
fn every_metadata_update_and_automatic_delete_has_ordered_revision() {
    let mut conn = Connection::open_in_memory().unwrap();
    history(&conn);
    let outbox = SourceOutbox::install(&mut conn, SourceTable::History).unwrap();
    insert(&conn, "original");
    conn.execute("UPDATE transcription_history SET saved=1 WHERE id=1", [])
        .unwrap();
    conn.execute(
        "UPDATE transcription_history SET post_processed_text='corrected' WHERE id=1",
        [],
    )
    .unwrap();
    conn.execute("DELETE FROM transcription_history WHERE saved=1", [])
        .unwrap();
    let events = outbox.read_batch(&conn, 0, 100).unwrap();
    assert_eq!(
        events.iter().map(|e| e.revision).collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    assert!(events[1].payload.as_ref().unwrap().starred);
    assert_eq!(
        events[2].payload.as_ref().unwrap().text.as_deref(),
        Some("corrected")
    );
    assert_eq!(events[3].operation, SourceOperation::Delete);
    assert!(events[3].payload.is_none());
}

#[test]
fn replayed_toggle_does_not_execute_again_and_conflicting_retry_is_rejected() {
    let mut conn = Connection::open_in_memory().unwrap();
    history(&conn);
    let outbox = SourceOutbox::install(&mut conn, SourceTable::History).unwrap();
    insert(&conn, "fixture");
    let result = outbox
        .mutate_once(&mut conn, "request-1", "digest-1", |tx| {
            tx.execute(
                "UPDATE transcription_history SET saved=NOT saved WHERE id=1",
                [],
            )?;
            Ok("saved".into())
        })
        .unwrap();
    assert!(!result.replayed);
    let replay = outbox
        .mutate_once(&mut conn, "request-1", "digest-1", |_| {
            panic!("duplicate side effect")
        })
        .unwrap();
    assert!(replay.replayed);
    assert_eq!(replay.response, "saved");
    assert!(matches!(
        outbox.mutate_once(&mut conn, "request-1", "different", |_| panic!(
            "conflicting mutation"
        )),
        Err(SourceError::OperationConflict)
    ));
    assert_eq!(outbox.read_batch(&conn, 0, 100).unwrap().len(), 2);
}

#[test]
fn disk_reopen_preserves_identity_receipts_and_next_sequence_after_ack() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("history.db");
    let mut conn = Connection::open(&path).unwrap();
    history(&conn);
    let outbox = SourceOutbox::install(&mut conn, SourceTable::History).unwrap();
    insert(&conn, "first");
    let id = outbox.store_id().to_owned();
    outbox.acknowledge(&mut conn, 1).unwrap();
    assert!(matches!(
        outbox.read_batch(&conn, 0, 100),
        Err(SourceError::SnapshotRequired {
            acknowledged_sequence: 1
        })
    ));
    drop(conn);
    let mut conn = Connection::open(path).unwrap();
    let reopened = SourceOutbox::install(&mut conn, SourceTable::History).unwrap();
    assert_eq!(reopened.store_id(), id);
    insert(&conn, "second");
    assert_eq!(reopened.read_batch(&conn, 1, 100).unwrap()[0].seq, 2);
    assert!(matches!(
        reopened.acknowledge(&mut conn, 3),
        Err(SourceError::InvalidAcknowledgement)
    ));
    assert_eq!(reopened.read_batch(&conn, 1, 100).unwrap().len(), 1);
}

#[test]
fn failed_source_operation_does_not_leave_outbox_or_request_receipt() {
    let mut conn = Connection::open_in_memory().unwrap();
    history(&conn);
    let outbox = SourceOutbox::install(&mut conn, SourceTable::History).unwrap();
    let result = outbox.mutate_once(&mut conn, "failed-op", "digest", |tx| {
        insert(tx, "rolled back");
        Err(rusqlite::Error::InvalidQuery)
    });
    assert!(result.is_err());
    assert!(outbox.read_batch(&conn, 0, 10).unwrap().is_empty());
    let retry = outbox
        .mutate_once(&mut conn, "failed-op", "digest", |tx| {
            insert(tx, "retry");
            Ok("success".into())
        })
        .unwrap();
    assert!(!retry.replayed);
}

#[test]
fn ack_then_index_loss_rebuilds_live_rows_and_deletions_at_one_watermark() {
    let mut conn = Connection::open_in_memory().unwrap();
    history(&conn);
    let outbox = SourceOutbox::install(&mut conn, SourceTable::History).unwrap();
    insert(&conn, "surviving");
    insert(&conn, "deleted");
    conn.execute("DELETE FROM transcription_history WHERE id=2", [])
        .unwrap();
    outbox.acknowledge(&mut conn, 3).unwrap();
    assert!(matches!(
        outbox.read_batch(&conn, 0, 10),
        Err(SourceError::SnapshotRequired {
            acknowledged_sequence: 3
        })
    ));
    outbox
        .with_snapshot(&mut conn, SourceTable::History, |view| {
            assert_eq!(view.header.through_sequence, 3);
            assert_eq!(view.header.store_id, outbox.store_id());
            let page1 = view.read_page(None, 1)?;
            let page2 = view.read_page(Some(&page1[0].record_id), 1)?;
            assert_eq!(
                page1[0].payload.as_ref().unwrap().text.as_deref(),
                Some("surviving")
            );
            assert!(page2[0].payload.is_none());
            assert_eq!(page2[0].revision, 2);
            assert!(view.read_page(Some(&page2[0].record_id), 1)?.is_empty());
            Ok(())
        })
        .unwrap();
    insert(&conn, "created after snapshot");
    assert_eq!(outbox.read_batch(&conn, 3, 10).unwrap()[0].seq, 4);
}

#[test]
fn clipboard_images_files_and_source_unknown_do_not_become_typed_terms() {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE clipboard_history (
        id INTEGER PRIMARY KEY AUTOINCREMENT,content_type TEXT NOT NULL,full_text TEXT,title TEXT,
        source_app TEXT,is_favorite BOOLEAN DEFAULT 0,is_pinned BOOLEAN DEFAULT 0,
        created_at INTEGER NOT NULL,image_path TEXT);",
    )
    .unwrap();
    let outbox = SourceOutbox::install(&mut conn, SourceTable::Clipboard).unwrap();
    conn.execute_batch("INSERT INTO clipboard_history(content_type,created_at,image_path) VALUES('image',1,'fixture.png');
        INSERT INTO clipboard_history(content_type,created_at,full_text,source_app) VALUES('file',1,'[\"/tmp/fixture\"]','observed-app');").unwrap();
    let events = outbox.read_batch(&conn, 0, 10).unwrap();
    assert_eq!(
        events[0].payload.as_ref().unwrap().content_type,
        ContentType::Image
    );
    assert_eq!(
        events[0].payload.as_ref().unwrap().asset_ref.as_deref(),
        Some("fixture.png")
    );
    assert_eq!(
        events[1].payload.as_ref().unwrap().content_type,
        ContentType::Files
    );
    assert_eq!(
        events[1].payload.as_ref().unwrap().source_trust,
        SourceTrust::Unknown
    );
    assert_eq!(
        events[1].payload.as_ref().unwrap().source_app.as_deref(),
        Some("observed-app")
    );
}

#[test]
fn policy_epoch_changes_only_new_events_and_cannot_regress() {
    let mut conn = Connection::open_in_memory().unwrap();
    history(&conn);
    let outbox = SourceOutbox::install(&mut conn, SourceTable::History).unwrap();
    insert(&conn, "before");
    outbox.advance_policy(&conn, 2).unwrap();
    insert(&conn, "after");
    assert!(matches!(
        outbox.advance_policy(&conn, 1),
        Err(SourceError::PolicyRegression)
    ));
    assert_eq!(
        outbox
            .read_batch(&conn, 0, 10)
            .unwrap()
            .iter()
            .map(|e| e.policy_epoch)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[test]
fn fifty_thousand_snapshot_rows_use_primary_key_lookup_and_preserve_every_record() {
    let mut conn = Connection::open_in_memory().unwrap();
    history(&conn);
    let tx = conn.transaction().unwrap();
    for _ in 0..50_000 {
        insert(&tx, "synthetic snapshot");
    }
    tx.commit().unwrap();
    let outbox = SourceOutbox::install(&mut conn, SourceTable::History).unwrap();
    let plan: Vec<String> = conn
        .prepare(
            "EXPLAIN QUERY PLAN SELECT version.record_id,source_row.transcription_text
        FROM unified_source_versions version LEFT JOIN transcription_history source_row
        ON source_row.id=CAST(version.record_id AS INTEGER)
        WHERE version.record_id>'' ORDER BY version.record_id LIMIT 2000",
        )
        .unwrap()
        .query_map([], |row| row.get(3))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(plan
        .iter()
        .any(|line| line.contains("SEARCH source_row USING INTEGER PRIMARY KEY")));
    let start = std::time::Instant::now();
    outbox
        .with_snapshot(&mut conn, SourceTable::History, |view| {
            let mut cursor = None;
            let mut count = 0;
            loop {
                let page = view.read_page(cursor.as_deref(), 2_000)?;
                if page.is_empty() {
                    break;
                }
                count += page.len();
                cursor = page.last().map(|item| item.record_id.clone());
            }
            assert_eq!(count, 50_000);
            Ok(())
        })
        .unwrap();
    eprintln!(
        "snapshot_records=50000 elapsed_ms={}",
        start.elapsed().as_millis()
    );
    assert!(matches!(
        outbox.read_batch(&conn, 50_001, 10),
        Err(SourceError::CursorAhead {
            maximum_sequence: 50_000
        })
    ));
}
