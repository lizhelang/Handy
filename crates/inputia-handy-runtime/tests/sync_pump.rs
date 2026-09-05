use inputia_handy_runtime::{
    source::SourceTable,
    store::{HistoryQuery, IntegrationStore},
    sync::SourcePump,
};
use rusqlite::Connection;

fn make_source(conn: &Connection) {
    conn.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE transcription_history(
       id INTEGER PRIMARY KEY AUTOINCREMENT,file_name TEXT NOT NULL,timestamp INTEGER NOT NULL,
       saved INTEGER NOT NULL DEFAULT 0,title TEXT NOT NULL,transcription_text TEXT NOT NULL,post_processed_text TEXT);
       INSERT INTO transcription_history(file_name,timestamp,title,transcription_text) VALUES('a.wav',1,'first','before');").unwrap();
}

#[test]
fn real_source_changes_flow_into_index_and_recover_after_index_loss() {
    let temp = tempfile::tempdir().unwrap();
    let source_path = temp.path().join("history.db");
    let source = Connection::open(&source_path).unwrap();
    make_source(&source);
    let mut pump = SourcePump::attach(source, SourceTable::History).unwrap();
    let mut store = IntegrationStore::open(temp.path().join("index.db"), "fixture").unwrap();
    assert_eq!(pump.sync_batch(&mut store).unwrap().applied_events, 1);
    let writer = Connection::open(&source_path).unwrap();
    writer
        .execute(
            "UPDATE transcription_history SET saved=1,post_processed_text='after' WHERE id=1",
            [],
        )
        .unwrap();
    assert_eq!(pump.sync_batch(&mut store).unwrap().applied_events, 1);
    let items = store.query(&HistoryQuery::default()).unwrap();
    assert!(items[0].snapshot.starred);
    assert_eq!(items[0].snapshot.text.as_deref(), Some("after"));
    assert_eq!(store.revisions(&items[0].item_id).unwrap().len(), 2);
    assert_eq!(pump.sync_batch(&mut store).unwrap().applied_events, 0);
    let mut recovered =
        IntegrationStore::open(temp.path().join("recovered-index.db"), "fixture").unwrap();
    assert_eq!(pump.sync_batch(&mut recovered).unwrap().restored_records, 1);
    assert_eq!(recovered.query(&HistoryQuery::default()).unwrap(), items);
    writer
        .execute("DELETE FROM transcription_history WHERE id=1", [])
        .unwrap();
    pump.sync_batch(&mut recovered).unwrap();
    assert_eq!(recovered.item_count().unwrap(), 0);
    assert!(recovered.revisions(&items[0].item_id).unwrap().is_empty());
}

#[test]
fn failed_projection_does_not_acknowledge_source_and_restart_retries_it() {
    let temp = tempfile::tempdir().unwrap();
    let source_path = temp.path().join("history.db");
    let source = Connection::open(&source_path).unwrap();
    make_source(&source);
    let mut pump = SourcePump::attach(source, SourceTable::History).unwrap();
    let index_path = temp.path().join("index.db");
    let mut store = IntegrationStore::open(&index_path, "fixture").unwrap();
    let fault = Connection::open(&index_path).unwrap();
    fault.execute_batch("CREATE TRIGGER fail_receipt BEFORE INSERT ON integration_events BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
    assert!(pump.sync_batch(&mut store).is_err());
    assert_eq!(store.item_count().unwrap(), 0);
    let check = Connection::open(&source_path).unwrap();
    assert_eq!(
        check
            .query_row("SELECT COUNT(*) FROM unified_source_outbox", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    fault.execute_batch("DROP TRIGGER fail_receipt;").unwrap();
    drop(pump);
    let mut restarted =
        SourcePump::attach(Connection::open(source_path).unwrap(), SourceTable::History).unwrap();
    assert_eq!(restarted.sync_batch(&mut store).unwrap().applied_events, 1);
    assert_eq!(store.item_count().unwrap(), 1);
}
