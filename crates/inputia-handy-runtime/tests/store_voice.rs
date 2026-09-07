use inputia_handy_runtime::{store::IntegrationStore, voice_protocol::*};
use rusqlite::Connection;

fn request() -> VoiceRequest {
    VoiceRequest {
        request_id: "start".into(),
        session_id: "voice".into(),
        server_instance: "server".into(),
        client_instance: "host".into(),
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
    }
}

#[test]
fn claim_rechecks_database_epoch_and_applied_barrier() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = IntegrationStore::open(temp.path().join("integration.db"), "fixture").unwrap();
    store.initialize_voice_sessions().unwrap();
    let request = request();
    store
        .prepare_voice_request(&request, "host", "server", Some(1))
        .unwrap();
    for epoch in [None, Some(0), Some(2)] {
        assert!(store
            .claim_voice_request(&request, "host", "server", epoch)
            .is_err());
    }
    store.advance_policy_epoch(1, 2).unwrap();
    assert!(store
        .claim_voice_request(&request, "host", "server", Some(1))
        .is_err());
    assert!(store
        .claim_voice_request(&request, "host", "server", Some(2))
        .is_err());
    assert_eq!(
        store.voice_session("voice").unwrap().unwrap().view.phase,
        VoicePhase::Preparing
    );
}

#[test]
fn changed_authenticated_peer_does_not_consume_claim() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = IntegrationStore::open(temp.path().join("integration.db"), "fixture").unwrap();
    store.initialize_voice_sessions().unwrap();
    let request = request();
    store
        .prepare_voice_request(&request, "host", "server", Some(1))
        .unwrap();
    assert!(store
        .claim_voice_request(&request, "other-host", "server", Some(1))
        .is_err());
    assert!(store
        .claim_voice_request(&request, "host", "other-server", Some(1))
        .is_err());
    assert!(store
        .claim_voice_request(&request, "host", "server", Some(1))
        .unwrap());
    assert!(!store
        .claim_voice_request(&request, "host", "server", Some(1))
        .unwrap());
}

#[test]
fn failed_second_statement_rolls_back_session_start_claim() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("integration.db");
    let mut store = IntegrationStore::open(&path, "fixture").unwrap();
    store.initialize_voice_sessions().unwrap();
    let request = request();
    store
        .prepare_voice_request(&request, "host", "server", Some(1))
        .unwrap();
    let observer = Connection::open(&path).unwrap();
    observer.execute_batch("CREATE TRIGGER fail_claim AFTER UPDATE ON unified_voice_requests BEGIN SELECT RAISE(ABORT,'fixture');END;").unwrap();
    assert!(store
        .claim_voice_request(&request, "host", "server", Some(1))
        .is_err());
    assert_eq!(
        observer
            .query_row(
                "SELECT start_claimed FROM unified_voice_sessions",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    observer.execute_batch("DROP TRIGGER fail_claim").unwrap();
    assert!(store
        .claim_voice_request(&request, "host", "server", Some(1))
        .unwrap());
}

#[test]
fn native_peer_binding_survives_server_restart_and_rejects_instance_takeover() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("integration.db");
    let mut store = IntegrationStore::open(&path, "fixture").unwrap();
    store.initialize_voice_sessions().unwrap();
    store.bind_voice_peer("host-instance", &[1; 32]).unwrap();
    for _ in 0..100 {
        store.bind_voice_peer("host-instance", &[1; 32]).unwrap();
    }
    assert!(store.bind_voice_peer("host-instance", &[2; 32]).is_err());
    assert!(store.bind_voice_peer("other", &[0; 32]).is_err());
    drop(store);
    let mut store = IntegrationStore::open(&path, "fixture").unwrap();
    store.initialize_voice_sessions().unwrap();
    assert!(store.bind_voice_peer("host-instance", &[2; 32]).is_err());
    store.bind_voice_peer("host-instance", &[1; 32]).unwrap();
    store
        .bind_voice_peer("new-host-instance", &[2; 32])
        .unwrap();
    let conn = Connection::open(path).unwrap();
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM unified_voice_peers", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn peer_binding_database_failure_does_not_authorize_or_consume_identity() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("integration.db");
    let mut store = IntegrationStore::open(&path, "fixture").unwrap();
    store.initialize_voice_sessions().unwrap();
    let conn = Connection::open(path).unwrap();
    conn.execute_batch("CREATE TRIGGER deny_peer AFTER INSERT ON unified_voice_peers BEGIN SELECT RAISE(ABORT,'fixture');END;").unwrap();
    assert!(store.bind_voice_peer("host", &[1; 32]).is_err());
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM unified_voice_peers", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    conn.execute_batch("DROP TRIGGER deny_peer").unwrap();
    store.bind_voice_peer("host", &[2; 32]).unwrap();
    assert!(store.bind_voice_peer("host", &[1; 32]).is_err());
}
