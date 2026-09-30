use inputia_handy_runtime::{
    output_ledger::{OutputAction, OutputOutcome, OutputOwner, OutputState},
    service::HistoryService,
};
use rusqlite::Connection;

fn history(root: &std::path::Path) -> Connection {
    let clipboard = Connection::open(root.join("clipboard.db")).unwrap();
    clipboard.execute_batch("CREATE TABLE clipboard_history(id INTEGER PRIMARY KEY AUTOINCREMENT,content_type TEXT,full_text TEXT,title TEXT,is_favorite INTEGER,is_pinned INTEGER,created_at INTEGER,image_path TEXT,source_app TEXT);").unwrap();
    let connection = Connection::open(root.join("history.db")).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE transcription_history(id INTEGER PRIMARY KEY AUTOINCREMENT,
         file_name TEXT,timestamp INTEGER,saved INTEGER,title TEXT,
         transcription_text TEXT,post_processed_text TEXT);
         INSERT INTO transcription_history VALUES(1,'',1,0,NULL,'原始识别','修订正文');
         INSERT INTO transcription_history VALUES(2,'',2,0,NULL,'另一条正文',NULL);",
        )
        .unwrap();
    connection
}

fn service(root: &std::path::Path) -> HistoryService {
    HistoryService::start(root.into(), "platform-output-fixture".into(), |_| {}).unwrap()
}

#[test]
fn missing_original_field_is_durable_pending_and_never_retargets_automatically() {
    let temp = tempfile::tempdir().unwrap();
    let _history = history(temp.path());
    let service = service(temp.path());
    let epoch = service.policy_epoch().unwrap();
    let pending = service
        .prepare_saved_platform_result(
            1,
            "修订正文".into(),
            None,
            Some(epoch),
            OutputAction::InsertText,
        )
        .unwrap();
    assert_eq!(pending.state, OutputState::PendingTarget);
    assert_eq!(pending.intent.owner, OutputOwner::Platform);
    assert!(!service.claim_output(pending.intent.clone()).unwrap());
    assert_eq!(
        service
            .prepare_saved_platform_result(
                1,
                "修订正文".into(),
                None,
                Some(epoch),
                OutputAction::InsertText,
            )
            .unwrap(),
        pending
    );
    assert!(service
        .prepare_saved_platform_result(
            1,
            "修订正文".into(),
            Some("new-field".into()),
            Some(epoch),
            OutputAction::InsertText,
        )
        .is_err());
    // 用户明确插入是新操作，不覆盖原录音的 pending 身份。
    let mut explicit = pending.intent.clone();
    explicit.operation_id = "explicit-new-user-action".into();
    explicit.target_id = Some("new-field".into());
    service.prepare_output(explicit.clone()).unwrap();
    assert!(service
        .claim_output_with_permit(explicit)
        .unwrap()
        .is_some());
    assert_eq!(
        service
            .output_record(pending.intent.operation_id)
            .unwrap()
            .unwrap()
            .state,
        OutputState::PendingTarget
    );
}

#[test]
fn platform_result_uses_saved_source_identity_and_refuses_text_revision_or_route_swap() {
    let temp = tempfile::tempdir().unwrap();
    let history = history(temp.path());
    let service = service(temp.path());
    let epoch = service.policy_epoch().unwrap();
    assert!(service
        .prepare_saved_platform_result(
            1,
            "原始识别".into(),
            Some("field".into()),
            Some(epoch),
            OutputAction::InsertText,
        )
        .is_err());
    let prepared = service
        .prepare_saved_platform_result(
            1,
            "修订正文".into(),
            Some("field".into()),
            Some(epoch),
            OutputAction::InsertText,
        )
        .unwrap();
    let other = service
        .prepare_saved_platform_result(
            2,
            "另一条正文".into(),
            Some("field".into()),
            Some(epoch),
            OutputAction::InsertText,
        )
        .unwrap();
    assert_ne!(prepared.intent.item_id, other.intent.item_id);
    assert_ne!(prepared.intent.operation_id, other.intent.operation_id);
    assert!(service
        .prepare_saved_platform_result(
            1,
            "修订正文".into(),
            None,
            Some(epoch),
            OutputAction::CopyPlainText,
        )
        .is_err());
    history
        .execute(
            "UPDATE transcription_history SET post_processed_text='再次编辑' WHERE id=1",
            [],
        )
        .unwrap();
    assert!(service
        .prepare_saved_platform_result(
            1,
            "再次编辑".into(),
            Some("field".into()),
            Some(epoch),
            OutputAction::InsertText,
        )
        .is_err());
    assert!(service.claim_output_with_permit(prepared.intent).is_err());
}

