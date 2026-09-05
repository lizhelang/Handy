use inputia_handy_runtime::{voice_ledger as ledger, voice_protocol::*};
use rusqlite::{Connection, TransactionBehavior};

fn request(id: &str) -> VoiceRequest {
    VoiceRequest {
        request_id: id.into(),
        session_id: "voice-1".into(),
        client_instance: "host".into(),
        server_instance: "server".into(),
        policy_epoch: 1,
        command: VoiceCommand::Start {
            target: HostTargetToken {
                target_id: "target".into(),
                host_instance: "host".into(),
                controller_id: "controller".into(),
                activation_generation: 1,
                field_id: Some("field".into()),
                selection_generation: 1,
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
fn peer(server: &str) -> VoicePeer<'_> {
    VoicePeer {
        server_instance: server,
        client_instance: "host",
        policy_epoch: 1,
        policy_applied: true,
    }
}
fn connection() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    ledger::initialize(&conn).unwrap();
    conn
}

#[test]
fn one_hundred_request_ids_for_same_start_grant_one_committed_start() {
    let mut conn = connection();
    let mut starts = 0;
    for index in 0..100 {
        let request = request(&format!("request-{index}"));
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        ledger::prepare(&tx, &request, &peer("server")).unwrap();
        let claimed = ledger::claim(&tx, &request).unwrap();
        tx.commit().unwrap();
        starts += usize::from(claimed);
    }
    assert_eq!(starts, 1);
    let mut conflict = request("different-target");
    if let VoiceCommand::Start { target, .. } = &mut conflict.command {
        target.target_id = "other".into();
    }
    assert!(ledger::prepare(&conn, &conflict, &peer("server")).is_err());
    assert_eq!(
        ledger::get(&conn, "voice-1").unwrap().unwrap().view.phase,
        VoicePhase::Preparing
    );
}

#[test]
fn request_identity_and_stop_owner_are_enforced() {
    let conn = connection();
    let start = request("first");
    ledger::prepare(&conn, &start, &peer("server")).unwrap();
    ledger::claim(&conn, &start).unwrap();
    let mut stop = VoiceRequest {
        request_id: "first".into(),
        command: VoiceCommand::Stop,
        ..start
    };
    assert!(ledger::prepare(&conn, &stop, &peer("server")).is_err());
    stop.request_id = "stop".into();
    stop.client_instance = "intruder".into();
    assert!(ledger::prepare(
        &conn,
        &stop,
        &VoicePeer {
            client_instance: "intruder",
            ..peer("server")
        }
    )
    .is_err());
    stop.client_instance = "host".into();
    ledger::prepare(&conn, &stop, &peer("server")).unwrap();
    assert!(ledger::claim(&conn, &stop).unwrap());
    assert!(!ledger::claim(&conn, &stop).unwrap());
}

#[test]
fn crash_retires_sessions_without_restarting_or_losing_pending_output_facts() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("voice.db");
    {
        let mut conn = Connection::open(&path).unwrap();
        ledger::initialize(&conn).unwrap();
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let request = request("start");
        ledger::prepare(&tx, &request, &peer("server")).unwrap();
        assert!(ledger::claim(&tx, &request).unwrap());
        tx.commit().unwrap();
    }
    let mut conn = Connection::open(path).unwrap();
    let tx = conn.transaction().unwrap();
    assert_eq!(ledger::recover(&tx).unwrap(), 1);
    tx.commit().unwrap();
    let record = ledger::get(&conn, "voice-1").unwrap().unwrap();
    assert!(record.retired);
    assert_eq!(record.view.phase, VoicePhase::Interrupted);
    assert!(!ledger::claim(&conn, &request("start")).unwrap());
    let newer_start = VoiceRequest {
        server_instance: "restarted".into(),
        ..request("new-start")
    };
    assert!(ledger::prepare(&conn, &newer_start, &peer("restarted")).is_err());
    let status = VoiceRequest {
        command: VoiceCommand::Status,
        ..newer_start
    };
    assert_eq!(
        ledger::prepare(&conn, &status, &peer("restarted"))
            .unwrap()
            .view
            .phase,
        VoicePhase::Interrupted
    );
    assert!(!ledger::claim(&conn, &status).unwrap());
    let mut stale = record.view;
    stale.generation += 1;
    stale.phase = VoicePhase::Recording;
    assert!(ledger::project(&conn, "host", "server", &stale).is_err());
}

#[test]
fn projection_is_monotonic_and_cannot_forge_readiness_before_claim() {
    let conn = connection();
    let request = request("start");
    let record = ledger::prepare(&conn, &request, &peer("server")).unwrap();
    let mut view = record.view;
    view.generation = 1;
    view.phase = VoicePhase::Recording;
    assert!(ledger::project(&conn, "host", "server", &view).is_err());
    ledger::claim(&conn, &request).unwrap();
    assert!(ledger::project(&conn, "host", "server", &view).unwrap());
    assert!(!ledger::project(&conn, "host", "server", &view).unwrap());
    view.phase = VoicePhase::Confirmed;
    assert!(ledger::project(&conn, "host", "server", &view).is_err());
    view.generation = 2;
    view.phase = VoicePhase::PendingTarget;
    view.item_id = Some("voice-store:1".into());
    assert!(ledger::project(&conn, "host", "server", &view).unwrap());
    ledger::recover(&conn).unwrap();
    assert_eq!(ledger::get(&conn, "voice-1").unwrap().unwrap().view, view);
}

#[test]
fn transaction_failure_and_uncommitted_claim_leave_no_execution_permission() {
    let mut conn = connection();
    conn.execute_batch("CREATE TEMP TRIGGER fail_request AFTER INSERT ON unified_voice_requests BEGIN SELECT RAISE(ABORT,'fixture');END;").unwrap();
    {
        let tx = conn.transaction().unwrap();
        assert!(ledger::prepare(&tx, &request("start"), &peer("server")).is_err());
    }
    assert!(ledger::get(&conn, "voice-1").unwrap().is_none());
    conn.execute_batch("DROP TRIGGER fail_request;").unwrap();
    {
        let tx = conn.transaction().unwrap();
        ledger::prepare(&tx, &request("start"), &peer("server")).unwrap();
        tx.commit().unwrap();
    }
    {
        let tx = conn.transaction().unwrap();
        assert!(ledger::claim(&tx, &request("start")).unwrap());
    }
    assert!(ledger::claim(&conn, &request("start")).unwrap());
}

#[test]
fn policy_withdrawal_blocks_new_start_not_owned_stop_and_corruption_fails_closed() {
    let conn = connection();
    let request = request("start");
    assert!(ledger::prepare(
        &conn,
        &request,
        &VoicePeer {
            policy_applied: false,
            ..peer("server")
        }
    )
    .is_err());
    ledger::prepare(&conn, &request, &peer("server")).unwrap();
    let stop = VoiceRequest {
        request_id: "stop".into(),
        command: VoiceCommand::Stop,
        ..request
    };
    assert!(ledger::prepare(
        &conn,
        &stop,
        &VoicePeer {
            policy_epoch: 2,
            policy_applied: false,
            ..peer("server")
        }
    )
    .is_ok());
    conn.execute("UPDATE unified_voice_sessions SET start_digest='wrong'", [])
        .unwrap();
    assert!(ledger::get(&conn, "voice-1").is_err());
}
