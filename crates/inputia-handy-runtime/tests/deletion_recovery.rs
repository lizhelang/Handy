use inputia_handy_runtime::{
    deletion_lifecycle::{
        AttachmentCleanup, DeleteFailure, DeleteRequest, DeleteState, DELETE_SCHEMA_VERSION,
    },
    output_ledger::{OutputAction, OutputIntent, OutputOutcome, OutputOwner, OutputState},
    service::HistoryService,
    source::{SourceOutbox, SourceTable},
    store::{HistoryQuery, IndexedItem, IntegrationStore},
};
use rusqlite::Connection;

struct Fixture {
    root: tempfile::TempDir,
    history: Connection,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let history = Connection::open(root.path().join("history.db")).unwrap();
        history.execute_batch("CREATE TABLE transcription_history(id INTEGER PRIMARY KEY AUTOINCREMENT,file_name TEXT,timestamp INTEGER,saved INTEGER,title TEXT,transcription_text TEXT,post_processed_text TEXT);
            INSERT INTO transcription_history VALUES(1,'private-recording.wav',1,0,'private title','PRIVATE TRANSCRIPT MUST NOT ENTER JOURNAL',NULL);").unwrap();
        Connection::open(root.path().join("clipboard.db")).unwrap().execute_batch("CREATE TABLE clipboard_history(id INTEGER PRIMARY KEY AUTOINCREMENT,content_type TEXT,full_text TEXT,title TEXT,is_favorite INTEGER,is_pinned INTEGER,created_at INTEGER,image_path TEXT,source_app TEXT);").unwrap();
        Self { root, history }
    }

    fn service(&self) -> HistoryService {
        HistoryService::start(self.root.path().into(), "fixture".into(), |_| {}).unwrap()
    }

    fn index(&self) -> (HistoryService, IndexedItem) {
        let service = self.service();
        service.synchronize().unwrap();
        let item = service.query(HistoryQuery::default()).unwrap().remove(0);
        (service, item)
    }

    fn store(&self) -> IntegrationStore {
        IntegrationStore::open(self.root.path().join("integration.db"), "fixture").unwrap()
    }

    fn projection(&self) -> Connection {
        Connection::open(self.root.path().join("integration.db")).unwrap()
    }

    fn request(item: &IndexedItem, operation_id: &str) -> DeleteRequest {
        DeleteRequest {
            schema_version: DELETE_SCHEMA_VERSION,
            operation_id: operation_id.into(),
            item_id: item.item_id.clone(),
            store_id: item.store_id.clone(),
            logical_name: "history".into(),
            record_id: item.record_id.clone(),
            expected_revision: item.revision,
        }
    }

    fn stage(&self, operation_id: &str) -> DeleteRequest {
        let (service, item) = self.index();
        drop(service);
        let request = Self::request(&item, operation_id);
        self.store().prepare_deletion(&request).unwrap();
        request
    }

    fn source_delete(&mut self, request: &DeleteRequest) {
        let source = SourceOutbox::install(&mut self.history, SourceTable::History).unwrap();
        source
            .delete_record(
                &mut self.history,
                SourceTable::History,
                &request.record_id,
                request.expected_revision,
                &request.operation_id,
            )
            .unwrap();
    }

