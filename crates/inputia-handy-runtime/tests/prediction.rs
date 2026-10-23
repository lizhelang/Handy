#[path = "support/privacy.rs"]
mod privacy;
use inputia_handy_runtime::personalization::*;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const APP: &str = "com.example.editor";

fn table() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../src-tauri/resources/personalization/prediction-ngram.sqlite")
}
fn registered() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    install_prediction_table(tmp.path(), &table()).unwrap();
    tmp
}
fn ask(root: &Path, context: &str, limit: usize) -> Query {
    Query {
        input_code: String::new(),
        schema_id: String::new(),
        context: context.into(),
        context_id: "target1".into(),
        source_app: APP.into(),
        learning_epoch: policy(root).unwrap().epoch,
        candidates: Vec::new(),
        limit,
    }
}
fn predictions(root: &Path, q: Query) -> Vec<(String, String)> {
    query(root, q).unwrap()["predictions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["id"].as_str().unwrap().into(),
                p["text"].as_str().unwrap().into(),
            )
        })
        .collect()
}
fn texts(root: &Path, context: &str) -> Vec<String> {
    predictions(root, ask(root, context, 5))
        .into_iter()
        .map(|(_, text)| text)
        .collect()
}
fn feedback_for(root: &Path, id: &str, text: &str, previous: &str, operation: &str) -> Feedback {
    Feedback {
        event_id: id.into(),
        context_id: "target1".into(),
        input_code: String::new(),
        schema_id: String::new(),
        text: text.into(),
        previous: previous.into(),
        explicit_selection: operation == "accept",
        original_rank: 0,
        source_app: APP.into(),
        learning_epoch: policy(root).unwrap().epoch,
        operation: operation.into(),
    }
}

#[test]
fn packaged_table_registers_once_and_answers_stably_within_admission_window() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    assert_eq!(
        install_prediction_table(root, &table()).unwrap()["changed"],
        true
    );
    assert_eq!(
        install_prediction_table(root, &table()).unwrap()["changed"],
        false
    );
    for context in ["我们", "今天", "谢谢", "因为", "但是"] {
        let shown = predictions(root, ask(root, context, 5));
        assert!(
            !shown.is_empty() && shown.len() <= 5,
            "{context}: {shown:?}"
        );
        assert!(shown.iter().all(|(id, _)| id.starts_with("base:")));
        // 准入用更宽的上限重查，必须仍包含已显示的每一项且顺序一致。
        let wider = predictions(root, ask(root, context, 9));
        assert_eq!(&wider[..shown.len()], &shown[..], "{context}");
        assert_eq!(predictions(root, ask(root, context, 5)), shown);
    }
    let mut micros = Vec::new();
    for i in 0..200 {
        let started = std::time::Instant::now();
        query(root, ask(root, ["我们", "今天下午", "项目"][i % 3], 5)).unwrap();
        micros.push(started.elapsed().as_micros());
    }
    micros.sort_unstable();
    assert!(micros[189] < 50_000, "联想查询 p95 {} us", micros[189]);
}

#[test]
fn punctuation_english_and_excluded_apps_get_no_public_continuation() {
    let tmp = registered();
    let root = tmp.path();
    for context in ["好的。", "好的，", "hello", "版本 2"] {
        assert!(texts(root, context).is_empty(), "{context}");
    }
    let mut excluded = ask(root, "我们", 5);
    excluded.source_app = "com.1password.1password".into();
    assert_eq!(query(root, excluded).unwrap()["predictions"], json!([]));
}

#[test]
fn personal_evidence_is_blended_instead_of_taking_every_slot() {
    let tmp = registered();
    let root = tmp.path();
    feedback(root, feedback_for(root, "strong", "开会", "我们", "accept")).unwrap();
    let shown = predictions(root, ask(root, "我们", 5));
    assert_eq!(shown[0].1, "开会");
    assert!(shown[0].0.starts_with("learned:"));
    assert!(
        shown
            .iter()
            .filter(|(id, _)| id.starts_with("base:"))
            .count()
            >= 2,
        "{shown:?}"
    );
    // 只有片段重合（前文含"项目"但不以它结尾）不说明下一个词，不推荐。
    feedback(
        root,
        feedback_for(root, "anchor", "报告", "项目进展汇报", "accept"),
    )
    .unwrap();
    assert!(!texts(root, "这个项目").contains(&"报告".to_string()));
    // 上一个词相同（前文同样以"项目"结尾）才计入个人证据。
    feedback(
        root,
        feedback_for(root, "last", "预算", "昨天写的项目", "accept"),
    )
    .unwrap();
    let shown = predictions(root, ask(root, "这个项目", 5));
    assert!(
        shown
            .iter()
            .any(|(id, text)| text == "预算" && id.starts_with("learned:")),
        "{shown:?}"
    );
    // 关闭个人学习后只剩公共续写。
    manage(root, "personalization_enabled", &json!({"enabled": false})).unwrap();
    let shown = predictions(root, ask(root, "我们", 5));
    assert!(!shown.is_empty());
    assert!(shown.iter().all(|(id, _)| id.starts_with("base:")));
}

