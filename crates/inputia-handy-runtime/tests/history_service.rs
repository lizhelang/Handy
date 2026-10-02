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
    let contender = HistoryService::start(root.path().into(), "fixture".into(), |_| {}).unwrap();
    assert!(contender.query(HistoryQuery::default()).is_err());
    drop(contender);
    let items = service.query(HistoryQuery::default()).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].snapshot.source_kind, SourceKind::Clipboard);
    assert_eq!(items[1].snapshot.source_kind, SourceKind::Voice);
    let voice_id = items[1].item_id.clone();
    let intent = inputia_handy_runtime::output_ledger::OutputIntent {
        operation_id: "insertion-permit".into(),
        item_id: voice_id.clone(),
        revision: 1,
        source: None,
        profile_id: None,
        deadline_at_ms: None,
        target_id: Some("target-fixture".into()),
        owner: inputia_handy_runtime::output_ledger::OutputOwner::Platform,
        policy_epoch: service.policy_epoch().unwrap(),
        action: inputia_handy_runtime::output_ledger::OutputAction::InsertText,
    };
    service.prepare_output(intent.clone()).unwrap();
    let permit = service
        .claim_output_with_permit(intent.clone())
        .unwrap()
        .unwrap();
    assert!(permit.check().is_ok());
    assert!(service
        .claim_output_with_permit(intent.clone())
        .unwrap()
        .is_none());
    service.synchronize().unwrap();
    assert!(permit.check().is_ok(), "空同步不得撤销有效许可");
    let patch = inputia_handy_runtime::source::HistoryPatch {
        starred: Some(true),
        pinned: Some(true),
        title: Some("命名语音".into()),
        ..Default::default()
    };
    let revision = service
        .update_item(
            voice_id.clone(),
            1,
            "metadata-operation".into(),
            patch.clone(),
        )
        .unwrap();
    assert_eq!(revision, 2);
    assert!(permit.check().is_err(), "修订写入前必须撤销旧输出许可");
    assert!(service.claim_output_with_permit(intent.clone()).is_err());
    assert_eq!(
        service
            .update_item(voice_id.clone(), 1, "metadata-operation".into(), patch)
            .unwrap(),
        2
    );
    assert!(service
        .update_item(
            voice_id.clone(),
            1,
            "stale-operation".into(),
            inputia_handy_runtime::source::HistoryPatch {
                title: Some("过时覆盖".into()),
                ..Default::default()
            }
        )
        .is_err());
    let current = service.get_item(voice_id.clone(), 2).unwrap();
    assert!(current.snapshot.pinned && current.snapshot.starred);
    assert_eq!(current.snapshot.title.as_deref(), Some("命名语音"));
    history
        .execute(
            "UPDATE transcription_history SET post_processed_text='修订文本',saved=1 WHERE id=1",
            [],
        )
        .unwrap();
    service.synchronize().unwrap();
    assert_eq!(service.revisions(voice_id.clone()).unwrap().len(), 2);
    let current = service
        .query(HistoryQuery::default())
        .unwrap()
        .into_iter()
        .find(|item| item.item_id == voice_id)
        .unwrap();
    let mut deleting_intent = intent.clone();
    deleting_intent.operation_id = "before-source-delete".into();
    deleting_intent.revision = current.revision;
    service.prepare_output(deleting_intent.clone()).unwrap();
    let deleting_permit = service
        .claim_output_with_permit(deleting_intent.clone())
        .unwrap()
        .unwrap();
    let source_write = service.begin_source_write();
    assert!(
        deleting_permit.check().is_err(),
        "源写入前立即撤销，不等同步泵"
    );
    deleting_intent.operation_id = "during-source-delete".into();
    service.prepare_output(deleting_intent.clone()).unwrap();
    assert!(
        service
            .claim_output_with_permit(deleting_intent.clone())
            .is_err(),
        "源事务期间禁止签发新许可"
    );
    history
        .execute("DELETE FROM transcription_history WHERE id=1", [])
        .unwrap();
    drop(source_write);
    assert!(
        service.claim_output_with_permit(deleting_intent).is_err(),
        "写入完成后必须先同步删除再验证claim"
    );
    service.synchronize().unwrap();
    assert!(service.revisions(voice_id).unwrap().is_empty());
    assert_eq!(service.query(HistoryQuery::default()).unwrap().len(), 1);
    assert!(!changes.lock().unwrap().is_empty());
    drop(service);
    let restarted = HistoryService::start(root.path().into(), "fixture".into(), |_| {}).unwrap();
    restarted.synchronize().unwrap();
    assert_eq!(
        restarted
            .output_record(intent.operation_id)
            .unwrap()
            .unwrap()
            .state,
        inputia_handy_runtime::output_ledger::OutputState::Uncertain
    );
    assert_eq!(restarted.query(HistoryQuery::default()).unwrap().len(), 1);
    let remaining = restarted.query(HistoryQuery::default()).unwrap().remove(0);
    assert!(restarted
        .delete_item(
            remaining.item_id.clone(),
            remaining.revision + 1,
            "stale-delete".into()
        )
        .is_err());
    assert!(restarted
        .delete_item(
            remaining.item_id.clone(),
            remaining.revision,
            "delete-final".into()
        )
        .unwrap());
    assert!(restarted
        .delete_item(
            remaining.item_id.clone(),
            remaining.revision,
            "delete-final".into()
        )
        .unwrap());
    assert!(restarted
        .delete_item(
            "foreign-item".into(),
            remaining.revision,
            "delete-final".into()
        )
        .is_err());
    assert!(restarted.query(HistoryQuery::default()).unwrap().is_empty());
}

#[test]
fn missing_sources_fail_explicitly_without_creating_empty_replacements() {
    let root = tempfile::tempdir().unwrap();
    let service = HistoryService::start(root.path().into(), "fixture".into(), |_| {}).unwrap();
    assert!(service.query(HistoryQuery::default()).is_err());
    assert!(!root.path().join("history.db").exists());
}
