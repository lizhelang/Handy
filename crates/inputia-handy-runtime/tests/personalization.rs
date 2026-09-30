use inputia_handy_runtime::{knowledge::KnowledgeStore, personalization::*, typed_history};
use serde_json::json;
use std::path::Path;
fn accept(root: &Path, id: &str, text: &str, previous: &str) -> Feedback {
    Feedback {
        event_id: id.into(),
        context_id: "target1".into(),
        schema_id: String::new(),
        input_code: "ba".into(),
        text: text.into(),
        previous: previous.into(),
        explicit_selection: true,
        original_rank: 4,
        source_app: "com.example.editor".into(),
        learning_epoch: policy(root).unwrap().epoch,
        operation: "accept".into(),
    }
}
fn query_for(root: &Path, context: &str, code: &str) -> Query {
    Query {
        schema_id: String::new(),
        input_code: code.into(),
        context: context.into(),
        context_id: "target1".into(),
        source_app: "com.example.editor".into(),
        learning_epoch: policy(root).unwrap().epoch,
        candidates: ["吧", "八", "巴", "把", "爸"]
            .iter()
            .enumerate()
            .map(|(i, t)| Candidate {
                id: i.to_string(),
                text: t.to_string(),
                base_rank: i,
                consumed_len: 2,
                match_type: "exact".into(),
            })
            .collect(),
        limit: 10,
    }
}
#[test]
fn explicit_hotwords_win_over_learned_order_without_crossing_native_groups() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    for i in 0..20 {
        feedback(
            root,
            accept(root, &format!("hotword-learned-{i}"), "爸", ""),
        )
        .unwrap();
    }
    let q = query_for(root, "", "ba");
    let learned = query(root, q.clone()).unwrap();
    let ids: Vec<String> = serde_json::from_value(learned["ordered_ids"].clone()).unwrap();
    assert_eq!(ids[0], "4");
    let promoted = prioritize_explicit_candidates(&q.candidates, &ids, &["把".into()]).unwrap();
    assert_eq!(promoted[0], "3");
    assert_eq!(promoted[1], "4");
    assert_eq!(
        prioritize_explicit_candidates(&q.candidates, &ids, &[]).unwrap(),
        ids
    );
    assert_eq!(
        prioritize_explicit_candidates(&q.candidates, &ids, &["不存在的新专名".into()]).unwrap(),
        ids
    );
    let mut candidates = q.candidates.clone();
    candidates[3].consumed_len = 1;
    let untouched = prioritize_explicit_candidates(
        &candidates,
        &["0".into(), "1".into(), "2".into(), "3".into(), "4".into()],
        &["把".into()],
    )
    .unwrap();
    assert_eq!(untouched[3], "3");
}
#[test]
fn repeat_choices_context_decay_and_eligibility_are_real() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let cold = query(root, query_for(root, "", "ba")).unwrap();
    assert_eq!(cold["ordered_ids"][0], "0");
    feedback(root, accept(root, "first", "爸", "")).unwrap();
    assert_ne!(
        query(root, query_for(root, "", "ba")).unwrap()["ordered_ids"][0],
        "4"
    );
    for i in 0..20 {
        feedback(root, accept(root, &format!("r{i}"), "爸", "")).unwrap();
    }
    assert_eq!(
        query(root, query_for(root, "", "ba")).unwrap()["ordered_ids"][0],
        "4"
    );
    let db = rusqlite::Connection::open(root.join("knowledge/personalization.sqlite")).unwrap();
    db.execute("UPDATE evidence SET created=created-86400*365", [])
        .unwrap();
    assert_eq!(
        query(root, query_for(root, "", "ba")).unwrap()["ordered_ids"][0],
        "0"
    );
    for i in 0..3 {
        feedback(root, accept(root, &format!("c{i}"), "把", "请")).unwrap();
    }
    assert_eq!(
        query(root, query_for(root, "请", "ba")).unwrap()["ordered_ids"][0],
        "3"
    );
    let mut q = query_for(root, "请", "ba");
    q.candidates[3].consumed_len = 1;
    assert_eq!(query(root, q).unwrap()["ordered_ids"][3], "3");
    let mut unknown = query_for(root, "陌生", "zzzz");
    unknown.candidates.clear();
    let result = query(root, unknown).unwrap();
    assert_eq!(result["ordered_ids"], json!([]));
    assert_eq!(result["predictions"], json!([]));
}
#[test]
fn undo_replay_forget_clear_and_closed_policy() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let f = accept(root, "original", "把", "请");
    feedback(root, f.clone()).unwrap();
    assert_eq!(feedback(root, f.clone()).unwrap()["duplicate"], true);
    let mut bad = f.clone();
    bad.operation = "undo".into();
    bad.previous = "其他".into();
    assert!(feedback(root, bad).is_err());
    let mut undo = f.clone();
    undo.operation = "undo".into();
    assert_eq!(feedback(root, undo.clone()).unwrap()["undone"], 1);
    assert_eq!(feedback(root, undo).unwrap()["undone"], 0);
    assert!(feedback(root, f.clone()).is_err());
    for i in 0..5 {
        feedback(root, accept(root, &format!("k{i}"), "把", "请")).unwrap();
    }
    assert!(
        !query(root, query_for(root, "请", "")).unwrap()["predictions"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    manage(root, "personalization_forget", &json!({"text":"把"})).unwrap();
    assert!(
        query(root, query_for(root, "请", "")).unwrap()["predictions"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(feedback(root, f).is_err());
    let before = policy(root).unwrap();
    set_enabled(root, false).unwrap();
    let f = accept(root, "disabled", "爸", "");
    assert!(feedback(root, f).is_err());
    set_enabled(root, true).unwrap();
    assert!(policy(root).unwrap().epoch > before.epoch);
    manage(root, "personalization_clear", &json!({})).unwrap();
    assert_eq!(
        manage(root, "personalization_status", &json!({})).unwrap()["counts"]["events"],
        0
    );
}
#[test]
fn zero_input_predictions_english_boundaries_and_privacy() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    feedback(root, accept(root, "bigram", "把", "请")).unwrap();
    assert_eq!(
        query(root, query_for(root, "请", "")).unwrap()["predictions"][0]["text"],
        "把"
    );
    assert_eq!(
        query(root, query_for(root, "不相干", "")).unwrap()["predictions"],
        json!([])
    );
    feedback(root, accept(root, "english", "world", "hello")).unwrap();
    assert_eq!(
        query(root, query_for(root, "HELLO", "")).unwrap()["predictions"][0]["text"],
        "world"
    );
    assert_eq!(
        query(root, query_for(root, "shello", "")).unwrap()["predictions"],
        json!([])
    );
    let mut secret = accept(root, "secret", "秘密", "");
    secret.source_app = "com.1password.1password".into();
    assert!(feedback(root, secret).is_err());
    assert!(!typed_history::policy(root).unwrap().enabled);
    assert!(policy(root).unwrap().enabled);
}
#[test]
fn explicit_backfill_prefix_revision_delete_and_forget() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    KnowledgeStore::open(root).unwrap();
    let epoch = typed_history::set_capture(root, true).unwrap().epoch;
    let recorded = typed_history::record(
        root,
        "t1",
        "seg",
        "今天下午开会",
        "com.example.editor",
        epoch,
    )
    .unwrap();
    let imported = manage(
        root,
        "personalization_backfill",
        &json!({"source":"typed","limit":20}),
    )
    .unwrap();
    assert_eq!(imported["backfill_summary"]["imported"], 1);
    assert_eq!(
        manage(root, "personalization_backfill", &json!({"source":"typed"})).unwrap()
            ["backfill_summary"]["skipped"],
        1
    );
    assert_eq!(
        query(root, query_for(root, "今天", "")).unwrap()["predictions"][0]["text"],
        "下午开会"
    );
    feedback(root, accept(root, "online", "独立", "前文")).unwrap();
    typed_history::delete(root, recorded["id"].as_str().unwrap()).unwrap();
    assert_eq!(
        query(root, query_for(root, "今天", "")).unwrap()["predictions"],
        json!([])
    );
    assert_eq!(
        query(root, query_for(root, "前文", "")).unwrap()["predictions"][0]["text"],
        "独立"
    );
    assert_eq!(
        manage(root, "personalization_status", &json!({})).unwrap()["counts"]["imports"],
        0
    );
    let epoch = typed_history::policy(root).unwrap().epoch;
    typed_history::record(
        root,
        "t2",
        "seg2",
        "明天下午开会",
        "com.example.editor",
        epoch,
    )
    .unwrap();
    manage(root, "personalization_backfill", &json!({"source":"typed"})).unwrap();
    manage(
        root,
        "personalization_forget",
        &json!({"text":"明天下午开会"}),
    )
    .unwrap();
    manage(root, "personalization_backfill", &json!({"source":"typed"})).unwrap();
    assert_eq!(
        query(root, query_for(root, "明天", "")).unwrap()["predictions"],
        json!([])
    );
}
#[test]
fn persistent_model_and_local_query_p95() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    for i in 0..100 {
        feedback(root, accept(root, &format!("e{i}"), "爸", "")).unwrap();
    }
    assert_eq!(policy(root).unwrap().epoch, 1);
    let mut timings = vec![];
    for _ in 0..100 {
        let start = std::time::Instant::now();
        let result = query(root, query_for(root, "", "ba")).unwrap();
        assert_eq!(result["ordered_ids"][0], "4");
        timings.push(start.elapsed().as_micros());
    }
    timings.sort();
    eprintln!(
        "personalization debug benchmark: 100 events, 5 candidates, n=100, p95={} us",
        timings[94]
    );
    assert!(
        timings[94] < 100_000,
        "本地查询p95超过100ms: {}us",
        timings[94]
    );
}
#[test]
fn backfill_two_hundred_records_resumes_and_deletion_propagates() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    KnowledgeStore::open(root).unwrap();
    let epoch = typed_history::set_capture(root, true).unwrap().epoch;
    let mut first = None;
    for i in 0..205 {
        let item = typed_history::record(
            root,
            &format!("event{i}"),
            &format!("segment{i}"),
            &format!("编号{i:03}下午开会"),
            "com.example.editor",
            epoch,
        )
        .unwrap();
        if i == 123 {
            first = Some(item);
        }
    }
    let batch = manage(
        root,
        "personalization_backfill",
        &json!({"source":"typed","limit":200}),
    )
    .unwrap();
    assert_eq!(batch["backfill_summary"]["imported"], 200);
    assert_eq!(batch["backfill_summary"]["has_more"], true);
    let next = manage(
        root,
        "personalization_backfill",
        &json!({"source":"typed","limit":500,"cursor":batch["backfill_summary"]["next_cursor"]}),
    )
    .unwrap();
    assert_eq!(next["backfill_summary"]["imported"], 5);
    assert_eq!(next["backfill_summary"]["has_more"], false);
    assert_eq!(next["counts"]["imports"], 205);
    let mut measurements = Vec::new();
    for _ in 0..20 {
        let start = std::time::Instant::now();
        let result = query(root, query_for(root, "编号", "")).unwrap();
        measurements.push(start.elapsed().as_micros());
        assert!(result["skipped_unverified_sources"].as_u64().unwrap() > 0);
    }
    measurements.sort();
    eprintln!(
        "personalization debug benchmark: 205 imported records, matching prefix, n=20, p95={} us",
        measurements[18]
    );

    let repeated = manage(
        root,
        "personalization_backfill",
        &json!({"source":"typed","limit":500}),
    )
    .unwrap();
    assert_eq!(repeated["backfill_summary"]["imported"], 0);
    assert_eq!(repeated["backfill_summary"]["skipped"], 205);
    assert_eq!(
        query(root, query_for(root, "编号123", "")).unwrap()["predictions"][0]["text"],
        "下午开会"
    );
    typed_history::delete(root, first.unwrap()["id"].as_str().unwrap()).unwrap();
    assert_eq!(
        query(root, query_for(root, "编号123", "")).unwrap()["predictions"],
        json!([])
    );
    assert_eq!(
        manage(root, "personalization_status", &json!({})).unwrap()["counts"]["imports"],
        204
    );
}
#[test]
fn changed_source_retracts_only_its_evidence_and_preserves_case() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    KnowledgeStore::open(root).unwrap();
    let epoch = typed_history::set_capture(root, true).unwrap().epoch;
    typed_history::record(root, "t1", "s", "Hello World", "com.example.editor", epoch).unwrap();
    manage(root, "personalization_backfill", &json!({"source":"typed"})).unwrap();
    assert_eq!(
        query(root, query_for(root, "hello", "")).unwrap()["predictions"][0]["text"],
        "World"
    );
    assert_eq!(
        query(root, query_for(root, "hel", "")).unwrap()["predictions"],
        json!([])
    );
    typed_history::record(root, "t2", "s", " Again", "com.example.editor", epoch).unwrap();
    assert_eq!(
        query(root, query_for(root, "hello", "")).unwrap()["predictions"],
        json!([])
    );
    manage(root, "personalization_backfill", &json!({"source":"typed"})).unwrap();
    assert_eq!(
        query(root, query_for(root, "hello", "")).unwrap()["predictions"][0]["text"],
        "World Again"
    );
    let store = KnowledgeStore::open_readonly(root).unwrap();
    assert_eq!(
        store
            .dispatch("search", json!({"query":"Hello"}), true)
            .unwrap()["items"],
        json!([])
    );
}
#[test]
fn explicit_selection_outweighs_default_and_reject_does_not_reinforce() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let mut chosen = accept(a.path(), "selected", "爸", "");
    feedback(a.path(), chosen.clone()).unwrap();
    chosen.learning_epoch = policy(b.path()).unwrap().epoch;
    chosen.explicit_selection = false;
    chosen.original_rank = 0;
    feedback(b.path(), chosen).unwrap();
    let qa = query(a.path(), query_for(a.path(), "", "ba")).unwrap();
    let qb = query(b.path(), query_for(b.path(), "", "ba")).unwrap();
    let position = |r: &serde_json::Value| {
        r["ordered_ids"]
            .as_array()
            .unwrap()
            .iter()
            .position(|v| v == "4")
            .unwrap()
    };
    assert!(position(&qa) < position(&qb));
    let mut rejection = accept(a.path(), "reject", "爸", "");
    rejection.operation = "reject".into();
    feedback(a.path(), rejection).unwrap();
    assert!(position(&query(a.path(), query_for(a.path(), "", "ba")).unwrap()) >= position(&qa));
    let mut cancel = accept(a.path(), "cancel", "把", "");
    cancel.operation = "cancel".into();
    assert!(feedback(a.path(), cancel).is_err());
}
#[test]
fn voice_and_clipboard_backfill_validate_live_sources_and_flags() {
    use inputia_handy_runtime::source::SOURCE_SCHEMA_VERSION;
    use rusqlite::{params, Connection};
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let store = KnowledgeStore::open(root).unwrap();
    let index = Connection::open(root.join("integration.db")).unwrap();
    index.execute_batch("CREATE TABLE integration_items(item_id TEXT,store_id TEXT,record_id TEXT,revision INTEGER,source_kind TEXT,content_type TEXT,created_at_ms INTEGER,snapshot TEXT)").unwrap();
    for (kind, file, table, logical, text) in [
        (
            "voice",
            "history.db",
            "transcription_history",
            "history",
            "语音项目讨论",
        ),
        (
            "clipboard",
            "clipboard.db",
            "clipboard_history",
            "clipboard",
            "剪贴资料摘要",
        ),
    ] {
        index
            .execute(
                "INSERT INTO integration_items VALUES(?1,?2,'1',1,?1,'text',1,?3)",
                params![
                    kind,
                    format!("store-{kind}"),
                    json!({"text":text,"title":kind,"source_app":"com.example.editor"}).to_string()
                ],
            )
            .unwrap();
        let source = Connection::open(root.join(file)).unwrap();
        source.execute_batch(&format!("CREATE TABLE {table}(id TEXT); INSERT INTO {table} VALUES('1'); CREATE TABLE unified_source_meta(singleton INTEGER,schema_version INTEGER,store_id TEXT,logical_name TEXT); CREATE TABLE unified_source_versions(record_id TEXT,revision INTEGER); INSERT INTO unified_source_versions VALUES('1',1);")).unwrap();
        source
            .execute(
                "INSERT INTO unified_source_meta VALUES(1,?1,?2,?3)",
                params![SOURCE_SCHEMA_VERSION, format!("store-{kind}"), logical],
            )
            .unwrap();
        assert_eq!(
            manage(root, "personalization_backfill", &json!({"source":kind})).unwrap()
                ["backfill_summary"]["imported"],
            1
        );
    }
    assert_eq!(
        query(root, query_for(root, "语音", "")).unwrap()["predictions"][0]["text"],
        "项目讨论"
    );
    assert_eq!(
        query(root, query_for(root, "剪贴", "")).unwrap()["predictions"][0]["text"],
        "资料摘要"
    );
    assert_eq!(
        store
            .dispatch("search", json!({"query":"资料"}), true)
            .unwrap()["items"],
        json!([])
    );
    store
        .dispatch(
            "update_source",
            json!({"id":"history:clipboard","enabled":false}),
            false,
        )
        .unwrap();
    assert!(manage(
        root,
        "personalization_backfill",
        &json!({"source":"clipboard"})
    )
    .is_err());
    assert_eq!(
        query(root, query_for(root, "剪贴", "")).unwrap()["predictions"],
        json!([])
    );
    Connection::open(root.join("history.db"))
        .unwrap()
        .execute("DELETE FROM transcription_history", [])
        .unwrap();
    assert_eq!(
        query(root, query_for(root, "语音", "")).unwrap()["predictions"],
        json!([])
    );
}
#[test]
fn packaged_general_lexicon_cold_start_context_ranking_and_indexed_latency() {
    let root_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../src-tauri/resources/personalization/base-lexicon.tsv");
    assert!(root_dir.is_file(), "随应用打包的通用词库必须存在");
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let installed = install_base_lexicon(root, &root_dir).unwrap();
    assert_eq!(installed["changed"], true);
    assert!(installed["entries"].as_u64().unwrap() > 200_000);
    assert_eq!(
        install_base_lexicon(root, &root_dir).unwrap()["changed"],
        false
    );
    let prefixes = [
        "对",
        "我觉得",
        "图书",
        "数据",
        "今天",
        "明天",
        "你好",
        "谢谢",
        "工作",
        "学习",
        "我们",
        "可以",
    ];
    for prefix in prefixes {
        let result = query(root, query_for(root, prefix, "")).unwrap();
        assert!(
            !result["predictions"].as_array().unwrap().is_empty(),
            "真实词库冷启动未覆盖{prefix}: {result}"
        );
        eprintln!("cold prefix {prefix}: {}", result["predictions"][0]["text"]);
    }
    for (context, code, texts, expected) in [
        ("对", "ba", ["八", "把", "吧"], "吧"),
        ("图书", "guan", ["官", "关", "馆"], "馆"),
        ("数据", "ku", ["苦", "酷", "库"], "库"),
    ] {
        let mut q = query_for(root, context, code);
        q.candidates = texts
            .into_iter()
            .enumerate()
            .map(|(i, text)| Candidate {
                id: text.to_string(),
                text: text.to_string(),
                base_rank: i,
                consumed_len: code.len(),
                match_type: "exact".into(),
            })
            .collect();
        let result = query(root, q).unwrap();
        assert_eq!(
            result["ordered_ids"][0], expected,
            "{context}/{code}: {result}"
        );
    }
    let state = manage(root, "personalization_status", &json!({})).unwrap();
    assert_eq!(state["counts"]["events"], 0);
    assert_eq!(state["counts"]["imports"], 0);
    let mut f = accept(root, "new-phrase", "先验证再发布", "我觉得");
    f.input_code = String::new();
    feedback(root, f).unwrap();
    assert_eq!(
        query(root, query_for(root, "我觉得", "")).unwrap()["predictions"][0]["text"],
        "先验证再发布"
    );
    let db = rusqlite::Connection::open(root.join("knowledge/personalization.sqlite")).unwrap();
    let mut stmt=db.prepare("EXPLAIN QUERY PLAN SELECT text FROM base_phrases WHERE normalized>?1 AND normalized<?2 ORDER BY frequency DESC LIMIT 100").unwrap();
    let plans: Vec<String> = stmt
        .query_map(["我", "我\u{10ffff}"], |r| r.get(3))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(
        plans
            .iter()
            .any(|p| p.contains("SEARCH base_phrases USING INDEX")),
        "{plans:?}"
    );
    assert!(
        !plans.iter().any(|p| p.contains("SCAN base_phrases")),
        "{plans:?}"
    );
    let mut measurements = Vec::new();
    for i in 0..100 {
        let start = std::time::Instant::now();
        query(root, query_for(root, prefixes[i % prefixes.len()], "")).unwrap();
        measurements.push(start.elapsed().as_micros());
    }
    measurements.sort();
    eprintln!("personalization real packaged lexicon {} entries, 12 cold prefixes, n=100, debug p95={} us; query plan={plans:?}",installed["entries"],measurements[94]);
    assert!(measurements[94] < 100_000, "通用词库查询p95超过100ms");
}
#[test]
fn undo_arriving_before_accept_never_learns_late_event() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let accepted = accept(root, "late-event", "吧", "对");
    let mut undone = accepted.clone();
    undone.operation = "undo".into();
    assert_eq!(feedback(root, undone.clone()).unwrap()["pending"], true);
    assert_eq!(feedback(root, undone).unwrap()["undone"], 0);
    assert!(feedback(root, accepted).is_err());
    assert_eq!(
        manage(root, "personalization_status", &json!({})).unwrap()["counts"]["events"],
        0
    );
}
#[test]
fn status_never_discloses_deleted_sources_and_reconcile_rotates_with_budget() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    KnowledgeStore::open(root).unwrap();
    let capture = typed_history::set_capture(root, true).unwrap();
    let mut ids = Vec::new();
    for i in 0..5 {
        ids.push(
            typed_history::record(
                root,
                &format!("s{i}"),
                &format!("segment{i}"),
                &format!("需删除的私密片段{i}"),
                "com.example.editor",
                capture.epoch,
            )
            .unwrap()["id"]
                .as_str()
                .unwrap()
                .to_string(),
        );
    }
    manage(root, "personalization_backfill", &json!({"source":"typed"})).unwrap();
    let before = policy(root).unwrap().epoch;
    typed_history::delete(root, &ids[0]).unwrap();
    let status = manage(root, "personalization_status", &json!({})).unwrap();
    assert!(status["learned_terms"]
        .as_array()
        .unwrap()
        .iter()
        .all(|v| v["text"] != "需删除的私密片段0"));
    assert_eq!(status["counts"]["imports"], 4);
    assert!(status["epoch"].as_u64().unwrap() > before);
    typed_history::clear(root).unwrap();
    let first = reconcile_imports(root, 2).unwrap();
    assert_eq!(first["checked"], 2);
    assert_eq!(first["removed"], 2);
    let second = reconcile_imports(root, 2).unwrap();
    assert_eq!(second["checked"], 2);
    assert_eq!(second["removed"], 2);
    assert_eq!(second["remaining"], 0);
    assert_eq!(
        manage(root, "personalization_status", &json!({})).unwrap()["learned_terms"],
        json!([])
    );
}

