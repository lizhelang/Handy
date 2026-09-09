use inputia_core::integration::{
    events::{Identifier, SourceRecord},
    privacy::{HistoryMode, PrivacyContext, PrivacyPolicy, SourceTrust},
    terms::{HotwordBudget, TermEvidence},
};
use inputia_handy_runtime::{
    learning::ContributionInput,
    store::{
        ContentType, IntegrationStore, ItemSnapshot, SourceChange, SourceKind, SourceOperation,
        SourceTrust as ItemTrust, StoreError,
    },
};
use rusqlite::Connection;
const KEY: [u8; 32] = [43; 32];

fn confirmation() -> inputia_handy_runtime::learning::HistoryTermConfirmation {
    inputia_handy_runtime::learning::HistoryTermConfirmation {
        operation_id: Identifier::parse("confirmation-1").unwrap(),
        item_id: inputia_handy_runtime::store::item_id("history", "1"),
        expected_revision: 1,
        term: "Codex".into(),
    }
}

#[test]
fn confirmation_replays_after_forgetting_and_restart_without_reviving() {
    use inputia_handy_runtime::learning::ApplyContribution;
    for forgotten in ["Inputia", "Codex"] {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("index.db");
        let mut store = open(&path);
        let request = confirmation();
        assert_eq!(
            store.confirm_history_term(&KEY, &request, || true).unwrap(),
            ApplyContribution::Applied
        );
        store.forget_term(&KEY, forgotten, 1).unwrap();
        let before = words(&store, 2);
        drop(store);
        let mut store = IntegrationStore::open(&path, "test").unwrap();
        store.enable_learning(&KEY).unwrap();
        assert_eq!(
            store
                .confirm_history_term(&KEY, &request, || false)
                .unwrap(),
            ApplyContribution::Replay
        );
        assert_eq!(words(&store, 2), before);
        for altered in [
            inputia_handy_runtime::learning::HistoryTermConfirmation {
                term: "Other".into(),
                ..request.clone()
            },
            inputia_handy_runtime::learning::HistoryTermConfirmation {
                item_id: "other".into(),
                ..request.clone()
            },
            inputia_handy_runtime::learning::HistoryTermConfirmation {
                expected_revision: 2,
                ..request.clone()
            },
        ] {
            assert!(store.confirm_history_term(&KEY, &altered, || true).is_err());
        }
        assert!(store
            .confirm_history_term(&[44; 32], &request, || true)
            .is_err());
        let reader = Connection::open(&path).unwrap();
        assert_eq!(
            reader
                .query_row(
                    "SELECT COUNT(*) FROM learning_confirmation_receipts",
                    [],
                    |row| row.get::<_, u32>(0)
                )
                .unwrap(),
            1
        );
    }
}

#[test]
fn confirmation_receipt_failure_and_guard_denial_are_atomic() {
    use inputia_handy_runtime::learning::ApplyContribution;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("index.db");
    let mut store = open(&path);
    let request = confirmation();
    assert!(store
        .confirm_history_term(&KEY, &request, || false)
        .is_err());
    assert_eq!(words(&store, 1), vec!["Inputia"]);
    let reader = Connection::open(&path).unwrap();
    reader.execute_batch("CREATE TRIGGER reject_confirmation BEFORE INSERT ON learning_confirmation_receipts BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(store.confirm_history_term(&KEY, &request, || true).is_err());
    assert_eq!(words(&store, 1), vec!["Inputia"]);
    assert_eq!(
        reader
            .query_row(
                "SELECT COUNT(*) FROM learning_receipts WHERE contribution_id='confirmation-1'",
                [],
                |row| row.get::<_, u32>(0)
            )
            .unwrap(),
        0
    );
    reader
        .execute_batch("DROP TRIGGER reject_confirmation;")
        .unwrap();
    assert_eq!(
        store.confirm_history_term(&KEY, &request, || true).unwrap(),
        ApplyContribution::Applied
    );
    store.apply_change(&change(2, 2, None)).unwrap();
    drop(store);
    let mut store = IntegrationStore::open(&path, "test").unwrap();
    store.enable_learning(&KEY).unwrap();
    assert_eq!(
        store.confirm_history_term(&KEY, &request, || true).unwrap(),
        ApplyContribution::Replay
    );
    assert!(words(&store, store.policy_epoch().unwrap()).is_empty());
}

#[test]
fn confirmation_never_promotes_source_trust_or_relearns_forgotten_terms() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = open(&temp.path().join("index.db"));
    let mut image = change(2, 2, Some("Codex"));
    image.payload.as_mut().unwrap().content_type = ContentType::Image;
    store.apply_change(&image).unwrap();
    assert!(store
        .confirm_history_term(
            &KEY,
            &inputia_handy_runtime::learning::HistoryTermConfirmation {
                expected_revision: 2,
                ..confirmation()
            },
            || true
        )
        .is_err());
    for trust in [ItemTrust::Unknown, ItemTrust::Observed] {
        let temp = tempfile::tempdir().unwrap();
        let mut store = open(&temp.path().join("index.db"));
        let mut update = change(2, 2, Some("Codex"));
        update.payload.as_mut().unwrap().source_trust = trust;
        store.apply_change(&update).unwrap();
        let request = inputia_handy_runtime::learning::HistoryTermConfirmation {
            expected_revision: 2,
            ..confirmation()
        };
        assert!(store.confirm_history_term(&KEY, &request, || true).is_err());
        assert!(words(&store, store.policy_epoch().unwrap()).is_empty());
    }
    let temp = tempfile::tempdir().unwrap();
    let mut store = open(&temp.path().join("index.db"));
    store.forget_term(&KEY, "Codex", 1).unwrap();
    assert!(store
        .confirm_history_term(&KEY, &confirmation(), || true)
        .is_err());
    let mut long = confirmation();
    long.term = "this is an entire paragraph and should never be a short term".into();
    assert!(store.confirm_history_term(&KEY, &long, || true).is_err());
}

