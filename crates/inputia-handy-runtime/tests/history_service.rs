use inputia_handy_runtime::{
    service::HistoryService,
    store::{HistoryQuery, SourceKind},
};
use rusqlite::Connection;
use std::sync::{Arc, Mutex};

#[test]
fn background_service_combines_real_sources_and_notifies_revisions() {
    let root = tempfile::tempdir().unwrap();
    let history = Connection::open(root.path().join("history.db")).unwrap();
    history.execute_batch("CREATE TABLE transcription_history(id INTEGER PRIMARY KEY AUTOINCREMENT,file_name TEXT,timestamp INTEGER,saved INTEGER,title TEXT,transcription_text TEXT,post_processed_text TEXT);
       INSERT INTO transcription_history VALUES(1,'voice.wav',1,0,'语音','实际语音记录',NULL);").unwrap();
    let clipboard = Connection::open(root.path().join("clipboard.db")).unwrap();
    clipboard.execute_batch("CREATE TABLE clipboard_history(id INTEGER PRIMARY KEY AUTOINCREMENT,content_type TEXT,full_text TEXT,title TEXT,is_favorite INTEGER,is_pinned INTEGER,created_at INTEGER,image_path TEXT,source_app TEXT);
       INSERT INTO clipboard_history VALUES(1,'text','实际复制记录',NULL,1,0,2,NULL,NULL);").unwrap();
    let changes = Arc::new(Mutex::new(Vec::new()));
    let notification = changes.clone();
    let service = HistoryService::start(root.path().into(), "fixture".into(), move |generation| {
        notification.lock().unwrap().push(generation)
    })
    .unwrap();
    service.synchronize().unwrap();
    let items = service.query(HistoryQuery::default()).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].snapshot.source_kind, SourceKind::Clipboard);
    assert_eq!(items[1].snapshot.source_kind, SourceKind::Voice);
    let voice_id = items[1].item_id.clone();
    history
        .execute(
            "UPDATE transcription_history SET post_processed_text='修订文本',saved=1 WHERE id=1",
            [],
        )
        .unwrap();
    service.synchronize().unwrap();
    assert_eq!(service.revisions(voice_id.clone()).unwrap().len(), 2);
    history
        .execute("DELETE FROM transcription_history WHERE id=1", [])
        .unwrap();
    service.synchronize().unwrap();
    assert!(service.revisions(voice_id).unwrap().is_empty());
    assert_eq!(service.query(HistoryQuery::default()).unwrap().len(), 1);
    assert!(!changes.lock().unwrap().is_empty());
    drop(service);
    let restarted = HistoryService::start(root.path().into(), "fixture".into(), |_| {}).unwrap();
    restarted.synchronize().unwrap();
    assert_eq!(restarted.query(HistoryQuery::default()).unwrap().len(), 1);
}

#[test]
fn missing_sources_fail_explicitly_without_creating_empty_replacements() {
    let root = tempfile::tempdir().unwrap();
    let service = HistoryService::start(root.path().into(), "fixture".into(), |_| {}).unwrap();
    assert!(service.query(HistoryQuery::default()).is_err());
    assert!(!root.path().join("history.db").exists());
}