/// 由下面父测试以两个独立操作系统进程调用，不依赖本进程内存。
#[test]
fn separate_process_persistence_child() {
    let Some(root) = std::env::var_os("INPUTIA_PERSONAL_TEST_ROOT") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    match std::env::var("INPUTIA_PERSONAL_TEST_PHASE")
        .unwrap()
        .as_str()
    {
        "train" => {
            for i in 0..12 {
                feedback(&root, accept(&root, &format!("process-{i}"), "爸", "我")).unwrap();
            }
            assert_eq!(
                manage(&root, "personalization_status", &json!({})).unwrap()["counts"]["events"],
                12
            );
        }
        "verify" => {
            assert_eq!(policy(&root).unwrap().epoch, 1);
            assert_eq!(
                query(&root, query_for(&root, "这是我", "ba")).unwrap()["ordered_ids"][0],
                "4"
            );
            assert_eq!(
                manage(&root, "personalization_status", &json!({})).unwrap()["counts"]["events"],
                12
            );
        }
        _ => panic!("未知子进程验证阶段"),
    }
}

#[test]
fn learned_preferences_survive_process_exit_and_database_reopen() {
    let temp = tempfile::tempdir().unwrap();
    for phase in ["train", "verify"] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "separate_process_persistence_child",
                "--nocapture",
            ])
            .env("INPUTIA_PERSONAL_TEST_ROOT", temp.path())
            .env("INPUTIA_PERSONAL_TEST_PHASE", phase)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{phase}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert_eq!(
        manage(temp.path(), "personalization_status", &json!({})).unwrap()["counts"]["events"],
        12
    );
}

