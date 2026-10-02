use inputia_handy_runtime::output_ledger::{
    self as ledger, OutputAction, OutputIntent, OutputOutcome, OutputOwner, OutputState,
};
use rusqlite::Connection;

fn intent(id: &str) -> OutputIntent {
    OutputIntent {
        operation_id: id.into(),
        item_id: "12:test-store-1:record-1".into(),
        revision: 1,
        source: None,
        profile_id: None,
        deadline_at_ms: None,
        target_id: Some("private-original-field".into()),
        owner: OutputOwner::Platform,
        policy_epoch: 1,
        action: OutputAction::InsertText,
    }
}

fn record(connection: &Connection, id: &str, outcome: Option<OutputOutcome>) -> OutputIntent {
    let intent = intent(id);
    ledger::prepare(connection, &intent).unwrap();
    if let Some(outcome) = outcome {
        if !matches!(
            outcome,
            OutputOutcome::PendingTarget | OutputOutcome::Rejected
        ) {
            assert!(ledger::claim_dispatch(connection, &intent).unwrap());
        }
        ledger::finish(connection, &intent, outcome).unwrap();
    }
    intent
}

#[test]
fn query_is_read_only_bounded_paginated_and_returns_only_safe_unresolved_summaries() {
    let connection = Connection::open_in_memory().unwrap();
    ledger::initialize(&connection).unwrap();
    record(&connection, "00-confirmed", Some(OutputOutcome::Confirmed));
    record(
        &connection,
        "01-dispatched",
        Some(OutputOutcome::DispatchedOnly),
    );
    record(&connection, "02-prepared", None);
    let inflight = record(&connection, "03-inflight", None);
    ledger::claim_dispatch(&connection, &inflight).unwrap();
    for (id, outcome) in [
        ("04-pending", OutputOutcome::PendingTarget),
        ("05-uncertain", OutputOutcome::Uncertain),
        ("06-rejected", OutputOutcome::Rejected),
    ] {
        record(&connection, id, Some(outcome));
    }
    let changes_before = connection.total_changes();
    let page = ledger::list_unresolved_notices(&connection, None, 2).unwrap();
    assert_eq!(
        page.items
            .iter()
            .map(|item| item.operation_id.as_str())
            .collect::<Vec<_>>(),
        vec!["04-pending", "05-uncertain"]
    );
    assert_eq!(page.next_cursor.as_deref(), Some("05-uncertain"));
    let second =
        ledger::list_unresolved_notices(&connection, page.next_cursor.as_deref(), 2).unwrap();
    assert_eq!(second.items.len(), 1);
    assert_eq!(second.items[0].state, OutputState::Rejected);
    assert!(second.next_cursor.is_none());
    assert_eq!(connection.total_changes(), changes_before);
    let encoded = serde_json::to_value(&page).unwrap();
    let item = encoded["items"][0].as_object().unwrap();
    assert_eq!(item.len(), 3);
    assert!(
        item.contains_key("operation_id")
            && item.contains_key("item_id")
            && item.contains_key("state")
    );
    assert!(!encoded.to_string().contains("private-original-field"));
    for limit in [0, ledger::MAX_NOTICE_PAGE_SIZE + 1] {
        assert!(ledger::list_unresolved_notices(&connection, None, limit).is_err());
    }
    assert!(ledger::list_unresolved_notices(&connection, Some("invalid\ncursor"), 1).is_err());
}

