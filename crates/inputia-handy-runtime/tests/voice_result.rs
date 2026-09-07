use inputia_handy_runtime::{
    output_ledger::{OutputOwner, OutputState},
    store::*,
    voice_protocol::*,
};
use rusqlite::Connection;

fn start() -> VoiceRequest {
    VoiceRequest {
        request_id: "start".into(),
        session_id: "session".into(),
        server_instance: "server".into(),
        client_instance: "host".into(),
        policy_epoch: 1,
        command: VoiceCommand::Start {
            target: HostTargetToken {
                target_id: "target".into(),
                host_instance: "host".into(),
                controller_id: "controller".into(),
                activation_generation: 1,
                field_id: Some("field".into()),
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
    }
}
fn change(seq: u64, record: &str, revision: u64, kind: SourceKind) -> SourceChange {
    SourceChange {
        store_id: "voice-store".into(),
        seq,
        event_id: format!("event-{seq}"),
        record_id: record.into(),
        revision,
        operation: SourceOperation::Upsert,
        policy_epoch: 1,
        payload: Some(ItemSnapshot {
            source_kind: kind,
            content_type: ContentType::Text,
            text: Some("synthetic result".into()),
            title: None,
            starred: false,
            pinned: false,
            created_at_ms: 1,
            asset_ref: None,
            source_app: None,
            source_trust: SourceTrust::Unknown,
        }),
    }
}
fn seeded(path: &std::path::Path) -> IntegrationStore {
    let mut store = IntegrationStore::open(path, "test").unwrap();
    store.register_source("history", "voice-store").unwrap();
    store.initialize_outputs().unwrap();
    store.initialize_voice_sessions().unwrap();
    store
        .apply_change(&change(1, "one", 1, SourceKind::Voice))
        .unwrap();
    let request = start();
    store
        .prepare_voice_request(&request, "host", "server", Some(1))
        .unwrap();
    assert!(store
        .claim_voice_request(&request, "host", "server", Some(1))
        .unwrap());
    store
}

#[test]
fn repeated_result_has_one_ime_owner_and_cannot_change_item_or_revision() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = seeded(&temp.path().join("integration.db"));
    let id = item_id("voice-store", "one");
    let first = store
        .prepare_voice_result("session", "host", "server", &id, 1)
        .unwrap();
    assert_eq!(first.intent.owner, OutputOwner::Ime);
    assert_eq!(first.intent.target_id.as_deref(), Some("target"));
    assert_eq!(first.state, OutputState::Prepared);
    for _ in 0..100 {
        assert_eq!(
            store
                .prepare_voice_result("session", "host", "server", &id, 1)
                .unwrap(),
            first
        );
    }
    store
        .apply_change(&change(2, "two", 1, SourceKind::Voice))
        .unwrap();
    assert!(store
        .prepare_voice_result(
            "session",
            "host",
            "server",
            &item_id("voice-store", "two"),
            1
        )
        .is_err());
    store
        .apply_change(&change(3, "one", 2, SourceKind::Voice))
        .unwrap();
    assert!(store
        .prepare_voice_result("session", "host", "server", &id, 2)
        .is_err());
    assert_eq!(store.voice_result("session").unwrap().unwrap(), first);
    assert!(store.claim_output(&first.intent).is_err());
}

#[test]
fn result_and_output_prepare_roll_back_together_when_association_write_fails() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("integration.db");
    let mut store = seeded(&path);
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("CREATE TRIGGER reject_result BEFORE INSERT ON unified_voice_results BEGIN SELECT RAISE(ABORT,'fixture');END;").unwrap();
    assert!(store
        .prepare_voice_result(
            "session",
            "host",
            "server",
            &item_id("voice-store", "one"),
            1
        )
        .is_err());
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM unified_output_operations",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert!(store.voice_result("session").unwrap().is_none());
    conn.execute_batch("DROP TRIGGER reject_result").unwrap();
    assert!(store
        .prepare_voice_result(
            "session",
            "host",
            "server",
            &item_id("voice-store", "one"),
            1
        )
        .is_ok());
}