#[test]
fn cancellation_changes_neither_evidence_nor_rank_and_match_groups_stay_eligible() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let before = manage(root, "personalization_status", &json!({})).unwrap();
    let rank_before = query(root, query_for(root, "请", "ba")).unwrap()["ordered_ids"].clone();
    let mut cancel = accept(root, "cancelled", "把", "请");
    cancel.operation = "cancel".into();
    assert!(feedback(root, cancel).is_err());
    assert_eq!(
        manage(root, "personalization_status", &json!({})).unwrap()["counts"],
        before["counts"]
    );
    assert_eq!(
        query(root, query_for(root, "请", "ba")).unwrap()["ordered_ids"],
        rank_before
    );
    for i in 0..20 {
        feedback(root, accept(root, &format!("short-{i}"), "爸", "我")).unwrap();
    }
    for (consumed, matching) in [(1, "exact"), (2, "completion"), (1, "abbreviation")] {
        let mut request = query_for(root, "我", "ba");
        request.candidates[4].consumed_len = consumed;
        request.candidates[4].match_type = matching.into();
        let result = query(root, request).unwrap();
        assert_eq!(
            result["ordered_ids"][4], "4",
            "跨资格分组挤占原槽位：{consumed}/{matching}"
        );
        let ids = result["ordered_ids"].as_array().unwrap();
        assert_eq!(ids.len(), 5);
        assert_eq!(
            ids.iter()
                .map(|v| v.as_str().unwrap())
                .collect::<std::collections::HashSet<_>>()
                .len(),
            5
        );
    }
}

