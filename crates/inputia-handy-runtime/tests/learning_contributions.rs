use inputia_core::integration::{
    events::{Identifier, SourceRecord},
    privacy::{HistoryMode, PrivacyContext, PrivacyPolicy, SourceTrust},
    terms::{HotwordBudget, TermEvidence},
};
use inputia_handy_runtime::learning::{
    ApplyContribution, ContributionInput, LearningError, LearningLedger,
};
use rusqlite::Connection;

fn id(s: &str) -> Identifier {
    Identifier::parse(s).unwrap()
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
fn policy(epoch: u64) -> PrivacyPolicy {
    PrivacyPolicy {
        epoch,
        history_enabled: true,
        history_mode: HistoryMode::Normal,
        learning_enabled: true,
        remote_learning_terms_enabled: false,
    }
}
fn input(event: &str, record: &str, term: &str, epoch: u64) -> ContributionInput {
    ContributionInput {
        contribution_id: id(event),
        source: SourceRecord {
            store_id: id("voice"),
            record_id: id(record),
        },
        source_revision: 1,
        policy_epoch: epoch,
        term: term.into(),
        evidence: TermEvidence::ConfirmedCorrection,
        explicit_relearn: false,
    }
}
fn setup() -> (Connection, LearningLedger) {
    let mut conn = Connection::open_in_memory().unwrap();
    let ledger = LearningLedger::new(&[19; 32]).unwrap();
    ledger.install(&conn).unwrap();
    let tx = conn.transaction().unwrap();
    ledger.advance_epoch(&tx, 1).unwrap();
    tx.commit().unwrap();
    (conn, ledger)
}
fn words(ledger: &LearningLedger, conn: &Connection, epoch: u64) -> Vec<String> {
    ledger
        .hotwords(
            conn,
            &policy(epoch),
            context(),
            epoch,
            &[],
            HotwordBudget::default(),
        )
        .unwrap()
}

#[test]
fn replay_is_idempotent_but_reused_id_with_changed_text_is_rejected() {
    let (mut conn, ledger) = setup();
    let first = input("event-1", "record-1", "Inputia", 1);
    let tx = conn.transaction().unwrap();
    assert_eq!(
        ledger
            .apply_contribution(&tx, &first, &policy(1), context())
            .unwrap(),
        ApplyContribution::Applied
    );
    tx.commit().unwrap();
    let tx = conn.transaction().unwrap();
    assert_eq!(
        ledger
            .apply_contribution(&tx, &first, &policy(1), context())
            .unwrap(),
        ApplyContribution::Replay
    );
    tx.commit().unwrap();
    let conflict = input("event-1", "record-1", "Different", 1);
    let tx = conn.transaction().unwrap();
    assert!(matches!(
        ledger.apply_contribution(&tx, &conflict, &policy(1), context()),
        Err(LearningError::ReplayConflict)
    ));
    drop(tx);
    assert_eq!(words(&ledger, &conn, 1), vec!["Inputia"]);
}

#[test]
fn deleting_one_source_preserves_other_contribution_and_blocks_old_source() {
    let (mut conn, ledger) = setup();
    for (event, record) in [("first", "one"), ("second", "two")] {
        let tx = conn.transaction().unwrap();
        ledger
            .apply_contribution(
                &tx,
                &input(event, record, "Inputia", 1),
                &policy(1),
                context(),
            )
            .unwrap();
        tx.commit().unwrap();
    }
    let first = input("third", "one", "Inputia", 1);
    let tx = conn.transaction().unwrap();
    ledger.delete_source(&tx, &first.source, 2).unwrap();
    tx.commit().unwrap();
    assert_eq!(words(&ledger, &conn, 1), vec!["Inputia"]);
    let tx = conn.transaction().unwrap();
    assert!(matches!(
        ledger.apply_contribution(&tx, &first, &policy(1), context()),
        Err(LearningError::SourceDeleted)
    ));
    drop(tx);
    let tx = conn.transaction().unwrap();
    ledger
        .delete_source(&tx, &input("fourth", "two", "Inputia", 1).source, 2)
        .unwrap();
    tx.commit().unwrap();
    assert!(words(&ledger, &conn, 1).is_empty());
}

#[test]
fn forget_requires_new_explicit_relearn_and_old_epoch_never_returns() {
    let (mut conn, ledger) = setup();
    let old = input("first", "one", "Inputia", 1);
    let tx = conn.transaction().unwrap();
    ledger
        .apply_contribution(&tx, &old, &policy(1), context())
        .unwrap();
    tx.commit().unwrap();
    let tx = conn.transaction().unwrap();
    ledger.forget_term(&tx, "Inputia", 2).unwrap();
    tx.commit().unwrap();
    assert!(words(&ledger, &conn, 2).is_empty());
    let tx = conn.transaction().unwrap();
    assert!(matches!(
        ledger.apply_contribution(&tx, &old, &policy(2), context()),
        Err(LearningError::StaleEpoch)
    ));
    drop(tx);
    let mut new = input("second", "two", "Inputia", 2);
    let tx = conn.transaction().unwrap();
    assert!(matches!(
        ledger.apply_contribution(&tx, &new, &policy(2), context()),
        Err(LearningError::Forgotten)
    ));
    drop(tx);
    new.evidence = TermEvidence::ExplicitUserTerm;
    new.explicit_relearn = true;
    let tx = conn.transaction().unwrap();
    ledger
        .apply_contribution(&tx, &new, &policy(2), context())
        .unwrap();
    tx.commit().unwrap();
    assert_eq!(words(&ledger, &conn, 2), vec!["Inputia"]);
    let tx = conn.transaction().unwrap();
    assert!(matches!(
        ledger.apply_contribution(&tx, &old, &policy(2), context()),
        Err(LearningError::StaleEpoch)
    ));
}

#[test]
fn failed_outer_transaction_rolls_back_contribution_and_receipt() {
    let (mut conn, ledger) = setup();
    let event = input("first", "one", "Inputia", 1);
    {
        let tx = conn.transaction().unwrap();
        ledger
            .apply_contribution(&tx, &event, &policy(1), context())
            .unwrap();
    }
    assert!(words(&ledger, &conn, 1).is_empty());
    let tx = conn.transaction().unwrap();
    assert_eq!(
        ledger
            .apply_contribution(&tx, &event, &policy(1), context())
            .unwrap(),
        ApplyContribution::Applied
    );
    tx.commit().unwrap();
}

#[test]
fn unknown_sources_raw_asr_and_sensitive_targets_cannot_feed_personalization() {
    let (mut conn, ledger) = setup();
    let mut event = input("first", "one", "Inputia", 1);
    let tx = conn.transaction().unwrap();
    assert!(matches!(
        ledger.apply_contribution(
            &tx,
            &event,
            &policy(1),
            PrivacyContext {
                source_trust: SourceTrust::Unknown,
                ..context()
            }
        ),
        Err(LearningError::PrivacyDenied)
    ));
    drop(tx);
    event.evidence = TermEvidence::UnconfirmedVoice;
    let tx = conn.transaction().unwrap();
    assert!(matches!(
        ledger.apply_contribution(&tx, &event, &policy(1), context()),
        Err(LearningError::InvalidTerm(_))
    ));
    drop(tx);
    assert!(matches!(
        ledger.hotwords(
            &conn,
            &policy(1),
            PrivacyContext {
                target_sensitive: true,
                ..context()
            },
            1,
            &["Inputia".into()],
            HotwordBudget::default()
        ),
        Err(LearningError::PrivacyDenied)
    ));
}

#[test]
fn key_and_epoch_survive_reopen_and_tombstone_contains_no_text() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("learning.db");
    let mut conn = Connection::open(&path).unwrap();
    let ledger = LearningLedger::new(&[42; 32]).unwrap();
    ledger.install(&conn).unwrap();
    let tx = conn.transaction().unwrap();
    ledger.advance_epoch(&tx, 1).unwrap();
    ledger
        .apply_contribution(
            &tx,
            &input("first", "one", "Inputia", 1),
            &policy(1),
            context(),
        )
        .unwrap();
    ledger.forget_term(&tx, "Inputia", 2).unwrap();
    tx.commit().unwrap();
    drop(conn);
    let conn = Connection::open(path).unwrap();
    ledger.install(&conn).unwrap();
    assert_eq!(ledger.epoch(&conn).unwrap(), 2);
    assert!(words(&ledger, &conn, 2).is_empty());
    let bytes: Vec<u8> = conn
        .query_row("SELECT term_id FROM learning_forgotten", [], |r| r.get(0))
        .unwrap();
    assert_eq!(bytes.len(), 32);
    assert!(!bytes.windows(7).any(|w| w == b"Inputia"));
    assert!(matches!(
        LearningLedger::new(&[43; 32]).unwrap().install(&conn),
        Err(LearningError::KeyMismatch)
    ));
}
