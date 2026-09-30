use inputia_handy_runtime::personalization::{self as model, Candidate, Feedback, Query};
use serde_json::json;
use std::path::Path;

fn event(root: &Path, id: &str, text: &str) -> Feedback {
    Feedback {
        event_id: id.into(),
        context_id: "field".into(),
        input_code: "test".into(),
        schema_id: "schema-a".into(),
        text: text.into(),
        previous: String::new(),
        explicit_selection: true,
        original_rank: 1,
        source_app: "app.editor".into(),
        learning_epoch: model::policy(root).unwrap().epoch,
        operation: "accept".into(),
    }
}
fn query(root: &Path) -> Query {
    Query {
        input_code: "test".into(),
        schema_id: "schema-a".into(),
        context: String::new(),
        context_id: "field".into(),
        source_app: "app.editor".into(),
        learning_epoch: model::policy(root).unwrap().epoch,
        candidates: vec![],
        limit: 5,
    }
}
fn candidates(texts: &[&str]) -> Vec<Candidate> {
    texts
        .iter()
        .enumerate()
        .map(|(rank, text)| Candidate {
            id: format!("native-{rank}"),
            text: (*text).into(),
            base_rank: rank,
            consumed_len: 4,
            match_type: "exact".into(),
        })
        .collect()
}

#[test]
fn recall_is_exact_bounded_deduplicated_and_revalidated_after_undo_forget() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    for i in 0..8 {
        model::feedback(root, event(root, &format!("e{i}"), &format!("术语{i}"))).unwrap();
    }
    let request = query(root);
    let result = model::query(root, request.clone()).unwrap();
    let recalled = result["recalled_candidates"].as_array().unwrap();
    assert_eq!(recalled.len(), 5);
    assert!(recalled
        .iter()
        .all(|c| c["id"].as_str().unwrap().starts_with("learned:")
            && c["consumed_len"] == 4
            && c["match_type"] == "exact"));
    assert_eq!(result["ordered_ids"], json!([]));
    assert_eq!(result, model::query(root, request.clone()).unwrap());
    for (schema, code) in [
        ("schema-b", "test"),
        ("schema-a", "testx"),
        ("", "test"),
        ("schema-a", "中文"),
    ] {
        let mut q = request.clone();
        q.schema_id = schema.into();
        q.input_code = code.into();
        assert_eq!(
            model::query(root, q).unwrap()["recalled_candidates"],
            json!([])
        );
    }
    let mut q = request.clone();
    q.candidates = candidates(&["术语0"]);
    assert!(model::query(root, q).unwrap()["recalled_candidates"]
        .as_array()
        .unwrap()
        .iter()
        .all(|c| c["text"] != "术语0"));
    let mut undo = event(root, "e0", "术语0");
    undo.operation = "undo".into();
    model::feedback(root, undo).unwrap();
    model::manage(root, "personalization_forget", &json!({"text":"术语1"})).unwrap();
    let current = model::query(root, query(root)).unwrap();
    assert!(current["recalled_candidates"]
        .as_array()
        .unwrap()
        .iter()
        .all(|c| c["text"] != "术语0" && c["text"] != "术语1"));
    assert!(model::query(root, request).unwrap()["recalled_candidates"]
        .as_array()
        .unwrap()
        .is_empty());
    model::set_enabled(root, false).unwrap();
    assert_eq!(
        model::query(root, query(root)).unwrap()["recalled_candidates"],
        json!([])
    );
}

#[test]
fn recall_never_infers_a_code_from_imports_or_legacy_schema() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let mut legacy = event(root, "legacy", "旧记录");
    legacy.schema_id.clear();
    model::feedback(root, legacy).unwrap();
    let imported = event(root, "imported", "导入正文");
    model::feedback(root, imported).unwrap();
    let db = rusqlite::Connection::open(root.join("knowledge/personalization.sqlite")).unwrap();
    db.execute(
        "UPDATE evidence SET origin='unknown-import' WHERE event_id='imported'",
        [],
    )
    .unwrap();
    assert_eq!(
        model::query(root, query(root)).unwrap()["recalled_candidates"],
        json!([])
    );
    let mut private = query(root);
    private.source_app = "com.1password.1password".into();
    assert_eq!(
        model::query(root, private).unwrap()["recalled_candidates"],
        json!([])
    );
}