#[test]
fn fixed_candidate_pool_replay_has_separate_training_and_evaluation_phases() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    // 这是预先固定的机制回放池，不是实时Rime输出，也不是用户留出语料。
    // 训练只提交下面的反馈事件，不查看/根据测试查询继续训练。
    let training = [
        ("爸", "我", 4usize, 6usize),
        ("把", "请", 3, 6),
        ("八", "数字", 1, 6),
        ("吧", "好", 0, 8),
    ];
    let mut count = 0usize;
    for (word, previous, rank, repetitions) in training {
        for i in 0..repetitions {
            let mut event = accept(root, &format!("replay-train-{word}-{i}"), word, previous);
            event.original_rank = rank;
            feedback(root, event).unwrap();
            count += 1;
        }
    }
    let test_cases = [
        ("这是我", "爸"),
        ("还有我", "爸"),
        ("麻烦请", "把"),
        ("劳驾请", "把"),
        ("这个数字", "八"),
        ("那个数字", "八"),
        ("那就好", "吧"),
        ("这样好", "吧"),
    ];
    let mut baseline_top1 = 0;
    let mut baseline_top3 = 0;
    let mut learned_top1 = 0;
    let mut learned_top3 = 0;
    let before =
        manage(root, "personalization_status", &json!({})).unwrap()["counts"]["events"].clone();
    for (context, expected) in test_cases {
        let request = query_for(root, context, "ba");
        let target = request
            .candidates
            .iter()
            .find(|c| c.text == expected)
            .unwrap()
            .id
            .clone();
        let baseline: Vec<_> = request.candidates.iter().map(|c| c.id.clone()).collect();
        baseline_top1 += usize::from(baseline[0] == target);
        baseline_top3 += usize::from(baseline[..3].contains(&target));
        let result = query(root, request).unwrap();
        let ordered = result["ordered_ids"].as_array().unwrap();
        learned_top1 += usize::from(ordered[0] == target);
        learned_top3 += usize::from(ordered[..3].iter().any(|id| id == &target));
    }
    assert_eq!(before, count);
    assert_eq!(
        manage(root, "personalization_status", &json!({})).unwrap()["counts"]["events"],
        before,
        "评估阶段不得学习测试答案"
    );
    eprintln!("FIXED-POOL REPLAY (not live Rime, not user-held-out): train_events={count}, test_cases={}; baseline Top1={baseline_top1}/{} Top3={baseline_top3}/{}; learned Top1={learned_top1}/{} Top3={learned_top3}/{}", test_cases.len(), test_cases.len(), test_cases.len(), test_cases.len(), test_cases.len());
    assert!(learned_top1 > baseline_top1);
    assert!(learned_top3 >= baseline_top3);
}

