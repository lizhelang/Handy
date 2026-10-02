use std::path::Path;

use inputia_handy_runtime::output_ledger::{
    OutputAction, OutputIntent, OutputLedgerError, OutputOutcome, OutputOwner, OutputState,
};
use inputia_handy_runtime::store::{
    item_id, ContentType, IntegrationStore, ItemSnapshot, SourceChange, SourceKind,
    SourceOperation, SourceTrust, StoreError,
};
use rusqlite::Connection;

fn change(seq: u64, record: &str, revision: u64) -> SourceChange {
    SourceChange {
        store_id: "voice-store".into(),
        seq,
        event_id: format!("voice-event-{seq}"),
        record_id: record.into(),
        revision,
        operation: SourceOperation::Upsert,
        policy_epoch: 1,
        payload: Some(ItemSnapshot {
            source_kind: SourceKind::Voice,
            content_type: ContentType::Text,
            text: Some(format!("synthetic revision {revision}")),
            title: None,
            starred: false,
            pinned: false,
            created_at_ms: 100,
            asset_ref: None,
            source_app: None,
            source_trust: SourceTrust::Unknown,
        }),
    }
}

fn open(path: &Path) -> IntegrationStore {
    let mut store = IntegrationStore::open(path, "output-test-profile").unwrap();
    store.register_source("history", "voice-store").unwrap();
    store
}

fn seeded(path: &Path) -> IntegrationStore {
    let mut store = open(path);
    store.initialize_outputs().unwrap();
    store.apply_change(&change(1, "one", 1)).unwrap();
    store
}

fn intent(id: &str) -> OutputIntent {
    OutputIntent {
        operation_id: id.into(),
        item_id: item_id("voice-store", "one"),
        revision: 1,
        source: None,
        profile_id: None,
        deadline_at_ms: None,
        target_id: Some("synthetic-target".into()),
        owner: OutputOwner::Platform,
        policy_epoch: 1,
        action: OutputAction::InsertText,
    }
}

fn state(store: &IntegrationStore, request: &OutputIntent) -> OutputState {
    store
        .output_record(&request.operation_id)
        .unwrap()
        .unwrap()
        .state
}

#[test]
fn live_revision_change_prevents_claim_and_invalid_prepare_leaves_no_record() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = seeded(&temp.path().join("revision.sqlite"));
    let request = intent("prepared-before-edit");
    store.prepare_output(&request).unwrap();
    store.apply_change(&change(2, "one", 2)).unwrap();
    assert!(matches!(
        store.claim_output(&request),
        Err(StoreError::Invalid(_))
    ));
    assert_eq!(state(&store, &request), OutputState::Prepared);

    let stale = intent("stale-prepare");
    assert!(matches!(
        store.prepare_output(&stale),
        Err(StoreError::Invalid(_))
    ));
    assert!(store.output_record(&stale.operation_id).unwrap().is_none());
    let mut fresh = stale;
    fresh.revision = 2;
    store.prepare_output(&fresh).unwrap();
    assert!(store.claim_output(&fresh).unwrap());
}

#[test]
fn epoch_change_prevents_claim_and_does_not_leave_an_old_epoch_preparation() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = seeded(&temp.path().join("epoch.sqlite"));
    let request = intent("prepared-before-epoch");
    store.prepare_output(&request).unwrap();
    store.advance_policy_epoch(1, 2).unwrap();
    assert!(matches!(
        store.claim_output(&request),
        Err(StoreError::EpochMismatch {
            expected: 2,
            actual: 1
        })
    ));
    assert_eq!(state(&store, &request), OutputState::Prepared);
    let stale = intent("epoch-stale-prepare");
    assert!(matches!(
        store.prepare_output(&stale),
        Err(StoreError::EpochMismatch {
            expected: 2,
            actual: 1
        })
    ));
    assert!(store.output_record(&stale.operation_id).unwrap().is_none());
    let mut fresh = intent("epoch-fresh-prepare");
    fresh.policy_epoch = 2;
    store.prepare_output(&fresh).unwrap();
    assert!(store.claim_output(&fresh).unwrap());
}

#[test]
fn deleted_item_cannot_be_claimed_or_prepared_again() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = seeded(&temp.path().join("deleted.sqlite"));
    let request = intent("prepared-before-delete");
    store.prepare_output(&request).unwrap();
    let mut deletion = change(2, "one", 2);
    deletion.operation = SourceOperation::Delete;
    deletion.payload = None;
    store.apply_change(&deletion).unwrap();
    assert!(store.get(&request.item_id).unwrap().is_none());
    assert!(matches!(
        store.claim_output(&request),
        Err(StoreError::Invalid(_))
    ));
    assert_eq!(state(&store, &request), OutputState::Prepared);
    let retry = intent("after-delete");
    assert!(store.prepare_output(&retry).is_err());
    assert!(store.output_record(&retry.operation_id).unwrap().is_none());
}

