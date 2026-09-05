use inputia_handy_runtime::source::{HistoryPatch, SourceOutbox, SourceTable};
use rusqlite::Connection;

fn fixture() -> (Connection, SourceOutbox) {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE clipboard_history(id INTEGER PRIMARY KEY AUTOINCREMENT,content_type TEXT,full_text TEXT,content_preview TEXT,content_hash TEXT UNIQUE,size_bytes INTEGER,title TEXT,source_app TEXT,is_favorite INTEGER DEFAULT 0,is_pinned INTEGER DEFAULT 0,created_at INTEGER,image_path TEXT);
       INSERT INTO clipboard_history(content_type,full_text,content_preview,content_hash,size_bytes,created_at) VALUES('text','before','before','hash-1',6,1);
       INSERT INTO clipboard_history(content_type,full_text,content_preview,content_hash,size_bytes,created_at) VALUES('file','[\"/tmp/fixture\"]','fixture','hash-2',20,2);").unwrap();
    let outbox = SourceOutbox::install(&mut conn, SourceTable::Clipboard).unwrap();
    (conn, outbox)
}

#[test]
fn text_edits_are_atomic_idempotent_and_recompute_content_metadata() {
    let (mut conn, outbox) = fixture();
    let patch = HistoryPatch {
        text: Some("修订文本".into()),
        title: Some("命名".into()),
        starred: Some(true),
        ..Default::default()
    };
    let result = outbox
        .update_record(&mut conn, SourceTable::Clipboard, "1", 1, "edit-1", &patch)
        .unwrap();
    assert_eq!(result.response, "2");
    assert!(
        outbox
            .update_record(&mut conn, SourceTable::Clipboard, "1", 1, "edit-1", &patch)
            .unwrap()
            .replayed
    );
    let (text,preview,hash,size):(String,String,String,i64)=conn.query_row("SELECT full_text,content_preview,content_hash,size_bytes FROM clipboard_history WHERE id=1",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?))).unwrap();
    assert_eq!(text, "修订文本");
    assert_eq!(preview, text);
    assert_eq!(hash.len(), 64);
    assert_eq!(size, text.len() as i64);
    assert!(outbox
        .update_record(
            &mut conn,
            SourceTable::Clipboard,
            "1",
            1,
            "stale",
            &HistoryPatch {
                text: Some("old write".into()),
                ..Default::default()
            }
        )
        .is_err());
    assert_eq!(outbox.read_batch(&conn, 0, 100).unwrap().len(), 3);
}

#[test]
fn editing_file_text_is_rejected_without_changing_original_formats() {
    let (mut conn, outbox) = fixture();
    let before = outbox.read_batch(&conn, 0, 100).unwrap();
    assert!(outbox
        .update_record(
            &mut conn,
            SourceTable::Clipboard,
            "2",
            1,
            "bad-edit",
            &HistoryPatch {
                text: Some("not a file".into()),
                ..Default::default()
            }
        )
        .is_err());
    assert_eq!(before, outbox.read_batch(&conn, 0, 100).unwrap());
    let kind: String = conn
        .query_row(
            "SELECT content_type FROM clipboard_history WHERE id=2",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(kind, "file");
}

#[test]
fn clearing_voice_text_preserves_original_and_later_transcription_replaces_override() {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE transcription_history(id INTEGER PRIMARY KEY AUTOINCREMENT,file_name TEXT,timestamp INTEGER,saved INTEGER,title TEXT,transcription_text TEXT,post_processed_text TEXT);
        INSERT INTO transcription_history VALUES(1,'test.wav',1,0,'fixture','original transcript',NULL);").unwrap();
    let outbox = SourceOutbox::install(&mut conn, SourceTable::History).unwrap();
    outbox
        .update_record(
            &mut conn,
            SourceTable::History,
            "1",
            1,
            "clear",
            &HistoryPatch {
                text: Some(String::new()),
                ..Default::default()
            },
        )
        .unwrap();
    let events = outbox.read_batch(&conn, 0, 100).unwrap();
    assert_eq!(
        events
            .last()
            .unwrap()
            .payload
            .as_ref()
            .unwrap()
            .text
            .as_deref(),
        Some("")
    );
    let original: String = conn
        .query_row(
            "SELECT transcription_text FROM transcription_history WHERE id=1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(original, "original transcript");
    outbox
        .update_record(
            &mut conn,
            SourceTable::History,
            "1",
            2,
            "pin",
            &HistoryPatch {
                pinned: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
    let events = outbox.read_batch(&conn, 0, 100).unwrap();
    assert_eq!(
        events
            .last()
            .unwrap()
            .payload
            .as_ref()
            .unwrap()
            .text
            .as_deref(),
        Some("")
    );
    assert!(events.last().unwrap().payload.as_ref().unwrap().pinned);
    conn.execute(
        "UPDATE transcription_history SET post_processed_text='new transcription' WHERE id=1",
        [],
    )
    .unwrap();
    assert_eq!(
        outbox
            .read_batch(&conn, 0, 100)
            .unwrap()
            .last()
            .unwrap()
            .payload
            .as_ref()
            .unwrap()
            .text
            .as_deref(),
        Some("new transcription")
    );
    outbox
        .update_record(
            &mut conn,
            SourceTable::History,
            "1",
            4,
            "edit-again",
            &HistoryPatch {
                text: Some("sensitive edited fixture".into()),
                ..Default::default()
            },
        )
        .unwrap();
    conn.execute("DELETE FROM transcription_history WHERE id=1", [])
        .unwrap();
    let annotations: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM unified_source_annotations WHERE record_id='1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(annotations, 0);
}