#[test]
fn claim_survives_restart_as_uncertain_and_cannot_replay_or_fall_back_to_copy() {
    let temp = tempfile::tempdir().unwrap();
    let _history = history(temp.path());
    let original;
    {
        let service = service(temp.path());
        original = service
            .prepare_saved_platform_result(
                1,
                "修订正文".into(),
                Some("field".into()),
                Some(1),
                OutputAction::InsertText,
            )
            .unwrap();
        assert!(service
            .claim_output_with_permit(original.intent.clone())
            .unwrap()
            .is_some());
    }
    let recovered = service(temp.path());
    let record = recovered
        .output_record(original.intent.operation_id.clone())
        .unwrap()
        .unwrap();
    assert_eq!(record.state, OutputState::Uncertain);
    assert!(recovered
        .claim_output_with_permit(record.intent.clone())
        .unwrap()
        .is_none());
    assert_eq!(
        recovered
            .prepare_saved_platform_result(
                1,
                "修订正文".into(),
                Some("field".into()),
                Some(1),
                OutputAction::InsertText,
            )
            .unwrap()
            .state,
        OutputState::Uncertain
    );
    let mut copy = record.intent;
    copy.action = OutputAction::CopyPlainText;
    assert!(recovered.prepare_output(copy).is_err());
}

#[test]
fn stale_or_unavailable_start_policy_stays_pending_even_with_observable_target() {
    for epoch in [None, Some(0), Some(99)] {
        let temp = tempfile::tempdir().unwrap();
        let _history = history(temp.path());
        let service = service(temp.path());
        let pending = service
            .prepare_saved_platform_result(
                1,
                "修订正文".into(),
                Some("field".into()),
                epoch,
                OutputAction::InsertText,
            )
            .unwrap();
        assert_eq!(pending.state, OutputState::PendingTarget);
        assert!(!service.claim_output(pending.intent).unwrap());
    }
}

#[test]
fn explicit_copy_needs_no_field_but_still_obeys_one_claim_and_revocation() {
    let temp = tempfile::tempdir().unwrap();
    let _history = history(temp.path());
    let service = service(temp.path());
    let prepared = service
        .prepare_saved_platform_result(
            1,
            "修订正文".into(),
            None,
            Some(1),
            OutputAction::CopyPlainText,
        )
        .unwrap();
    assert_eq!(prepared.state, OutputState::Prepared);
    let permit = service
        .claim_output_with_permit(prepared.intent.clone())
        .unwrap()
        .unwrap();
    assert!(permit.check().is_ok());
    let write_guard = service.begin_source_write();
    assert!(permit.check().is_err());
    assert!(service
        .claim_output_with_permit(prepared.intent.clone())
        .is_err());
    drop(write_guard);
    service
        .finish_output(
            prepared.intent.clone(),
            OutputOutcome::NotDispatchedPendingTarget,
        )
        .unwrap();
    assert!(service
        .claim_output_with_permit(prepared.intent)
        .unwrap()
        .is_none());
}

#[test]
fn background_service_recovers_and_acknowledges_saved_output_notices_without_replay() {
    let temp = tempfile::tempdir().unwrap();
    let _history = history(temp.path());
    let operation;
    {
        let service = service(temp.path());
        let output = service
            .prepare_saved_platform_result(
                1,
                "修订正文".into(),
                Some("field".into()),
                Some(1),
                OutputAction::InsertText,
            )
            .unwrap();
        operation = output.intent.operation_id.clone();
        assert!(service
            .claim_output_with_permit(output.intent)
            .unwrap()
            .is_some());
    }
    {
        let service = service(temp.path());
        let page = service.unresolved_output_notices(None, 50).unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].operation_id, operation);
        assert_eq!(page.items[0].state, OutputState::Uncertain);
        service
            .acknowledge_output_notice(operation.clone(), OutputState::Uncertain)
            .unwrap();
        assert!(service
            .unresolved_output_notices(None, 50)
            .unwrap()
            .items
            .is_empty());
    }
    let service = service(temp.path());
    assert!(service
        .unresolved_output_notices(None, 50)
        .unwrap()
        .items
        .is_empty());
    let output = service.output_record(operation).unwrap().unwrap();
    assert_eq!(output.state, OutputState::Uncertain);
    assert!(service
        .claim_output_with_permit(output.intent)
        .unwrap()
        .is_none());
}