#[test]
fn just_accepted_continuation_is_not_suggested_again() {
    let tmp = registered();
    let root = tmp.path();
    // 用户在"在这"之后选了"个"；下一轮前文是"在这个"，不能再推"个"。
    feedback(root, feedback_for(root, "chosen", "个", "在这", "accept")).unwrap();
    assert_eq!(texts(root, "在这")[0], "个");
    let next = texts(root, "在这个");
    assert!(!next.contains(&"个".to_string()), "{next:?}");
    assert!(!next.is_empty());
}

#[test]
fn forgotten_and_rejected_public_continuations_stay_hidden() {
    let tmp = registered();
    let root = tmp.path();
    let before = texts(root, "我们");
    assert!(before.len() >= 2, "{before:?}");
    privacy::forget(root, Some(&before[0]));
    let after_forget = texts(root, "我们");
    assert!(!after_forget.contains(&before[0]), "{after_forget:?}");
    let rejected = after_forget[0].clone();
    feedback(
        root,
        feedback_for(root, "reject", &rejected, "我们", "reject"),
    )
    .unwrap();
    assert!(!texts(root, "我们").contains(&rejected));
    let reply: Value = query(root, ask(root, "我们", 5)).unwrap();
    assert_eq!(reply["enabled"], true);
}

#[test]
fn confirmed_history_backfill_learns_word_pairs_and_every_revocation_removes_them() {
    use inputia_handy_runtime::{knowledge::KnowledgeStore, typed_history};
    let tmp = registered();
    let root = tmp.path();
    KnowledgeStore::open(root).unwrap();
    let epoch = typed_history::set_capture(root, true).unwrap().epoch;
    let text = "这个预算下周再定。这个预算我来改。";
    let recorded =
        typed_history::record(root, "t1", "seg", text, "com.example.editor", epoch).unwrap();
    let learned = |root: &Path| {
        predictions(root, ask(root, "这个", 3))
            .into_iter()
            .any(|(id, text)| text == "预算" && id.starts_with("learned:"))
    };
    // 只收录不回填时不学历史；用户确认回填后才计入。
    assert!(!learned(root));
    manage(root, "personalization_backfill", &json!({"source":"typed"})).unwrap();
    assert!(
        learned(root),
        "{:?}",
        predictions(root, ask(root, "这个", 3))
    );
    let totals = |root: &Path| -> i64 {
        rusqlite::Connection::open(root.join("knowledge/personalization.sqlite"))
            .unwrap()
            .query_row(
                "SELECT COALESCE(SUM(count),0) FROM history_bigram_totals",
                [],
                |r| r.get(0),
            )
            .unwrap()
    };
    assert!(totals(root) > 0);
    // 来源删除后由来源核对撤销，汇总同步扣减为零。
    typed_history::delete(root, recorded["id"].as_str().unwrap()).unwrap();
    reconcile_imports(root, 64).unwrap();
    assert!(!learned(root));
    assert_eq!(totals(root), 0);
    // 遗忘与清空同样移除历史词对。
    let epoch = typed_history::policy(root).unwrap().epoch;
    typed_history::record(root, "t2", "seg2", text, "com.example.editor", epoch).unwrap();
    manage(root, "personalization_backfill", &json!({"source":"typed"})).unwrap();
    assert!(learned(root));
    privacy::forget(root, Some("预算"));
    assert!(!learned(root));
    let epoch = typed_history::policy(root).unwrap().epoch;
    typed_history::record(
        root,
        "t3",
        "seg3",
        "这个方案下周再定。这个方案我来改。",
        "com.example.editor",
        epoch,
    )
    .unwrap();
    manage(root, "personalization_backfill", &json!({"source":"typed"})).unwrap();
    assert!(totals(root) > 0);
    privacy::forget(root, None);
    assert_eq!(totals(root), 0);
}