    fn source_revision(&self) -> u64 {
        self.history
            .query_row(
                "SELECT revision FROM unified_source_versions WHERE record_id='1'",
                [],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn source_count(&self) -> u64 {
        self.history
            .query_row("SELECT COUNT(*) FROM transcription_history", [], |row| {
                row.get(0)
            })
            .unwrap()
    }

    fn assert_finished(&self, service: &HistoryService, request: &DeleteRequest) {
        let record = service
            .deletion_record(request.operation_id.clone())
            .unwrap()
            .unwrap();
        assert_eq!(record.state, DeleteState::ProjectionRevoked);
        assert_eq!(record.attachment_cleanup, AttachmentCleanup::NotStarted);
        assert_eq!(record.last_failure, None);
        assert_eq!(self.source_count(), 0);
        assert_eq!(self.source_revision(), request.expected_revision + 1);
        assert!(service.query(HistoryQuery::default()).unwrap().is_empty());
        assert!(service
            .revisions(request.item_id.clone())
            .unwrap()
            .is_empty());
        let projection = self.projection();
        assert_eq!(projection.query_row("SELECT COUNT(*) FROM integration_tombstones WHERE logical_name='history' AND record_id='1'",[],|r|r.get::<_,u64>(0)).unwrap(),1);
        assert_eq!(projection.query_row("SELECT COUNT(*) FROM learning_contributions WHERE store_id=?1 AND record_id='1'",[&request.store_id],|r|r.get::<_,u64>(0)).unwrap(),0);
    }
}

#[test]
fn ordinary_delete_uses_durable_journal_and_replays_without_new_source_revision() {
    let fixture = Fixture::new();
    let (service, item) = fixture.index();
    let request = Fixture::request(&item, "normal-delete");
    let intent = OutputIntent {
        operation_id: "previous-output".into(),
        item_id: item.item_id.clone(),
        revision: item.revision,
        target_id: Some("target".into()),
        owner: OutputOwner::Platform,
        policy_epoch: service.policy_epoch().unwrap(),
        action: OutputAction::InsertText,
    };
    service.prepare_output(intent.clone()).unwrap();
    let permit = service.claim_output_with_permit(intent).unwrap().unwrap();
    assert!(service
        .delete_item(
            request.item_id.clone(),
            request.expected_revision,
            request.operation_id.clone()
        )
        .unwrap());
    assert!(permit.check().is_err());
    assert!(service
        .delete_item(
            request.item_id.clone(),
            request.expected_revision,
            request.operation_id.clone()
        )
        .unwrap());
    fixture.assert_finished(&service, &request);
    assert!(service
        .delete_item(
            request.item_id.clone(),
            request.expected_revision + 1,
            request.operation_id.clone()
        )
        .is_err());
    assert!(service
        .delete_item(
            "other-item".into(),
            request.expected_revision,
            request.operation_id.clone()
        )
        .is_err());
    let record = service
        .deletion_record(request.operation_id)
        .unwrap()
        .unwrap();
    let encoded = serde_json::to_string(&record).unwrap();
    assert!(
        !encoded.contains("PRIVATE")
            && !encoded.contains("private-recording.wav")
            && !encoded.contains("private title")
    );
    assert_eq!(fixture.source_revision(), 2);
}

#[test]
fn startup_recovers_requested_before_source_commit() {
    let fixture = Fixture::new();
    let request = fixture.stage("requested-crash");
    let service = fixture.service();
    service.synchronize().unwrap();
    fixture.assert_finished(&service, &request);
}

#[test]
fn startup_recovers_source_commit_without_phase_receipt_or_second_delete() {
    let mut fixture = Fixture::new();
    let request = fixture.stage("source-commit-crash");
    fixture.source_delete(&request);
    assert_eq!(
        fixture
            .store()
            .deletion_record(&request.operation_id)
            .unwrap()
            .unwrap()
            .state,
        DeleteState::Requested
    );
    let service = fixture.service();
    service.synchronize().unwrap();
    fixture.assert_finished(&service, &request);
}

#[test]
fn failed_projection_blocks_contents_and_claims_but_keeps_output_settlement() {
    let fixture = Fixture::new();
    let (service, item) = fixture.index();
    let request = Fixture::request(&item, "projection-fault");
    let intent = OutputIntent {
        operation_id: "in-flight-output".into(),
        item_id: item.item_id.clone(),
        revision: item.revision,
        target_id: Some("target".into()),
        owner: OutputOwner::Platform,
        policy_epoch: service.policy_epoch().unwrap(),
        action: OutputAction::InsertText,
    };
    service.prepare_output(intent.clone()).unwrap();
    let permit = service
        .claim_output_with_permit(intent.clone())
        .unwrap()
        .unwrap();
    let projection = fixture.projection();
    projection.execute_batch("CREATE TRIGGER fail_projection BEFORE DELETE ON integration_items BEGIN SELECT RAISE(ABORT,'fault'); END;").unwrap();
    assert!(service
        .delete_item(
            request.item_id.clone(),
            request.expected_revision,
            request.operation_id.clone()
        )
        .is_err());
    assert!(permit.check().is_err());
    assert_eq!(fixture.source_count(), 0);
    assert_eq!(
        service
            .deletion_record(request.operation_id.clone())
            .unwrap()
            .unwrap()
            .state,
        DeleteState::SourceApplied
    );
    assert!(service.query(HistoryQuery::default()).is_err());
    assert!(service
        .get_item(item.item_id.clone(), item.revision)
        .is_err());
    assert!(service.revisions(item.item_id.clone()).is_err());
    assert!(service.list_terms(10, 0).is_err());
    assert!(service.claim_output_with_permit(intent.clone()).is_err());
    assert_eq!(
        service
            .finish_output(intent.clone(), OutputOutcome::Uncertain)
            .unwrap()
            .state,
        OutputState::Uncertain
    );
    assert_eq!(
        service
            .output_record(intent.operation_id)
            .unwrap()
            .unwrap()
            .state,
        OutputState::Uncertain
    );
    drop(service);
    projection
        .execute_batch("DROP TRIGGER fail_projection;")
        .unwrap();
    let restarted = fixture.service();
    restarted.synchronize().unwrap();
    fixture.assert_finished(&restarted, &request);
}

#[test]
fn failed_journal_prepare_has_no_source_side_effect() {
    let fixture = Fixture::new();
    let (service, item) = fixture.index();
    fixture.projection().execute_batch("CREATE TRIGGER fail_prepare BEFORE INSERT ON integration_deletion_operations BEGIN SELECT RAISE(ABORT,'fault'); END;").unwrap();
    assert!(service
        .delete_item(item.item_id, 1, "no-journal-no-side-effect".into())
        .is_err());
    assert_eq!(fixture.source_count(), 1);
    assert_eq!(fixture.source_revision(), 1);
    assert!(service
        .deletion_record("no-journal-no-side-effect".into())
        .unwrap()
        .is_none());
}

#[test]
fn failed_phase_write_recovers_from_atomic_source_receipt() {
    for failed_state in ["source_applied", "projection_revoked"] {
        let fixture = Fixture::new();
        let (service, item) = fixture.index();
        let request = Fixture::request(&item, "phase-write-fault");
        let projection = fixture.projection();
        projection.execute_batch(&format!("CREATE TRIGGER fail_phase BEFORE UPDATE OF state ON integration_deletion_operations WHEN NEW.state='{failed_state}' BEGIN SELECT RAISE(ABORT,'fault'); END;")).unwrap();
        assert!(service
            .delete_item(
                request.item_id.clone(),
                request.expected_revision,
                request.operation_id.clone()
            )
            .is_err());
        assert_eq!(fixture.source_count(), 0);
        assert!(service.query(HistoryQuery::default()).is_err());
        let record = service
            .deletion_record(request.operation_id.clone())
            .unwrap()
            .unwrap();
        assert_eq!(
            record.state,
            if failed_state == "source_applied" {
                DeleteState::Requested
            } else {
                DeleteState::SourceApplied
            }
        );
        drop(service);
        projection
            .execute_batch("DROP TRIGGER fail_phase;")
            .unwrap();
        let restarted = fixture.service();
        restarted.synchronize().unwrap();
        fixture.assert_finished(&restarted, &request);
    }
}

#[test]
fn newer_source_revision_rejects_without_deleting_changed_text() {
    let fixture = Fixture::new();
    let request = fixture.stage("stale-revision");
    fixture.history.execute("UPDATE transcription_history SET transcription_text='new protected revision' WHERE id=1",[]).unwrap();
    let service = fixture.service();
    service.synchronize().unwrap();
    let record = service
        .deletion_record(request.operation_id.clone())
        .unwrap()
        .unwrap();
    assert_eq!(record.state, DeleteState::Rejected);
    assert_eq!(record.last_failure, Some(DeleteFailure::SourceRevision));
    assert_eq!(fixture.source_count(), 1);
    assert_eq!(fixture.source_revision(), 2);
    assert!(service
        .delete_item(
            request.item_id,
            request.expected_revision,
            request.operation_id
        )
        .is_err());
    assert_eq!(
        service.query(HistoryQuery::default()).unwrap()[0]
            .snapshot
            .text
            .as_deref(),
        Some("new protected revision")
    );
}

#[test]
fn pending_source_identity_change_never_deletes_replacement_and_original_can_resume() {
    let fixture = Fixture::new();
    let request = fixture.stage("source-identity-change");
    fixture
        .history
        .execute(
            "UPDATE unified_source_meta SET store_id='replacement-source' WHERE singleton=1",
            [],
        )
        .unwrap();
    let service = fixture.service();
    assert!(service.query(HistoryQuery::default()).is_err());
    assert_eq!(fixture.source_count(), 1);
    let record = service
        .deletion_record(request.operation_id.clone())
        .unwrap()
        .unwrap();
    assert_eq!(record.state, DeleteState::Requested);
    assert_eq!(record.last_failure, Some(DeleteFailure::SourceIdentity));
    drop(service);
    fixture
        .history
        .execute(
            "UPDATE unified_source_meta SET store_id=?1 WHERE singleton=1",
            [&request.store_id],
        )
        .unwrap();
    let restarted = fixture.service();
    restarted.synchronize().unwrap();
    fixture.assert_finished(&restarted, &request);
}

#[test]
fn replay_receipt_with_reinserted_source_does_not_delete_again() {
    let mut fixture = Fixture::new();
    let request = fixture.stage("reinsert-after-receipt");
    fixture.source_delete(&request);
    fixture.history.execute_batch("INSERT INTO transcription_history(id,file_name,timestamp,saved,transcription_text) VALUES(1,'',2,0,'new content after original deletion');").unwrap();
    let service = fixture.service();
    assert!(service.query(HistoryQuery::default()).is_err());
    let record = service
        .deletion_record(request.operation_id.clone())
        .unwrap()
        .unwrap();
    assert_eq!(record.state, DeleteState::Requested);
    assert_eq!(
        record.last_failure,
        Some(DeleteFailure::SourceChangedAfterCommit)
    );
    assert!(service
        .delete_item(
            request.item_id,
            request.expected_revision,
            request.operation_id
        )
        .is_err());
    assert_eq!(fixture.source_count(), 1);
    assert_eq!(fixture.source_revision(), 3);
}

#[test]
fn legacy_source_receipt_is_adopted_when_projection_and_item_are_already_gone() {
    let mut fixture = Fixture::new();
    let (service, item) = fixture.index();
    let request = Fixture::request(&item, "legacy-delete");
    drop(service);
    fixture.source_delete(&request);
    let service = fixture.service();
    service.synchronize().unwrap();
    assert!(service.query(HistoryQuery::default()).unwrap().is_empty());
    assert!(service
        .deletion_record(request.operation_id.clone())
        .unwrap()
        .is_none());
    assert!(service
        .delete_item(
            request.item_id.clone(),
            request.expected_revision,
            request.operation_id.clone()
        )
        .unwrap());
    fixture.assert_finished(&service, &request);
}

#[test]
fn request_identity_version_and_digest_tampering_fail_closed() {
    let fixture = Fixture::new();
    let request = fixture.stage("tamper-test");
    let mut store = fixture.store();
    assert!(store.prepare_deletion(&request).is_ok());
    let mut changed = request.clone();
    changed.expected_revision += 1;
    assert!(store.prepare_deletion(&changed).is_err());
    changed = request.clone();
    changed.schema_version += 1;
    assert!(store.prepare_deletion(&changed).is_err());
    changed = request.clone();
    changed.record_id = "2".into();
    assert!(store.prepare_deletion(&changed).is_err());
    drop(store);
    fixture
        .projection()
        .execute(
            "UPDATE integration_deletion_operations SET request_digest=zeroblob(32)",
            [],
        )
        .unwrap();
    let service = fixture.service();
    assert!(service.query(HistoryQuery::default()).is_err());
    assert!(service.deletion_record(request.operation_id).is_err());
    assert_eq!(fixture.source_count(), 1);
}

#[test]
fn completed_delete_revokes_real_revisions_learning_and_old_term_generation() {
    use inputia_core::integration::events::Identifier;
    use inputia_handy_runtime::learning::HistoryTermConfirmation;
    let fixture = Fixture::new();
    let (service, _) = fixture.index();
    fixture.history.execute("UPDATE transcription_history SET transcription_text='Inputia approved text',inputia_source_trust='verified',inputia_source_app='com.example.Editor' WHERE id=1",[]).unwrap();
    service.synchronize().unwrap();
    let item = service.query(HistoryQuery::default()).unwrap().remove(0);
    assert_eq!(service.revisions(item.item_id.clone()).unwrap().len(), 2);
    service
        .confirm_history_term(
            HistoryTermConfirmation {
                operation_id: Identifier::parse("learn-before-delete").unwrap(),
                item_id: item.item_id.clone(),
                expected_revision: item.revision,
                term: "Inputia".into(),
            },
            || true,
        )
        .unwrap();
    assert_eq!(service.list_terms(10, 0).unwrap().len(), 1);
    let before = service.voice_terms_version().unwrap();
    let request = Fixture::request(&item, "delete-learned-item");
    service
        .delete_item(
            request.item_id.clone(),
            request.expected_revision,
            request.operation_id.clone(),
        )
        .unwrap();
    fixture.assert_finished(&service, &request);
    assert!(service.list_terms(10, 0).unwrap().is_empty());
    assert!(
        service.voice_terms_version().unwrap().learning_generation > before.learning_generation
    );
}

#[cfg(unix)]
#[test]
fn live_source_file_replacement_with_same_uuid_is_rejected_before_mutation() {
    let fixture = Fixture::new();
    let (service, item) = fixture.index();
    let path = fixture.root.path().join("history.db");
    let old = fixture.root.path().join("history-old.db");
    let copy = fixture.root.path().join("history-copy.db");
    // 源库使用普通 rollback journal；空闲时复制保留 UUID，模拟路径被另一 inode 替换。
    std::fs::copy(&path, &copy).unwrap();
    std::fs::rename(&path, &old).unwrap();
    std::fs::rename(&copy, &path).unwrap();
    assert!(service
        .delete_item(item.item_id, item.revision, "physical-replacement".into())
        .is_err());
    assert_eq!(fixture.source_count(), 1, "旧连接不得继续删除已移走的源库");
    let replacement = Connection::open(&path).unwrap();
    assert_eq!(
        replacement
            .query_row("SELECT COUNT(*) FROM transcription_history", [], |row| row
                .get::<_, u64>(
                0
            ))
            .unwrap(),
        1
    );
    assert!(service.synchronize().is_err());
}

#[test]
fn pending_projection_identity_replacement_does_not_mutate_original_source() {
    let fixture = Fixture::new();
    let request = fixture.stage("projection-identity-change");
    let projection = fixture.projection();
    projection.execute("UPDATE integration_sources SET active_store_id='different-source' WHERE logical_name='history'",[]).unwrap();
    let service = fixture.service();
    assert!(service.query(HistoryQuery::default()).is_err());
    assert_eq!(fixture.source_count(), 1);
    assert_eq!(
        service
            .deletion_record(request.operation_id)
            .unwrap()
            .unwrap()
            .last_failure,
        Some(DeleteFailure::SourceIdentity)
    );
}

#[test]
fn forged_terminal_phase_cannot_skip_startup_evidence_audit_or_delete_source() {
    for (phase, failure) in [
        ("projection_revoked", None),
        ("rejected", None),
        ("rejected", Some("source_revision")),
    ] {
        let fixture = Fixture::new();
        let request = fixture.stage("invalid-terminal");
        fixture
            .projection()
            .execute(
                "UPDATE integration_deletion_operations SET state=?1,last_failure=?2",
                rusqlite::params![phase, failure],
            )
            .unwrap();
        let service = fixture.service();
        assert!(service.query(HistoryQuery::default()).is_err());
        assert!(service.synchronize().is_err());
        assert!(service
            .prepare_output(OutputIntent {
                operation_id: "must-not-prepare".into(),
                item_id: request.item_id,
                revision: request.expected_revision,
                target_id: Some("target".into()),
                owner: OutputOwner::Platform,
                policy_epoch: 1,
                action: OutputAction::InsertText
            })
            .is_err());
        assert_eq!(fixture.source_count(), 1);
        assert_eq!(fixture.source_revision(), 1);
        assert!(service.policy_epoch().is_ok(), "证据检查不阻塞安全状态请求");
    }
}

#[test]
fn completed_phase_without_source_receipt_is_not_accepted_after_restart() {
    let fixture = Fixture::new();
    let (service, item) = fixture.index();
    let request = Fixture::request(&item, "missing-completion-receipt");
    service
        .delete_item(
            request.item_id,
            request.expected_revision,
            request.operation_id.clone(),
        )
        .unwrap();
    drop(service);
    fixture
        .history
        .execute(
            "DELETE FROM unified_source_operations WHERE operation_id=?1",
            [request.operation_id],
        )
        .unwrap();
    let restarted = fixture.service();
    assert!(restarted.query(HistoryQuery::default()).is_err());
    assert_eq!(fixture.source_count(), 0);
    assert_eq!(fixture.source_revision(), 2);
}

#[test]
fn completed_phase_with_new_source_revision_is_audited_without_repeating_delete() {
    let fixture = Fixture::new();
    let (service, item) = fixture.index();
    service
        .delete_item(
            item.item_id,
            item.revision,
            "completed-then-restored".into(),
        )
        .unwrap();
    drop(service);
    fixture.history.execute_batch("INSERT INTO transcription_history(id,file_name,timestamp,saved,transcription_text) VALUES(1,'',3,0,'restored new revision');").unwrap();
    let restarted = fixture.service();
    assert!(restarted.query(HistoryQuery::default()).is_err());
    assert_eq!(fixture.source_count(), 1);
    assert_eq!(fixture.source_revision(), 3);
}

#[test]
fn paged_completion_audit_keeps_access_closed_until_last_page_and_keeps_receipts_writable() {
    let fixture = Fixture::new();
    let (service, item) = fixture.index();
    let intent = OutputIntent {
        operation_id: "settle-during-audit".into(),
        item_id: item.item_id.clone(),
        revision: item.revision,
        target_id: Some("target".into()),
        owner: OutputOwner::Platform,
        policy_epoch: 1,
        action: OutputAction::InsertText,
    };
    service.prepare_output(intent.clone()).unwrap();
    service
        .claim_output_with_permit(intent.clone())
        .unwrap()
        .unwrap();
    for index in 0..33 {
        assert!(service
            .delete_item(item.item_id.clone(), 2, format!("paged-{index:03}"))
            .is_err());
    }
    drop(service);
    fixture.projection().execute("UPDATE integration_deletion_operations SET state='projection_revoked',last_failure=NULL WHERE operation_id='paged-032'",[]).unwrap();
    let restarted = fixture.service();
    assert!(restarted.query(HistoryQuery::default()).is_err());
    assert_eq!(
        restarted
            .finish_output(intent.clone(), OutputOutcome::Uncertain)
            .unwrap()
            .state,
        OutputState::Uncertain
    );
    assert_eq!(
        restarted
            .output_record(intent.operation_id)
            .unwrap()
            .unwrap()
            .state,
        OutputState::Uncertain
    );
    assert_eq!(fixture.source_count(), 1);
}

#[test]
fn persistent_first_failure_does_not_starve_later_deletion_recovery() {
    let mut fixture = Fixture::new();
    fixture.history.execute_batch("INSERT INTO transcription_history(id,file_name,timestamp,saved,transcription_text) VALUES(2,'',2,0,'second item');").unwrap();
    let service = fixture.service();
    service.synchronize().unwrap();
    let items = service.query(HistoryQuery::default()).unwrap();
    let first = Fixture::request(
        items.iter().find(|item| item.record_id == "1").unwrap(),
        "a-blocked",
    );
    let second = Fixture::request(
        items.iter().find(|item| item.record_id == "2").unwrap(),
        "b-progress",
    );
    drop(service);
    let mut store = fixture.store();
    store.prepare_deletion(&first).unwrap();
    store.prepare_deletion(&second).unwrap();
    drop(store);
    fixture.source_delete(&first);
    fixture.history.execute_batch("INSERT INTO transcription_history(id,file_name,timestamp,saved,transcription_text) VALUES(1,'',3,0,'restored first item');").unwrap();
    let restarted = fixture.service();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let state = restarted
            .deletion_record(second.operation_id.clone())
            .unwrap()
            .unwrap()
            .state;
        if state == DeleteState::ProjectionRevoked {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "后续删除被失败的第一条饿死"
        );
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert_eq!(
        restarted
            .deletion_record(first.operation_id)
            .unwrap()
            .unwrap()
            .state,
        DeleteState::Requested
    );
    assert_eq!(fixture.source_count(), 1);
    assert!(
        restarted.query(HistoryQuery::default()).is_err(),
        "仍有未恢复项时保持正文屏障"
    );
}

#[test]
fn source_rejection_survives_crash_before_projection_receipt_and_later_matching_revision() {
    let mut fixture = Fixture::new();
    let (service, item) = fixture.index();
    drop(service);
    let mut request = Fixture::request(&item, "rejection-commit-crash");
    request.expected_revision = 2;
    fixture.store().prepare_deletion(&request).unwrap();
    let outbox = SourceOutbox::install(&mut fixture.history, SourceTable::History).unwrap();
    assert!(outbox
        .delete_record(
            &mut fixture.history,
            SourceTable::History,
            "1",
            2,
            &request.operation_id
        )
        .is_err());
    assert_eq!(
        outbox
            .delete_receipt(&fixture.history, &request.item_id, 2, &request.operation_id)
            .unwrap(),
        Some(false)
    );
    assert_eq!(
        fixture
            .store()
            .deletion_record(&request.operation_id)
            .unwrap()
            .unwrap()
            .state,
        DeleteState::Requested
    );
    fixture.history.execute("UPDATE transcription_history SET transcription_text='revision now matches old refused operation' WHERE id=1",[]).unwrap();
    assert_eq!(fixture.source_revision(), 2);
    let restarted = fixture.service();
    restarted.synchronize().unwrap();
    assert_eq!(
        restarted
            .deletion_record(request.operation_id.clone())
            .unwrap()
            .unwrap()
            .state,
        DeleteState::Rejected
    );
    assert_eq!(fixture.source_count(), 1);
    assert_eq!(fixture.source_revision(), 2);
    drop(restarted);
    // 终态审计使用拒绝时的持久观察，不把之后的正常修改误判成坏回执。
    let restarted = fixture.service();
    restarted.synchronize().unwrap();
    assert_eq!(restarted.query(HistoryQuery::default()).unwrap().len(), 1);
    assert!(restarted
        .delete_item(request.item_id, 2, request.operation_id)
        .is_err());
    assert_eq!(fixture.source_count(), 1);
}

#[test]
fn unsupported_source_rejection_receipt_version_does_not_unlock_or_repeat_operation() {
    let mut fixture = Fixture::new();
    let request = fixture.stage("unsupported-receipt");
    fixture.source_delete(&request);
    fixture
        .history
        .execute(
            "UPDATE unified_source_operations SET response=?1 WHERE operation_id=?2",
            rusqlite::params![
                r#"{"version":99,"actual_revision":0,"record_exists":false}"#,
                request.operation_id
            ],
        )
        .unwrap();
    let service = fixture.service();
    assert!(service.query(HistoryQuery::default()).is_err());
    assert_eq!(
        service
            .deletion_record(request.operation_id)
            .unwrap()
            .unwrap()
            .state,
        DeleteState::Requested
    );
    assert_eq!(fixture.source_revision(), 2);
}
