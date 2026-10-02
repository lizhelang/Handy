use std::sync::{Arc, Barrier};
use std::time::Duration;

use inputia_handy_runtime::output_ledger::{
    self as ledger, OutputAction, OutputIntent, OutputLedgerError, OutputOutcome, OutputOwner,
    OutputState, RecoverySummary,
};
use rusqlite::Connection;

fn intent(operation_id: &str) -> OutputIntent {
    OutputIntent {
        operation_id: operation_id.into(),
        item_id: "12:test-store-1:record-1".into(),
        revision: 1,
        source: None,
        profile_id: None,
        deadline_at_ms: None,
        target_id: Some("target-1".into()),
        owner: OutputOwner::Platform,
        policy_epoch: 7,
        action: OutputAction::InsertText,
    }
}

fn database() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    ledger::initialize(&connection).unwrap();
    connection
}

#[test]
fn plain_text_copy_cannot_replay_native_copy_and_unknown_never_redispatches() {
    let connection = database();
    let mut request = intent("copy-format");
    request.action = OutputAction::Copy;
    ledger::prepare(&connection, &request).unwrap();
    let mut plain = request.clone();
    plain.action = OutputAction::CopyPlainText;
    assert!(matches!(
        ledger::prepare(&connection, &plain),
        Err(OutputLedgerError::ReplayConflict)
    ));
    plain.operation_id = "explicit-plain".into();
    ledger::prepare(&connection, &plain).unwrap();
    assert!(ledger::claim_dispatch(&connection, &plain).unwrap());
    ledger::finish(&connection, &plain, OutputOutcome::Uncertain).unwrap();
    assert!(!ledger::claim_dispatch(&connection, &plain).unwrap());
}

#[test]
fn expired_output_intent_cannot_claim_dispatch() {
    let connection = database();
    let mut request = intent("expired-output");
    request.deadline_at_ms = Some(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
            - 1,
    );
    ledger::prepare(&connection, &request).unwrap();
    assert!(!ledger::claim_dispatch(&connection, &request).unwrap());
    assert_eq!(
        ledger::get(&connection, &request.operation_id)
            .unwrap()
            .unwrap()
            .state,
        OutputState::Prepared
    );
}

#[test]
fn a_hundred_identical_requests_dispatch_once_and_keep_confirmation() {
    let connection = database();
    let request = intent("confirmed-100");
    let mut dispatches = 0;
    for _ in 0..100 {
        ledger::prepare(&connection, &request).unwrap();
        if ledger::claim_dispatch(&connection, &request).unwrap() {
            dispatches += 1;
            ledger::finish(&connection, &request, OutputOutcome::Confirmed).unwrap();
        }
        assert_eq!(
            ledger::finish(&connection, &request, OutputOutcome::Confirmed)
                .unwrap()
                .state,
            OutputState::Confirmed
        );
    }
    assert_eq!(dispatches, 1);
}

#[test]
fn every_payload_field_is_bound_to_the_operation_identity() {
    let connection = database();
    let request = intent("bound-fields");
    ledger::prepare(&connection, &request).unwrap();
    let mut changed = Vec::new();
    let mut variant = request.clone();
    variant.item_id.push('2');
    changed.push(variant);
    let mut variant = request.clone();
    variant.revision += 1;
    changed.push(variant);
    let mut variant = request.clone();
    variant.target_id = Some("target-2".into());
    changed.push(variant);
    let mut variant = request.clone();
    variant.target_id = None;
    changed.push(variant);
    let mut variant = request.clone();
    variant.owner = OutputOwner::Ime;
    changed.push(variant);
    let mut variant = request.clone();
    variant.policy_epoch += 1;
    changed.push(variant);
    let mut variant = request.clone();
    variant.action = OutputAction::Copy;
    changed.push(variant);
    for variant in changed {
        assert!(matches!(
            ledger::prepare(&connection, &variant),
            Err(OutputLedgerError::ReplayConflict)
        ));
        assert!(matches!(
            ledger::claim_dispatch(&connection, &variant),
            Err(OutputLedgerError::ReplayConflict)
        ));
        assert!(matches!(
            ledger::finish(&connection, &variant, OutputOutcome::Rejected),
            Err(OutputLedgerError::ReplayConflict)
        ));
    }
    assert_eq!(
        ledger::get(&connection, &request.operation_id)
            .unwrap()
            .unwrap()
            .state,
        OutputState::Prepared
    );
}

