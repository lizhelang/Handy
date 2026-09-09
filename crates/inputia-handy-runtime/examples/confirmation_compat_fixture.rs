//! 新版本生成跨版本对账用合成数据，禁止覆盖既有文件。
use inputia_core::integration::events::Identifier;
use inputia_handy_runtime::{learning::HistoryTermConfirmation, store::*};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("explicit integration-copy.db required")?,
    );
    let verify = std::env::args().nth(2).as_deref() == Some("--verify");
    if !path.is_absolute()
        || path.exists() != verify
        || path.file_name().and_then(|p| p.to_str()) != Some("integration-copy.db")
    {
        return Err("a new absolute integration-copy.db is required".into());
    }
    let key = [93; 32]; // 仅合成fixture，不是用户密钥。
    let mut store = IntegrationStore::open(&path, "compat-fixture")?;
    if verify {
        store.enable_learning(&key)?;
        for (seq, term) in [(1, "AlphaTerm"), (2, "BetaTerm")] {
            assert_eq!(
                store.confirm_history_term(
                    &key,
                    &HistoryTermConfirmation {
                        operation_id: Identifier::parse(format!("confirm-{seq}"))?,
                        item_id: item_id("fixture-source", &seq.to_string()),
                        expected_revision: 1,
                        term: term.into(),
                    },
                    || true
                )?,
                inputia_handy_runtime::learning::ApplyContribution::Replay
            );
        }
        let words = store.list_terms(10, 0)?;
        assert_eq!(words.len(), 1);
        assert_eq!(words[0].term, "AlphaTerm");
        assert_eq!(words[0].contributions, 1);
        println!("new_after_compat=pass receipts_replay=true forgotten_not_revived=true contribution_count=1");
        return Ok(());
    }
    store.register_source("history", "fixture-source")?;
    store.enable_learning(&key)?;
    for (seq, term) in [(1, "AlphaTerm"), (2, "BetaTerm")] {
        let record = seq.to_string();
        store.apply_change(&SourceChange {
            store_id: "fixture-source".into(),
            seq,
            event_id: format!("fixture-event-{seq}"),
            record_id: record.clone(),
            revision: 1,
            operation: SourceOperation::Upsert,
            policy_epoch: 1,
            payload: Some(ItemSnapshot {
                source_kind: SourceKind::Voice,
                content_type: ContentType::Text,
                text: Some(term.into()),
                title: Some("fixture title".into()),
                starred: true,
                pinned: true,
                created_at_ms: seq as i64,
                asset_ref: Some("fixture.wav".into()),
                source_app: Some("synthetic.editor".into()),
                source_trust: SourceTrust::Verified,
            }),
        })?;
        store.confirm_history_term(
            &key,
            &HistoryTermConfirmation {
                operation_id: Identifier::parse(format!("confirm-{seq}"))?,
                item_id: item_id("fixture-source", &record),
                expected_revision: 1,
                term: term.into(),
            },
            || true,
        )?;
    }
    store.forget_term_with_receipt(&key, &Identifier::parse("forget-beta")?, "BetaTerm", 1)?;
    assert_eq!(store.list_terms(10, 0)?.len(), 1);
    println!("new_fixture_ready=true history_items=2 confirmation_receipts=2 forgotten_terms=1");
    Ok(())
}