#[test]
fn forget_receipt_failure_rolls_back_the_entire_forget() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("index.db");
    let mut store = open(&path);
    let injector = Connection::open(&path).unwrap();
    injector.execute_batch("CREATE TRIGGER reject_forget_receipt BEFORE INSERT ON learning_forget_receipts BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    let operation = Identifier::parse("forget-atomic").unwrap();
    assert!(store
        .forget_term_with_receipt(&KEY, &operation, "Inputia", 1)
        .is_err());
    assert_eq!(words(&store, 1), vec!["Inputia"]);
    injector
        .execute_batch("DROP TRIGGER reject_forget_receipt;")
        .unwrap();
    assert_eq!(
        store
            .forget_term_with_receipt(&KEY, &operation, "Inputia", 1)
            .unwrap(),
        2
    );
    assert_eq!(
        store
            .forget_term_with_receipt(&KEY, &operation, "Inputia", 1)
            .unwrap(),
        2
    );
    assert!(words(&store, 2).is_empty());
}
fn policy(epoch: u64) -> PrivacyPolicy {
    PrivacyPolicy {
        epoch,
        history_enabled: true,
        history_mode: HistoryMode::Normal,
        learning_enabled: true,
        remote_learning_terms_enabled: false,
    }
}
fn context() -> PrivacyContext {
    PrivacyContext {
        source_trust: SourceTrust::Verified,
        source_sensitive: false,
        target_known: true,
        target_sensitive: false,
        secure_input: false,
        transient_or_concealed: false,
    }
}
fn change(seq: u64, revision: u64, text: Option<&str>) -> SourceChange {
    SourceChange {
        store_id: "history".into(),
        seq,
        event_id: format!("event-{seq}"),
        record_id: "1".into(),
        revision,
        operation: if text.is_some() {
            SourceOperation::Upsert
        } else {
            SourceOperation::Delete
        },
        policy_epoch: 1,
        payload: text.map(|text| ItemSnapshot {
            source_kind: SourceKind::Voice,
            content_type: ContentType::Text,
            text: Some(text.into()),
            title: None,
            starred: false,
            pinned: false,
            created_at_ms: 1,
            asset_ref: None,
            source_app: None,
            source_trust: ItemTrust::Verified,
        }),
    }
}
fn contribution() -> ContributionInput {
    ContributionInput {
        contribution_id: Identifier::parse("term-1").unwrap(),
        source: SourceRecord {
            store_id: Identifier::parse("history").unwrap(),
            record_id: Identifier::parse("1").unwrap(),
        },
        source_revision: 1,
        policy_epoch: 1,
        term: "Inputia".into(),
        evidence: TermEvidence::ConfirmedCorrection,
        explicit_relearn: false,
    }
}
fn open(path: &std::path::Path) -> IntegrationStore {
    let mut store = IntegrationStore::open(path, "test").unwrap();
    store.register_source("voice", "history").unwrap();
    store.enable_learning(&KEY).unwrap();
    store
        .apply_change(&change(1, 1, Some("Inputia 是术语")))
        .unwrap();
    store
        .contribute_term(&KEY, &contribution(), &policy(1), context())
        .unwrap();
    store
}
fn words(store: &IntegrationStore, epoch: u64) -> Vec<String> {
    store
        .hotwords(
            &KEY,
            &policy(epoch),
            context(),
            &[],
            HotwordBudget::default(),
        )
        .unwrap()
}