#[test]
fn lost_receipts_and_prepared_work_remain_terminal_after_database_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("output.sqlite");
    let lost = intent("lost-receipt");
    let abandoned = intent("abandoned-preparation");
    let sent = intent("dispatch-only");
    {
        let connection = Connection::open(&path).unwrap();
        ledger::initialize(&connection).unwrap();
        for request in [&lost, &abandoned, &sent] {
            ledger::prepare(&connection, request).unwrap();
        }
        assert!(ledger::claim_dispatch(&connection, &lost).unwrap());
        assert!(ledger::claim_dispatch(&connection, &sent).unwrap());
        ledger::finish(&connection, &sent, OutputOutcome::DispatchedOnly).unwrap();
        // 外部副作用可能发生，此处模拟没有写入其回执就退出。
    }
    let connection = Connection::open(&path).unwrap();
    ledger::initialize(&connection).unwrap();
    assert_eq!(
        ledger::recover_inflight(&connection).unwrap(),
        RecoverySummary {
            uncertain: 1,
            rejected: 1
        }
    );
    assert_eq!(
        ledger::recover_inflight(&connection).unwrap(),
        RecoverySummary::default()
    );
    for (request, expected) in [
        (&lost, OutputState::Uncertain),
        (&abandoned, OutputState::Rejected),
        (&sent, OutputState::DispatchedOnly),
    ] {
        for _ in 0..100 {
            assert_eq!(
                ledger::prepare(&connection, request).unwrap().state,
                expected
            );
            assert!(!ledger::claim_dispatch(&connection, request).unwrap());
        }
    }
    assert!(matches!(
        ledger::finish(&connection, &lost, OutputOutcome::Rejected),
        Err(OutputLedgerError::InvalidTransition)
    ));
}

#[test]
fn pending_target_requires_a_new_explicit_operation_and_no_owner_fallback() {
    let connection = database();
    let mut missing = intent("missing-target");
    missing.target_id = None;
    ledger::prepare(&connection, &missing).unwrap();
    assert_eq!(
        ledger::finish(&connection, &missing, OutputOutcome::PendingTarget)
            .unwrap()
            .state,
        OutputState::PendingTarget
    );
    assert!(!ledger::claim_dispatch(&connection, &missing).unwrap());
    let mut replacement = missing.clone();
    replacement.target_id = Some("fresh-target".into());
    assert!(matches!(
        ledger::prepare(&connection, &replacement),
        Err(OutputLedgerError::ReplayConflict)
    ));
    replacement.operation_id = "explicit-retry".into();
    replacement.owner = OutputOwner::Ime;
    ledger::prepare(&connection, &replacement).unwrap();
    assert!(ledger::claim_dispatch(&connection, &replacement).unwrap());
    ledger::finish(&connection, &replacement, OutputOutcome::Uncertain).unwrap();
    replacement.owner = OutputOwner::Platform;
    assert!(matches!(
        ledger::claim_dispatch(&connection, &replacement),
        Err(OutputLedgerError::ReplayConflict)
    ));
}

#[test]
fn terminal_result_cannot_hide_an_uncertain_dispatch_or_claim_target_confirmation() {
    let connection = database();
    for outcome in [
        OutputOutcome::Confirmed,
        OutputOutcome::DispatchedOnly,
        OutputOutcome::Uncertain,
    ] {
        let request = intent(&format!("finish-{outcome:?}"));
        ledger::prepare(&connection, &request).unwrap();
        assert!(matches!(
            ledger::finish(&connection, &request, outcome),
            Err(OutputLedgerError::InvalidTransition)
        ));
        assert!(ledger::claim_dispatch(&connection, &request).unwrap());
        for impossible in [OutputOutcome::PendingTarget, OutputOutcome::Rejected] {
            assert!(matches!(
                ledger::finish(&connection, &request, impossible),
                Err(OutputLedgerError::InvalidTransition)
            ));
        }
        ledger::finish(&connection, &request, outcome).unwrap();
        ledger::finish(&connection, &request, outcome).unwrap();
        assert!(!ledger::claim_dispatch(&connection, &request).unwrap());
        for conflicting in [
            OutputOutcome::Confirmed,
            OutputOutcome::DispatchedOnly,
            OutputOutcome::Uncertain,
            OutputOutcome::PendingTarget,
            OutputOutcome::Rejected,
            OutputOutcome::NotDispatchedPendingTarget,
            OutputOutcome::NotDispatchedRejected,
        ] {
            if conflicting != outcome {
                assert!(matches!(
                    ledger::finish(&connection, &request, conflicting),
                    Err(OutputLedgerError::InvalidTransition)
                ));
            }
        }
    }
}