#[test]
fn same_operation_cannot_change_owner_action_or_item() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = seeded(&temp.path().join("identity.sqlite"));
    store.apply_change(&change(2, "two", 1)).unwrap();
    let request = intent("bound-operation");
    store.prepare_output(&request).unwrap();
    let mut owner = request.clone();
    owner.owner = OutputOwner::Ime;
    let mut action = request.clone();
    action.action = OutputAction::Copy;
    let mut item = request.clone();
    item.item_id = item_id("voice-store", "two");
    for changed in [owner, action, item] {
        assert!(matches!(
            store.prepare_output(&changed),
            Err(StoreError::Output(OutputLedgerError::ReplayConflict))
        ));
        assert!(matches!(
            store.claim_output(&changed),
            Err(StoreError::Output(OutputLedgerError::ReplayConflict))
        ));
        assert!(matches!(
            store.finish_output(&changed, OutputOutcome::Rejected),
            Err(StoreError::Output(OutputLedgerError::ReplayConflict))
        ));
        assert_eq!(
            store
                .output_record(&request.operation_id)
                .unwrap()
                .unwrap()
                .intent,
            request
        );
        assert_eq!(state(&store, &request), OutputState::Prepared);
    }
}

#[test]
fn startup_recovery_makes_abandoned_operations_terminal_and_preserves_receipts() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("restart.sqlite");
    let claimed = intent("claimed-at-crash");
    let prepared = intent("prepared-at-crash");
    let confirmed = intent("confirmed-before-crash");
    {
        let mut store = seeded(&path);
        for request in [&claimed, &prepared, &confirmed] {
            store.prepare_output(request).unwrap();
        }
        assert!(store.claim_output(&claimed).unwrap());
        assert!(store.claim_output(&confirmed).unwrap());
        store
            .finish_output(&confirmed, OutputOutcome::Confirmed)
            .unwrap();
    }
    let mut store = open(&path);
    store.initialize_outputs().unwrap();
    store.initialize_outputs().unwrap();
    for (request, expected) in [
        (&claimed, OutputState::Uncertain),
        (&prepared, OutputState::Rejected),
        (&confirmed, OutputState::Confirmed),
    ] {
        assert_eq!(state(&store, request), expected);
        assert_eq!(store.prepare_output(request).unwrap().state, expected);
        assert!(!store.claim_output(request).unwrap());
    }
    assert!(store
        .finish_output(&claimed, OutputOutcome::NotDispatchedRejected)
        .is_err());
    // 历史随后清理或策略变化，也不能将已知回执丢掉或重置成可执行状态。
    store.advance_policy_epoch(1, 2).unwrap();
    assert_eq!(
        store.prepare_output(&confirmed).unwrap().state,
        OutputState::Confirmed
    );
    assert_eq!(state(&store, &claimed), OutputState::Uncertain);
}

#[test]
fn sqlite_failure_rolls_back_prepare_claim_and_receipt_transactions() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("faults.sqlite");
    let mut store = seeded(&path);
    let fault_connection = Connection::open(&path).unwrap();
    let request = intent("failure-injection");
    // AFTER trigger 让底层语句先尝试修改，再产生错误，验证整个事务不泄漏部分状态。
    fault_connection.execute_batch("CREATE TRIGGER fail_prepare AFTER INSERT ON unified_output_operations BEGIN SELECT RAISE(ABORT, 'synthetic prepare fault'); END;").unwrap();
    assert!(store.prepare_output(&request).is_err());
    assert!(store
        .output_record(&request.operation_id)
        .unwrap()
        .is_none());
    fault_connection
        .execute_batch("DROP TRIGGER fail_prepare;")
        .unwrap();
    store.prepare_output(&request).unwrap();

    fault_connection.execute_batch("CREATE TRIGGER fail_claim AFTER UPDATE OF state ON unified_output_operations WHEN NEW.state='dispatched' BEGIN SELECT RAISE(ABORT, 'synthetic claim fault'); END;").unwrap();
    assert!(store.claim_output(&request).is_err());
    assert_eq!(state(&store, &request), OutputState::Prepared);
    fault_connection
        .execute_batch("DROP TRIGGER fail_claim;")
        .unwrap();
    assert!(store.claim_output(&request).unwrap());

    fault_connection.execute_batch("CREATE TRIGGER fail_receipt AFTER UPDATE OF state ON unified_output_operations WHEN NEW.state='confirmed' BEGIN SELECT RAISE(ABORT, 'synthetic receipt fault'); END;").unwrap();
    assert!(store
        .finish_output(&request, OutputOutcome::Confirmed)
        .is_err());
    assert_eq!(state(&store, &request), OutputState::Dispatched);
    assert!(!store.claim_output(&request).unwrap());
    fault_connection
        .execute_batch("DROP TRIGGER fail_receipt;")
        .unwrap();
    store
        .finish_output(&request, OutputOutcome::Uncertain)
        .unwrap();
    assert_eq!(state(&store, &request), OutputState::Uncertain);
}

#[test]
fn duplicate_requests_across_connections_never_grant_a_second_dispatch() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("duplicate.sqlite");
    let mut first = seeded(&path);
    // 第二连接不是新服务启动，不调用恢复；生产环境还由服务级独占锁限制 writer。
    let mut second = open(&path);
    let request = intent("repeated-output");
    let mut dispatch_count = 0;
    for index in 0..100 {
        let store = if index % 2 == 0 {
            &mut first
        } else {
            &mut second
        };
        store.prepare_output(&request).unwrap();
        if store.claim_output(&request).unwrap() {
            dispatch_count += 1;
            store
                .finish_output(&request, OutputOutcome::DispatchedOnly)
                .unwrap();
        }
    }
    assert_eq!(dispatch_count, 1);
    assert_eq!(state(&first, &request), OutputState::DispatchedOnly);
    assert_eq!(state(&second, &request), OutputState::DispatchedOnly);
}
