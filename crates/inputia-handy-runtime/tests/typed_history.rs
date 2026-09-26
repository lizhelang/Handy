use inputia_handy_runtime::{
    knowledge::KnowledgeStore,
    typed_history::{self, TypedPolicy},
};
use serde_json::{json, Value};
fn record(root: &std::path::Path, event: &str, segment: &str, text: &str, epoch: u64) -> Value {
    typed_history::record(root, event, segment, text, "com.example.editor", epoch).unwrap()
}
#[test]
fn defaults_do_not_create_storage_and_epochs_reject_old_queues() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    assert_eq!(
        typed_history::policy(root).unwrap(),
        TypedPolicy {
            enabled: false,
            epoch: 1
        }
    );
    assert!(!root.join("knowledge").exists());
    assert!(typed_history::record(root, "e", "s", "不应收录", "com.example.editor", 1).is_err());
    assert!(!root.join("knowledge").exists());
    let on = typed_history::set_capture(root, true).unwrap();
    assert_eq!(on.epoch, 2);
    let first = record(root, "e1", "s", "中文", on.epoch);
    let duplicate = record(root, "e1", "s", "中文", on.epoch);
    assert_eq!(first["id"], duplicate["id"]);
    assert_eq!(duplicate["duplicate"], true);
    assert!(typed_history::record(
        root,
        "e1",
        "s",
        "另一个正文",
        "com.example.editor",
        on.epoch
    )
    .is_err());
    let off = typed_history::set_capture(root, false).unwrap();
    assert_eq!(off.epoch, 3);
    let again = typed_history::set_capture(root, true).unwrap();
    assert_eq!(again.epoch, 4);
    assert!(
        typed_history::record(root, "e2", "s", "迟到正文", "com.example.editor", on.epoch).is_err()
    );
    let new = record(root, "e3", "s", "新世代", again.epoch);
    assert_ne!(first["id"], new["id"]);
}
#[test]
fn segments_append_but_never_cross_source_and_old_revisions_fail() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let epoch = typed_history::set_capture(root, true).unwrap().epoch;
    let first = record(root, "1", "a", "中文第一句", epoch);
    let second = record(root, "2", "a", "，追加 alpha", epoch);
    assert_eq!(first["id"], second["id"]);
    assert_eq!(second["revision"], "2");
    assert!(typed_history::read(root, first["id"].as_str().unwrap(), "1", 0, 8000).is_err());
    let read = typed_history::read(root, second["id"].as_str().unwrap(), "2", 0, 3).unwrap();
    assert_eq!(read["text"], "中文第");
    assert_eq!(read["truncated"], true);
    assert_eq!(read["next_offset"], 3);
    let other =
        typed_history::record(root, "3", "a", "其他软件", "com.example.other", epoch).unwrap();
    assert_ne!(other["id"], first["id"]);
    let segment = record(root, "4", "b", "另一个段落", epoch);
    assert_ne!(segment["id"], first["id"]);
    assert!(
        typed_history::record(root, "5", "p", "密码正文", "com.1password.1password", epoch)
            .is_err()
    );
    assert!(typed_history::record(
        root,
        "6",
        "p",
        &"长".repeat(8193),
        "com.example.editor",
        epoch
    )
    .is_err());
    let (found, more) = typed_history::search(root, "中文 ALPHA", 10).unwrap();
    assert_eq!(found.len(), 1);
    assert!(!more);
    assert_eq!(found[0]["text"], "中文第一句，追加 alpha");
}
#[test]
fn shared_search_read_and_revocation_are_independent_of_capture() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let store = KnowledgeStore::open(root).unwrap();
    let policy = store
        .dispatch("typed_capture", json!({"enabled":true}), false)
        .unwrap();
    let epoch = policy["epoch"].as_u64().unwrap();
    let r = record(root, "e1", "s", "知识库中文正文", epoch);
    let payload = json!({"query":"中文","source_id":"history:saved_snippet"});
    assert!(
        store.dispatch("search", payload.clone(), true).unwrap()["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    store
        .dispatch(
            "update_source",
            json!({"id":"history:saved_snippet","external_access":true}),
            false,
        )
        .unwrap();
    assert_eq!(
        store.dispatch("search", payload.clone(), true).unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let read = json!({"id":r["id"],"revision":r["revision"]});
    assert!(store.dispatch("read", read.clone(), true).is_ok());
    store
        .dispatch("typed_capture", json!({"enabled":false}), false)
        .unwrap();
    assert!(store.dispatch("read", read.clone(), true).is_ok());
    store
        .dispatch(
            "update_source",
            json!({"id":"history:saved_snippet","external_access":false}),
            false,
        )
        .unwrap();
    assert!(store.dispatch("read", read.clone(), true).is_err());
    assert!(store.dispatch("read", read.clone(), false).is_ok());
    store
        .dispatch(
            "update_source",
            json!({"id":"history:saved_snippet","enabled":false}),
            false,
        )
        .unwrap();
    assert!(store.dispatch("read", read, false).is_err());
    let status = store.dispatch("status", json!({}), false).unwrap();
    let source = status["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "history:saved_snippet")
        .unwrap();
    assert_eq!(source["capture_enabled"], false);
    assert_eq!(source["enabled"], false);
    for action in ["typed_capture", "delete_typed", "clear_typed"] {
        assert!(store
            .dispatch(action, json!({"enabled":true,"id":r["id"]}), true)
            .is_err());
    }
}
#[test]
fn deletion_and_clear_block_late_events_without_resurrection() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let store = KnowledgeStore::open(root).unwrap();
    let epoch = typed_history::set_capture(root, true).unwrap().epoch;
    let a = record(root, "a1", "a", "删除正文", epoch);
    let b = record(root, "b1", "b", "保留正文", epoch);
    let deleted = store
        .dispatch("delete_typed", json!({"id":a["id"]}), false)
        .unwrap();
    let next = deleted["capture_epoch"].as_u64().unwrap();
    assert!(next > epoch);
    assert_eq!(typed_history::count(root).unwrap(), 1);
    assert!(typed_history::record(root, "a2", "a", "迟到", "com.example.editor", epoch).is_err());
    assert!(typed_history::read(root, a["id"].as_str().unwrap(), "1", 0, 8000).is_err());
    assert!(typed_history::read(root, b["id"].as_str().unwrap(), "1", 0, 8000).is_ok());
    record(root, "c1", "c", "新增正文", next);
    let clear = store.dispatch("clear_typed", json!({}), false).unwrap();
    assert_eq!(clear["deleted"], 2);
    assert_eq!(typed_history::count(root).unwrap(), 0);
    assert!(typed_history::record(root, "c2", "c", "迟到", "com.example.editor", next).is_err());
    let current = clear["capture_epoch"].as_u64().unwrap();
    assert!(
        typed_history::record(root, "a1", "a", "删除正文", "com.example.editor", current).is_err()
    );
}
#[test]
fn concurrent_replay_appends_once_and_segment_size_is_bounded() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let epoch = typed_history::set_capture(root, true).unwrap().epoch;
    let threads: Vec<_> = (0..6)
        .map(|_| {
            let root = root.to_owned();
            std::thread::spawn(move || record(&root, "same-event", "segment", "仅一次", epoch))
        })
        .collect();
    let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert_eq!(
        results.iter().filter(|v| v["duplicate"] == false).count(),
        1
    );
    let (rows, _) = typed_history::search(root, "仅一次", 10).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["text"], "仅一次");
    assert_eq!(rows[0]["revision"], "1");
    for i in 0..4 {
        record(
            root,
            &format!("bounded-{i}"),
            "bounded",
            &"长".repeat(8192),
            epoch,
        );
    }
    assert!(typed_history::record(
        root,
        "overflow",
        "bounded",
        "多",
        "com.example.editor",
        epoch
    )
    .is_err());
    assert_eq!(typed_history::count(root).unwrap(), 2);
}