#[test]
fn claimed_but_proven_not_dispatched_can_finish_without_authorizing_retry() {
    let connection = database();
    for (outcome, state) in [
        (
            OutputOutcome::NotDispatchedPendingTarget,
            OutputState::PendingTarget,
        ),
        (OutputOutcome::NotDispatchedRejected, OutputState::Rejected),
    ] {
        let request = intent(&format!("not-dispatched-{outcome:?}"));
        ledger::prepare(&connection, &request).unwrap();
        // 已 claim 的专用证明不能替代尚未 claim 的普通前置失败。
        assert!(matches!(
            ledger::finish(&connection, &request, outcome),
            Err(OutputLedgerError::InvalidTransition)
        ));
        assert!(ledger::claim_dispatch(&connection, &request).unwrap());
        for ordinary in [OutputOutcome::PendingTarget, OutputOutcome::Rejected] {
            assert!(matches!(
                ledger::finish(&connection, &request, ordinary),
                Err(OutputLedgerError::InvalidTransition)
            ));
        }
        // 模拟适配器最后检查目标失效，尚未启动任何剪贴板或插入动作。
        assert_eq!(
            ledger::finish(&connection, &request, outcome)
                .unwrap()
                .state,
            state
        );
        for _ in 0..100 {
            assert_eq!(
                ledger::finish(&connection, &request, outcome)
                    .unwrap()
                    .state,
                state
            );
            assert_eq!(ledger::prepare(&connection, &request).unwrap().state, state);
            assert!(!ledger::claim_dispatch(&connection, &request).unwrap());
        }
        assert_eq!(
            ledger::recover_inflight(&connection).unwrap(),
            RecoverySummary::default()
        );
        for conflicting in [
            OutputOutcome::Confirmed,
            OutputOutcome::DispatchedOnly,
            OutputOutcome::Uncertain,
        ] {
            assert!(matches!(
                ledger::finish(&connection, &request, conflicting),
                Err(OutputLedgerError::InvalidTransition)
            ));
        }
        let mut fallback = request.clone();
        fallback.owner = OutputOwner::Ime;
        assert!(matches!(
            ledger::prepare(&connection, &fallback),
            Err(OutputLedgerError::ReplayConflict)
        ));
        assert!(matches!(
            ledger::finish(&connection, &fallback, outcome),
            Err(OutputLedgerError::ReplayConflict)
        ));
    }
}

#[test]
fn missing_not_dispatched_proof_still_recovers_as_uncertain() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("missing-proof.sqlite");
    let request = intent("crashed-before-proof");
    {
        let mut connection = Connection::open(&path).unwrap();
        ledger::initialize(&connection).unwrap();
        ledger::prepare(&connection, &request).unwrap();
        assert!(ledger::claim_dispatch(&connection, &request).unwrap());
        // 即使内存中知道没派发，只要证明事务没提交，恢复时仍视为未知。
        let tx = connection.transaction().unwrap();
        ledger::finish(&tx, &request, OutputOutcome::NotDispatchedPendingTarget).unwrap();
        tx.rollback().unwrap();
    }
    let connection = Connection::open(path).unwrap();
    assert_eq!(
        ledger::recover_inflight(&connection).unwrap(),
        RecoverySummary {
            uncertain: 1,
            rejected: 0
        }
    );
    for outcome in [
        OutputOutcome::NotDispatchedPendingTarget,
        OutputOutcome::NotDispatchedRejected,
    ] {
        assert!(matches!(
            ledger::finish(&connection, &request, outcome),
            Err(OutputLedgerError::InvalidTransition)
        ));
    }
    assert!(!ledger::claim_dispatch(&connection, &request).unwrap());
}