#[test]
fn rejection_is_scoped_to_schema_code_app_and_original_context() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let mut rejected = event(root, "reject", "候选甲");
    rejected.previous = "原上下文".into();
    rejected.operation = "reject".into();
    model::feedback(root, rejected).unwrap();
    let mut same = query(root);
    same.context = "前面的原上下文".into();
    same.candidates = candidates(&["候选甲", "候选乙"]);
    assert_eq!(
        model::query(root, same.clone()).unwrap()["ordered_ids"][0],
        "native-1"
    );
    for i in 0..4 {
        let mut other = same.clone();
        match i {
            0 => other.schema_id = "schema-b".into(),
            1 => other.input_code = "other".into(),
            2 => other.source_app = "app.chat".into(),
            _ => other.context = "另一个上下文".into(),
        }
        assert_eq!(
            model::query(root, other).unwrap()["ordered_ids"][0],
            "native-0"
        );
    }
}

#[test]
fn app_preference_keeps_a_weaker_shared_global_prior() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    for i in 0..12 {
        model::feedback(root, event(root, &format!("a{i}"), "领域甲")).unwrap();
        let mut b = event(root, &format!("b{i}"), "领域乙");
        b.source_app = "app.chat".into();
        model::feedback(root, b).unwrap();
    }
    let mut q = query(root);
    q.candidates = candidates(&["领域甲", "领域乙"]);
    assert_eq!(
        model::query(root, q.clone()).unwrap()["ordered_ids"][0],
        "native-0"
    );
    q.source_app = "app.chat".into();
    assert_eq!(model::query(root, q).unwrap()["ordered_ids"][0], "native-1");
    let mut shared = query(root);
    shared.source_app = "app.new".into();
    shared.candidates = candidates(&["陌生词", "领域甲"]);
    assert_eq!(
        model::query(root, shared).unwrap()["ordered_ids"][0],
        "native-1"
    );
}

#[test]
fn explicit_first_choice_and_other_choice_both_learn_rephrased_context() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let training = [("候选甲", "海岛旅行", 0), ("候选乙", "矿物研究", 1)];
    for (text, previous, rank) in training {
        for i in 0..6 {
            let mut f = event(root, &format!("{text}-{i}"), text);
            f.previous = previous.into();
            f.original_rank = rank;
            model::feedback(root, f).unwrap();
        }
    }
    let count = model::manage(root, "personalization_status", &json!({})).unwrap()["counts"]
        ["events"]
        .clone();
    for (context, expected) in [
        ("我们讨论海岛旅行的安排", "native-0"),
        ("开始矿物研究的分析", "native-1"),
    ] {
        let mut q = query(root);
        q.context = context.into();
        q.candidates = candidates(&["候选甲", "候选乙"]);
        assert_eq!(model::query(root, q).unwrap()["ordered_ids"][0], expected);
    }
    assert_eq!(
        model::manage(root, "personalization_status", &json!({})).unwrap()["counts"]["events"],
        count
    );
}