#[test]
fn deleted_or_closed_history_sources_stop_suggesting_immediately_without_touching_history() {
    use inputia_handy_runtime::{knowledge::KnowledgeStore, typed_history};
    let tmp = registered();
    let root = tmp.path();
    let store = KnowledgeStore::open(root).unwrap();
    let epoch = typed_history::set_capture(root, true).unwrap().epoch;
    let text = "这个预算下周再定。这个预算我来改。";
    let first =
        typed_history::record(root, "t1", "seg", text, "com.example.editor", epoch).unwrap();
    manage(root, "personalization_backfill", &json!({"source":"typed"})).unwrap();
    // 每轮按输入法的做法读取最新学习世代；世代被撤销推进时本轮作废、下一轮重查。
    let shows = |root: &Path| {
        let reply = query(root, ask(root, "这个", 3)).unwrap();
        reply["predictions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["text"] == "预算" && p["id"].as_str().unwrap().starts_with("learned:"))
    };
    assert!(shows(root));
    // 删除来源后不等后台核对：第一轮展示前核对即撤销并作废，之后不再出现。
    typed_history::delete(root, first["id"].as_str().unwrap()).unwrap();
    let invalidated = query(root, ask(root, "这个", 3)).unwrap();
    assert_eq!(invalidated["invalidated"], true, "{invalidated}");
    assert!(!shows(root));
    // 关闭来源同样立即失效，但只撤销学习，键入原文保留。
    let epoch = typed_history::policy(root).unwrap().epoch;
    typed_history::record(root, "t2", "seg2", text, "com.example.editor", epoch).unwrap();
    manage(root, "personalization_backfill", &json!({"source":"typed"})).unwrap();
    assert!(shows(root));
    store
        .dispatch(
            "update_source",
            json!({"id":"history:saved_snippet","enabled":false}),
            false,
        )
        .unwrap();
    let _ = query(root, ask(root, "这个", 3)).unwrap();
    assert!(!shows(root));
    assert_eq!(
        typed_history::count(root).unwrap(),
        1,
        "撤销学习不能删除键入原文"
    );
}

#[test]
fn voice_history_pairs_follow_source_deletion_and_revision_immediately() {
    use inputia_handy_runtime::{knowledge::KnowledgeStore, source::SOURCE_SCHEMA_VERSION};
    use rusqlite::{params, Connection};
    let tmp = registered();
    let root = tmp.path();
    KnowledgeStore::open(root).unwrap();
    let index = Connection::open(root.join("integration.db")).unwrap();
    index.execute_batch("CREATE TABLE integration_items(item_id TEXT,store_id TEXT,record_id TEXT,revision INTEGER,source_kind TEXT,content_type TEXT,created_at_ms INTEGER,snapshot TEXT)").unwrap();
    let source = Connection::open(root.join("history.db")).unwrap();
    source.execute_batch("CREATE TABLE transcription_history(id TEXT); CREATE TABLE unified_source_meta(singleton INTEGER,schema_version INTEGER,store_id TEXT,logical_name TEXT); CREATE TABLE unified_source_versions(record_id TEXT,revision INTEGER);").unwrap();
    source
        .execute(
            "INSERT INTO unified_source_meta VALUES(1,?1,'store-voice','history')",
            params![SOURCE_SCHEMA_VERSION],
        )
        .unwrap();
    for record in ["1", "2"] {
        index
            .execute(
                "INSERT INTO integration_items VALUES(?1,'store-voice',?1,1,'voice','text',1,?2)",
                params![
                    record,
                    json!({"text":"这个预算下周再定。这个预算我来改。","source_app":"com.example.editor"})
                        .to_string()
                ],
            )
            .unwrap();
        source
            .execute("INSERT INTO transcription_history VALUES(?1)", [record])
            .unwrap();
        source
            .execute("INSERT INTO unified_source_versions VALUES(?1,1)", [record])
            .unwrap();
    }
    manage(root, "personalization_backfill", &json!({"source":"voice"})).unwrap();
    let shows = |root: &Path| {
        let reply = query(root, ask(root, "这个", 3)).unwrap();
        reply["predictions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["text"] == "预算")
    };
    assert!(shows(root));
    // 一条源记录被删除（投影尚未同步）：只撤销它，另一条仍支撑该续写。
    source
        .execute("DELETE FROM transcription_history WHERE id='1'", [])
        .unwrap();
    let _ = query(root, ask(root, "这个", 3)).unwrap();
    assert!(shows(root));
    // 另一条被修改（修订推进）：已无有效来源，立即不再出现。
    source
        .execute(
            "UPDATE unified_source_versions SET revision=2 WHERE record_id='2'",
            [],
        )
        .unwrap();
    let _ = query(root, ask(root, "这个", 3)).unwrap();
    assert!(!shows(root));
    // 学习撤销不改动任何历史库内容。
    let kept: i64 = index
        .query_row("SELECT COUNT(*) FROM integration_items", [], |r| r.get(0))
        .unwrap();
    assert_eq!(kept, 2);
}