#[test]
fn separate_sqlite_connections_racing_claim_have_one_winner() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("racing.sqlite");
    let connection = Connection::open(&path).unwrap();
    ledger::initialize(&connection).unwrap();
    let request = intent("simultaneous-request");
    ledger::prepare(&connection, &request).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let path = path.clone();
            let request = request.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                let connection = Connection::open(path).unwrap();
                connection.busy_timeout(Duration::from_secs(5)).unwrap();
                barrier.wait();
                ledger::claim_dispatch(&connection, &request).unwrap()
            })
        })
        .collect();
    let winners = threads
        .into_iter()
        .map(|thread| usize::from(thread.join().unwrap()))
        .sum::<usize>();
    assert_eq!(winners, 1);
}

#[test]
fn rollback_revokes_claim_and_never_authorizes_an_external_side_effect() {
    let mut connection = database();
    let request = intent("rollback-request");
    {
        let tx = connection.transaction().unwrap();
        ledger::prepare(&tx, &request).unwrap();
        assert!(ledger::claim_dispatch(&tx, &request).unwrap());
        tx.rollback().unwrap();
    }
    assert!(ledger::get(&connection, &request.operation_id)
        .unwrap()
        .is_none());
    ledger::prepare(&connection, &request).unwrap();
    {
        let tx = connection.transaction().unwrap();
        assert!(ledger::claim_dispatch(&tx, &request).unwrap());
        tx.rollback().unwrap();
    }
    let mut externally_dispatched = 0;
    let tx = connection.transaction().unwrap();
    let claimed = ledger::claim_dispatch(&tx, &request).unwrap();
    tx.commit().unwrap();
    if claimed {
        externally_dispatched += 1;
    }
    {
        let tx = connection.transaction().unwrap();
        ledger::finish(&tx, &request, OutputOutcome::Confirmed).unwrap();
        tx.rollback().unwrap();
    }
    assert_eq!(
        ledger::get(&connection, &request.operation_id)
            .unwrap()
            .unwrap()
            .state,
        OutputState::Dispatched
    );
    assert!(!ledger::claim_dispatch(&connection, &request).unwrap());
    assert_eq!(externally_dispatched, 1);
}

#[test]
fn invalid_intents_are_rejected_but_composite_store_identity_is_supported() {
    let connection = database();
    let mut request = intent("long-composite");
    request.item_id = format!("160:{}:160:{}", "a".repeat(160), "b".repeat(160));
    request.revision = u64::MAX;
    request.policy_epoch = u64::MAX;
    ledger::prepare(&connection, &request).unwrap();
    assert_eq!(
        ledger::get(&connection, &request.operation_id)
            .unwrap()
            .unwrap()
            .intent,
        request
    );
    let invalid = ["", "line\nbreak", "tab\tidentity"];
    for item_id in invalid {
        let mut request = intent("invalid-item");
        request.item_id = item_id.into();
        assert!(matches!(
            ledger::prepare(&connection, &request),
            Err(OutputLedgerError::InvalidIntent)
        ));
    }
    let mut request = intent("invalid-revision");
    request.revision = 0;
    assert!(matches!(
        ledger::prepare(&connection, &request),
        Err(OutputLedgerError::InvalidIntent)
    ));
    let mut request = intent("invalid-ime-asset");
    request.owner = OutputOwner::Ime;
    request.action = OutputAction::PasteAsset;
    assert!(matches!(
        ledger::prepare(&connection, &request),
        Err(OutputLedgerError::InvalidIntent)
    ));
}

#[test]
fn corrupt_persisted_digest_fails_closed_without_dispatch() {
    let connection = database();
    let request = intent("corrupt-digest");
    ledger::prepare(&connection, &request).unwrap();
    connection.execute(
        "UPDATE unified_output_operations SET intent_digest = zeroblob(32) WHERE operation_id = ?1",
        [&request.operation_id],
    ).unwrap();
    assert!(matches!(
        ledger::claim_dispatch(&connection, &request),
        Err(OutputLedgerError::CorruptRecord)
    ));
    let state: String = connection
        .query_row(
            "SELECT state FROM unified_output_operations WHERE operation_id = ?1",
            [&request.operation_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(state, "prepared");
}