#[test]
fn notice_acknowledgment_is_durable_and_cannot_delete_or_reauthorize_unknown_output() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("ledger.sqlite");
    let request;
    {
        let mut connection = Connection::open(&path).unwrap();
        ledger::initialize(&connection).unwrap();
        request = record(&connection, "unknown-once", Some(OutputOutcome::Uncertain));
        let transaction = connection.transaction().unwrap();
        ledger::acknowledge_notice(&transaction, &request.operation_id, OutputState::Uncertain)
            .unwrap();
        transaction.commit().unwrap();
        ledger::acknowledge_notice(&connection, &request.operation_id, OutputState::Uncertain)
            .unwrap();
        assert!(ledger::list_unresolved_notices(&connection, None, 20)
            .unwrap()
            .items
            .is_empty());
        assert_eq!(
            ledger::get(&connection, &request.operation_id)
                .unwrap()
                .unwrap()
                .state,
            OutputState::Uncertain
        );
        assert!(!ledger::claim_dispatch(&connection, &request).unwrap());
    }
    let connection = Connection::open(&path).unwrap();
    ledger::initialize(&connection).unwrap();
    ledger::recover_inflight(&connection).unwrap();
    assert!(ledger::list_unresolved_notices(&connection, None, 20)
        .unwrap()
        .items
        .is_empty());
    assert_eq!(
        ledger::prepare(&connection, &request).unwrap().state,
        OutputState::Uncertain
    );
    assert!(!ledger::claim_dispatch(&connection, &request).unwrap());
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM unified_output_operations",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}

#[test]
fn acknowledgment_rejects_stale_or_nonterminal_state_and_rolls_back_without_hiding_notice() {
    let mut connection = Connection::open_in_memory().unwrap();
    ledger::initialize(&connection).unwrap();
    let request = record(&connection, "pending", Some(OutputOutcome::PendingTarget));
    for state in [
        OutputState::Uncertain,
        OutputState::Dispatched,
        OutputState::Prepared,
    ] {
        assert!(ledger::acknowledge_notice(&connection, &request.operation_id, state).is_err());
    }
    assert!(
        ledger::acknowledge_notice(&connection, "missing", OutputState::PendingTarget).is_err()
    );
    {
        let transaction = connection.transaction().unwrap();
        ledger::acknowledge_notice(
            &transaction,
            &request.operation_id,
            OutputState::PendingTarget,
        )
        .unwrap();
        transaction.rollback().unwrap();
    }
    assert_eq!(
        ledger::list_unresolved_notices(&connection, None, 20)
            .unwrap()
            .items
            .len(),
        1
    );
    assert!(!ledger::claim_dispatch(&connection, &request).unwrap());
}

#[test]
fn startup_discovers_lost_claim_and_abandoned_preparation_without_replaying_either() {
    let connection = Connection::open_in_memory().unwrap();
    ledger::initialize(&connection).unwrap();
    let prepared = record(&connection, "prepared", None);
    let claimed = record(&connection, "claimed", None);
    ledger::claim_dispatch(&connection, &claimed).unwrap();
    assert!(ledger::list_unresolved_notices(&connection, None, 20)
        .unwrap()
        .items
        .is_empty());
    ledger::recover_inflight(&connection).unwrap();
    let notice = ledger::list_unresolved_notices(&connection, None, 20).unwrap();
    assert_eq!(notice.items.len(), 2);
    assert_eq!(notice.items[0].state, OutputState::Uncertain);
    assert_eq!(notice.items[1].state, OutputState::Rejected);
    assert!(!ledger::claim_dispatch(&connection, &prepared).unwrap());
    assert!(!ledger::claim_dispatch(&connection, &claimed).unwrap());
}

#[test]
fn corrupted_receipt_cannot_leak_intent_fields_or_be_marked_read() {
    let connection = Connection::open_in_memory().unwrap();
    ledger::initialize(&connection).unwrap();
    let request = record(&connection, "corrupt", Some(OutputOutcome::Uncertain));
    connection
        .execute(
            "UPDATE unified_output_operations SET intent_digest=zeroblob(32) WHERE operation_id=?1",
            [&request.operation_id],
        )
        .unwrap();
    assert!(ledger::list_unresolved_notices(&connection, None, 20).is_err());
    assert!(
        ledger::acknowledge_notice(&connection, &request.operation_id, OutputState::Uncertain)
            .is_err()
    );
}