#[test]
fn real_flypy_gr_pool_respects_compound_evidence_eligibility_and_personal_feedback() {
    // 静态bundled Rime、隔离user dir，真实tuuu→图书提交后输入gr捕获的32池。
    // 捕获命令与完整报告：/tmp/inputia-flypy-model-probe.ARUKYh/。
    let pool: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/flypy-gr-native-pool.json")).unwrap();
    let candidates: Vec<Candidate> = serde_json::from_value(pool["candidates"].clone()).unwrap();
    assert_eq!(
        candidates
            .iter()
            .take(7)
            .map(|c| c.text.as_str())
            .collect::<Vec<_>>(),
        vec!["管", "关", "官", "观", "馆", "灌", "冠"]
    );
    let target = candidates.iter().find(|c| c.text == "馆").unwrap();
    assert_eq!(target.base_rank, 4);
    assert_eq!(target.consumed_len, 2);
    assert_eq!(target.match_type, "exact");
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let lexicon = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../src-tauri/resources/personalization/base-lexicon.tsv");
    install_base_lexicon(root, &lexicon).unwrap();
    let request = |context: &str| Query {
        schema_id: String::new(),
        input_code: "gr".into(),
        context: context.into(),
        context_id: "isolated-field".into(),
        source_app: "com.example.editor".into(),
        learning_epoch: policy(root).unwrap().epoch,
        candidates: candidates.clone(),
        limit: 7,
    };
    let baseline = query(root, request("")).unwrap();
    assert_eq!(
        baseline["ordered_ids"],
        json!(candidates.iter().map(|c| &c.id).collect::<Vec<_>>())
    );
    let ranked = query(root, request("图书")).unwrap();
    assert_eq!(ranked["ordered_ids"][0], target.id);
    let mut zero = request("图书");
    zero.input_code.clear();
    zero.candidates.clear();
    assert_eq!(query(root, zero).unwrap()["predictions"][0]["text"], "馆");
    for (consumed, kind) in [(1, "partial"), (2, "correction")] {
        let mut q = request("图书");
        q.candidates[4].consumed_len = consumed;
        q.candidates[4].match_type = kind.into();
        assert_eq!(query(root, q).unwrap()["ordered_ids"][4], target.id);
    }
    let preference = Feedback {
        event_id: "prefer-guan".into(),
        context_id: "isolated-field".into(),
        schema_id: String::new(),
        input_code: "gr".into(),
        text: "关".into(),
        previous: "图书".into(),
        explicit_selection: true,
        original_rank: 1,
        source_app: "com.example.editor".into(),
        learning_epoch: policy(root).unwrap().epoch,
        operation: "accept".into(),
    };
    feedback(root, preference.clone()).unwrap();
    assert_eq!(
        query(root, request("图书")).unwrap()["ordered_ids"][0],
        candidates[1].id,
        "明确个人选择应优于公共组合词"
    );
    let mut undo = preference;
    undo.operation = "undo".into();
    feedback(root, undo).unwrap();
    let rejection = Feedback {
        event_id: "reject-guan".into(),
        context_id: "isolated-field".into(),
        schema_id: String::new(),
        input_code: "gr".into(),
        text: "馆".into(),
        previous: "图书".into(),
        explicit_selection: true,
        original_rank: 4,
        source_app: "com.example.editor".into(),
        learning_epoch: policy(root).unwrap().epoch,
        operation: "reject".into(),
    };
    feedback(root, rejection).unwrap();
    assert_ne!(
        query(root, request("图书")).unwrap()["ordered_ids"][0],
        target.id,
        "公共词库不得覆盖明确负反馈"
    );
}

#[test]
fn rare_public_compound_does_not_override_the_original_first_choice() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let path = root.join("rare.tsv");
    std::fs::write(&path, "海洋馆\t1\n").unwrap();
    install_base_lexicon(root, &path).unwrap();
    let mut request = query_for(root, "海洋", "gr");
    request.candidates = vec![
        Candidate {
            id: "first".into(),
            text: "管".into(),
            base_rank: 0,
            consumed_len: 2,
            match_type: "exact".into(),
        },
        Candidate {
            id: "rare".into(),
            text: "馆".into(),
            base_rank: 1,
            consumed_len: 2,
            match_type: "exact".into(),
        },
    ];
    assert_eq!(query(root, request).unwrap()["ordered_ids"][0], "first");
}