#[test]
fn legacy_database_migrates_without_losing_evidence_or_gaining_recall() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::create_dir_all(root.join("knowledge")).unwrap();
    let db = rusqlite::Connection::open(root.join("knowledge/personalization.sqlite")).unwrap();
    db.execute_batch("CREATE TABLE evidence(event_id TEXT PRIMARY KEY,fingerprint TEXT NOT NULL,text TEXT NOT NULL,normalized TEXT NOT NULL,previous TEXT NOT NULL,code TEXT NOT NULL,weight REAL NOT NULL,created REAL NOT NULL,undone INTEGER NOT NULL DEFAULT 0,origin TEXT NOT NULL DEFAULT '',origin_revision TEXT NOT NULL DEFAULT ''); INSERT INTO evidence(event_id,fingerprint,text,normalized,previous,code,weight,created) VALUES('legacy','legacy','保留词','保留词','','test',10,strftime('%s','now'));").unwrap();
    let result = model::query(root, query(root)).unwrap();
    assert_eq!(result["recalled_candidates"], json!([]));
    assert_eq!(
        db.query_row("SELECT count(*) FROM evidence", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    let mut q = query(root);
    q.candidates = candidates(&["默认词", "保留词"]);
    assert_eq!(model::query(root, q).unwrap()["ordered_ids"][0], "native-1");
    assert_eq!(
        db.query_row("SELECT source_app || schema_id FROM evidence", [], |r| {
            r.get::<_, String>(0)
        })
        .unwrap(),
        ""
    );
}

#[test]
fn experimental_candidate_model_requires_explicit_opt_in() {
    assert!(!model::experimental_candidate_model_enabled(None));
    for value in ["", "0", "true", "yes", " 1", "1 "] {
        assert!(!model::experimental_candidate_model_enabled(Some(value)));
    }
    assert!(model::experimental_candidate_model_enabled(Some("1")));
}

#[test]
fn recalled_identity_survives_native_dedup_and_reject_has_contextual_expiry() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    for i in 0..7 {
        model::feedback(root, event(root, &format!("word-{i}"), &format!("记忆{i}"))).unwrap();
    }
    let mut displayed = query(root);
    displayed.candidates = candidates(&["记忆0", "记忆1"]);
    let visible = model::query(root, displayed).unwrap();
    let admission = model::query(root, query(root)).unwrap();
    for candidate in visible["recalled_candidates"].as_array().unwrap() {
        assert!(admission["recalled_candidates"]
            .as_array()
            .unwrap()
            .iter()
            .any(
                |current| current["id"] == candidate["id"] && current["text"] == candidate["text"]
            ));
    }
    let mut rejection = event(root, "rejected-word", "记忆6");
    rejection.operation = "reject".into();
    model::feedback(root, rejection).unwrap();
    let hidden = model::query(root, query(root)).unwrap();
    assert!(hidden["recalled_candidates"]
        .as_array()
        .unwrap()
        .iter()
        .all(|c| c["text"] != "记忆6"));
    let db = rusqlite::Connection::open(root.join("knowledge/personalization.sqlite")).unwrap();
    db.execute(
        "UPDATE evidence SET created=created-15*86400 WHERE event_id='rejected-word'",
        [],
    )
    .unwrap();
    let expired = model::query(root, query(root)).unwrap();
    assert!(expired["recalled_candidates"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["text"] == "记忆6"));
    let mut uppercase = query(root);
    uppercase.input_code = "TEST".into();
    assert_eq!(
        model::query(root, uppercase).unwrap()["recalled_candidates"],
        json!([])
    );
}

#[test]
fn saturated_frequency_leaves_room_for_distant_context_and_stops_at_sentence_boundary() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    // 训练词与独立质量回放不同；两项频率都达到上限，评估阶段不再学习。
    for (text, previous) in [("解释", "园艺修剪"), ("轨迹", "星系观测")] {
        for i in 0..80 {
            let mut f = event(root, &format!("saturated-{text}-{i}"), text);
            f.previous = previous.into();
            model::feedback(root, f).unwrap();
        }
    }
    let before = model::manage(root, "personalization_status", &json!({})).unwrap()["counts"]
        ["events"]
        .clone();
    for (context, expected) in [
        (
            "星系观测的资料经过大家反复整理以后现在准备开始进一步讨论",
            "native-1",
        ),
        (
            "结合星系观测结果接下来安排人员整理数据并编写说明",
            "native-1",
        ),
        ("", "native-0"),
        ("完全陌生的语境", "native-0"),
        ("星系观测。接下来讨论无关的事情", "native-0"),
        ("星系观测\n接下来讨论无关的事情", "native-0"),
    ] {
        let mut q = query(root);
        q.context = context.into();
        q.candidates = candidates(&["解释", "轨迹"]);
        assert_eq!(
            model::query(root, q).unwrap()["ordered_ids"][0],
            expected,
            "{context}"
        );
    }
    assert_eq!(
        model::manage(root, "personalization_status", &json!({})).unwrap()["counts"]["events"],
        before
    );
}

#[test]
fn scoped_rejection_cannot_be_overwhelmed_by_saturated_frequency_or_context() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    for i in 0..80 {
        let mut f = event(root, &format!("frequent-{i}"), "常用乙");
        f.previous = "原始语境".into();
        model::feedback(root, f).unwrap();
    }
    let mut q = query(root);
    q.context = "原始语境".into();
    q.candidates = candidates(&["默认甲", "常用乙"]);
    assert_eq!(
        model::query(root, q.clone()).unwrap()["ordered_ids"][0],
        "native-1"
    );
    let mut rejection = event(root, "scoped-rejection", "常用乙");
    rejection.previous = "原始语境".into();
    rejection.operation = "reject".into();
    model::feedback(root, rejection).unwrap();
    assert_eq!(
        model::query(root, q.clone()).unwrap()["ordered_ids"][0],
        "native-0"
    );
    let mut renewed = event(root, "renewed-accept", "常用乙");
    renewed.previous = "原始语境".into();
    model::feedback(root, renewed).unwrap();
    assert_eq!(model::query(root, q).unwrap()["ordered_ids"][0], "native-1");
}