#[test]
fn changed_peer_epoch_cancelled_session_and_nonvoice_source_are_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = seeded(&temp.path().join("integration.db"));
    let id = item_id("voice-store", "one");
    assert!(store
        .prepare_voice_result("session", "other", "server", &id, 1)
        .is_err());
    assert!(store
        .prepare_voice_result("session", "host", "other", &id, 1)
        .is_err());
    store
        .apply_change(&change(2, "copy", 1, SourceKind::Clipboard))
        .unwrap();
    assert!(store
        .prepare_voice_result(
            "session",
            "host",
            "server",
            &item_id("voice-store", "copy"),
            1
        )
        .is_err());
    store.advance_policy_epoch(1, 2).unwrap();
    assert!(store
        .prepare_voice_result("session", "host", "server", &id, 1)
        .is_err());
    let mut view = store.voice_session("session").unwrap().unwrap().view;
    view.phase = VoicePhase::Cancelled;
    view.generation = 1;
    store
        .project_voice_session("host", "server", &view)
        .unwrap();
    assert!(store
        .prepare_voice_result("session", "host", "server", &id, 1)
        .is_err());
    assert!(store.voice_result("session").unwrap().is_none());
}

#[test]
fn restart_preserves_result_reference_but_never_grants_second_dispatch() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("integration.db");
    let mut store = seeded(&path);
    let result = store
        .prepare_voice_result(
            "session",
            "host",
            "server",
            &item_id("voice-store", "one"),
            1,
        )
        .unwrap();
    assert!(store.claim_output(&result.intent).unwrap());
    drop(store);
    let mut store = IntegrationStore::open(&path, "test").unwrap();
    store.initialize_outputs().unwrap();
    store.initialize_voice_sessions().unwrap();
    let recovered = store.voice_result("session").unwrap().unwrap();
    assert_eq!(recovered.state, OutputState::Uncertain);
    assert!(!store.claim_output(&result.intent).unwrap());
    assert!(store
        .prepare_voice_result("session", "host", "server", &result.intent.item_id, 1)
        .is_err());
}

#[test]
fn unclaimed_start_and_deleted_content_cannot_create_result_or_output() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("integration.db");
    let mut store = seeded(&path);
    let mut request = start();
    request.session_id = "not-started".into();
    request.request_id = "not-claimed".into();
    store
        .prepare_voice_request(&request, "host", "server", Some(1))
        .unwrap();
    let id = item_id("voice-store", "one");
    assert!(store
        .prepare_voice_result("not-started", "host", "server", &id, 1)
        .is_err());
    let mut deletion = change(2, "one", 2, SourceKind::Voice);
    deletion.operation = SourceOperation::Delete;
    deletion.payload = None;
    store.apply_change(&deletion).unwrap();
    assert!(store
        .prepare_voice_result("session", "host", "server", &id, 1)
        .is_err());
    assert!(store.voice_result("session").unwrap().is_none());
    let conn = Connection::open(path).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM unified_output_operations",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}

#[test]
fn saved_source_bridge_uses_exact_record_and_text_not_latest_entry() {
    use inputia_handy_runtime::service::HistoryService;
    let temp = tempfile::tempdir().unwrap();
    let history = Connection::open(temp.path().join("history.db")).unwrap();
    history.execute_batch("CREATE TABLE transcription_history(id INTEGER PRIMARY KEY,file_name TEXT,timestamp INTEGER,saved INTEGER,title TEXT,transcription_text TEXT,post_processed_text TEXT);
        INSERT INTO transcription_history VALUES(1,'',1,0,'fixture','original','expected');
        INSERT INTO transcription_history VALUES(2,'',2,0,'newer','other',NULL);").unwrap();
    Connection::open(temp.path().join("clipboard.db")).unwrap().execute_batch("CREATE TABLE clipboard_history(id INTEGER PRIMARY KEY,content_type TEXT,full_text TEXT,title TEXT,is_favorite INTEGER,is_pinned INTEGER,created_at INTEGER,image_path TEXT,source_app TEXT)").unwrap();
    let service = HistoryService::start(temp.path().into(), "test".into(), |_| {}).unwrap();
    let request = start();
    service
        .prepare_voice_request(request.clone(), "host".into(), "server".into(), Some(1))
        .unwrap();
    assert!(service
        .claim_voice_request(request.clone(), "host".into(), "server".into(), Some(1))
        .unwrap());
    assert!(service
        .prepare_saved_voice_result(request.clone(), 1, "wrong".into())
        .is_err());
    assert!(service
        .prepare_saved_voice_result(request.clone(), 2, "expected".into())
        .is_err());
    assert!(service.voice_result("session".into()).unwrap().is_none());
    let result = service
        .prepare_saved_voice_result(request.clone(), 1, "expected".into())
        .unwrap();
    assert!(result.intent.item_id.ends_with("1:1"));
    history
        .execute(
            "UPDATE transcription_history SET post_processed_text='changed' WHERE id=1",
            [],
        )
        .unwrap();
    assert!(service
        .prepare_saved_voice_result(request, 1, "expected".into())
        .is_err());
    assert_eq!(
        service
            .voice_result("session".into())
            .unwrap()
            .unwrap()
            .intent,
        result.intent
    );
}