#[test]
fn deleting_source_and_its_learning_share_the_same_commit() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("index.db");
    let mut store = open(&path);
    assert_eq!(words(&store, 1), vec!["Inputia"]);
    let terms = store.list_terms(50, 0).unwrap();
    assert_eq!(terms.len(), 1);
    assert!(terms[0].explicitly_confirmed);
    assert_eq!(terms[0].contributions, 1);
    let injector = Connection::open(&path).unwrap();
    injector.execute_batch("CREATE TRIGGER reject_event BEFORE INSERT ON integration_events BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(store.apply_change(&change(2, 2, None)).is_err());
    assert_eq!(store.item_count().unwrap(), 1);
    assert_eq!(words(&store, 1), vec!["Inputia"]);
    injector
        .execute_batch("DROP TRIGGER reject_event;")
        .unwrap();
    store.apply_change(&change(2, 2, None)).unwrap();
    assert!(words(&store, 1).is_empty());
    assert_eq!(store.item_count().unwrap(), 0);
    drop(store);
    let mut reopened = IntegrationStore::open(&path, "test").unwrap();
    reopened.enable_learning(&KEY).unwrap();
    assert!(words(&reopened, 1).is_empty());
    assert!(reopened
        .contribute_term(&KEY, &contribution(), &policy(1), context())
        .is_err());
}

#[test]
fn metadata_preserves_terms_but_body_revision_revokes_them() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = open(&temp.path().join("index.db"));
    let mut metadata = change(2, 2, Some("Inputia 是术语"));
    metadata.payload.as_mut().unwrap().starred = true;
    store.apply_change(&metadata).unwrap();
    assert_eq!(words(&store, 1), vec!["Inputia"]);
    let mut second = contribution();
    second.contribution_id = Identifier::parse("second-term").unwrap();
    second.source_revision = 2;
    second.term = "SecondTerm".into();
    store
        .contribute_term(&KEY, &second, &policy(1), context())
        .unwrap();
    assert_eq!(words(&store, 1), vec!["Inputia", "SecondTerm"]);
    store
        .apply_change(&change(3, 3, Some("修改为另一段文字")))
        .unwrap();
    assert!(words(&store, 1).is_empty());
    assert!(store
        .contribute_term(&KEY, &contribution(), &policy(1), context())
        .is_err());
}

#[test]
fn persisted_unknown_source_cannot_be_promoted_by_a_caller() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = IntegrationStore::open(temp.path().join("index.db"), "test").unwrap();
    store.register_source("voice", "history").unwrap();
    store.enable_learning(&KEY).unwrap();
    let mut unknown = change(1, 1, Some("Inputia"));
    unknown.payload.as_mut().unwrap().source_trust = ItemTrust::Unknown;
    store.apply_change(&unknown).unwrap();
    assert!(matches!(
        store.contribute_term(&KEY, &contribution(), &policy(1), context()),
        Err(StoreError::Learning(_))
    ));
    assert!(words(&store, 1).is_empty());
}

#[test]
fn source_revocation_invalidates_snapshot_without_blocking_subsequent_source_sequence() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = open(&temp.path().join("index.db"));
    let before = store
        .term_snapshot(&KEY, &policy(1), context(), &[], HotwordBudget::default())
        .unwrap();
    assert!(store.term_snapshot_is_current(&before).unwrap());
    store.apply_change(&change(2, 2, None)).unwrap();
    assert!(!store.term_snapshot_is_current(&before).unwrap());
    let after = store
        .term_snapshot(&KEY, &policy(1), context(), &[], HotwordBudget::default())
        .unwrap();
    assert!(after.terms.is_empty());
    assert!(after.learning_generation > before.learning_generation);
    let mut next = change(3, 1, Some("next record"));
    next.record_id = "2".into();
    store.apply_change(&next).unwrap();
    assert_eq!(store.cursor("history").unwrap(), 3);
}

#[test]
fn forget_policy_and_contributions_are_atomic_even_when_policy_write_fails() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("index.db");
    let mut store = open(&path);
    let injector = Connection::open(&path).unwrap();
    injector.execute_batch("CREATE TRIGGER reject_policy BEFORE UPDATE ON integration_meta WHEN NEW.key='policy_epoch' BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    assert!(store.forget_term(&KEY, "Inputia", 1).is_err());
    assert_eq!(store.policy_epoch().unwrap(), 1);
    assert_eq!(words(&store, 1), vec!["Inputia"]);
    injector
        .execute_batch("DROP TRIGGER reject_policy")
        .unwrap();
    assert_eq!(store.forget_term(&KEY, "Inputia", 1).unwrap(), 2);
    assert!(words(&store, 2).is_empty());
    assert!(matches!(
        store.contribute_term(&KEY, &contribution(), &policy(1), context()),
        Err(StoreError::EpochMismatch { .. })
    ));
}

#[test]
fn an_existing_source_deletion_remains_blocked_when_learning_is_enabled_later() {
    let temp = tempfile::tempdir().unwrap();
    let mut store = IntegrationStore::open(temp.path().join("index.db"), "test").unwrap();
    store.register_source("voice", "history").unwrap();
    store.apply_change(&change(1, 1, Some("Inputia"))).unwrap();
    store.apply_change(&change(2, 2, None)).unwrap();
    store.enable_learning(&KEY).unwrap();
    assert!(store
        .contribute_term(&KEY, &contribution(), &policy(1), context())
        .is_err());
    assert!(words(&store, 1).is_empty());
}
