use inputia_handy_runtime::{
    knowledge_history::{augment, handle},
    source::SOURCE_SCHEMA_VERSION,
};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::path::Path;
fn fixture(root: &Path) {
    let db = Connection::open(root.join("integration.db")).unwrap();
    db.execute_batch("CREATE TABLE integration_items(item_id TEXT,store_id TEXT,record_id TEXT,revision INTEGER,source_kind TEXT,content_type TEXT,created_at_ms INTEGER,snapshot TEXT)").unwrap();
    for (id, kind, app, text) in [
        ("v1", "voice", "com.example.editor", "项目知识 alpha"),
        ("c1", "clipboard", "com.example.editor", "剪贴板 alpha"),
        ("s1", "clipboard", "com.1password.1password", "密码 alpha"),
    ] {
        db.execute(
            "INSERT INTO integration_items VALUES(?1,?2,?1,1,?3,'text',1,?4)",
            params![
                id,
                format!("store-{kind}"),
                kind,
                json!({"text":text,"title":"测试","source_app":app}).to_string()
            ],
        )
        .unwrap();
    }
    for (file, table, logical, kind, ids) in [
        (
            "history.db",
            "transcription_history",
            "history",
            "voice",
            vec!["v1"],
        ),
        (
            "clipboard.db",
            "clipboard_history",
            "clipboard",
            "clipboard",
            vec!["c1", "s1"],
        ),
    ] {
        let source = Connection::open(root.join(file)).unwrap();
        source.execute_batch(&format!("CREATE TABLE {table}(id TEXT PRIMARY KEY); CREATE TABLE unified_source_meta(singleton INTEGER,schema_version INTEGER,store_id TEXT,logical_name TEXT); CREATE TABLE unified_source_versions(record_id TEXT,revision INTEGER);")).unwrap();
        source
            .execute(
                "INSERT INTO unified_source_meta VALUES(1,?1,?2,?3)",
                params![SOURCE_SCHEMA_VERSION, format!("store-{kind}"), logical],
            )
            .unwrap();
        for id in ids {
            source
                .execute(&format!("INSERT INTO {table} VALUES(?1)"), [id])
                .unwrap();
            source
                .execute("INSERT INTO unified_source_versions VALUES(?1,1)", [id])
                .unwrap();
        }
    }
}
fn search(root: &Path, external: bool) -> Value {
    augment(
        root,
        "search",
        &json!({"query":"alpha","limit":10}),
        external,
        json!({"items":[],"warnings":[]}),
    )
    .unwrap()
}
fn share(root: &Path, kind: &str, value: bool) {
    handle(
        root,
        "update_source",
        &json!({"id":format!("history:{kind}"),"external_access":value}),
        false,
    )
    .unwrap();
}
#[test]
fn external_defaults_closed_and_revocation_applies_to_read() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fixture(root);
    assert!(search(root, true)["items"].as_array().unwrap().is_empty());
    let sources = augment(root, "sources", &json!({}), true, json!({"sources":[]})).unwrap();
    assert_eq!(sources["sources"], json!([]));
    share(root, "voice", true);
    let found = search(root, true);
    assert_eq!(found["items"].as_array().unwrap().len(), 1);
    let read = json!({"id":"history:v1","revision":"1"});
    assert!(handle(root, "read", &read, true).is_ok());
    share(root, "voice", false);
    assert!(handle(root, "read", &read, true).is_err());
    assert!(handle(
        root,
        "update_source",
        &json!({"id":"history:voice","external_access":true}),
        true
    )
    .is_err());
}
#[test]
fn stale_source_delete_and_revision_never_return_old_content() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fixture(root);
    share(root, "voice", true);
    let db = Connection::open(root.join("history.db")).unwrap();
    db.execute("UPDATE unified_source_versions SET revision=2", [])
        .unwrap();
    assert_eq!(search(root, true)["items"], json!([]));
    assert!(handle(
        root,
        "read",
        &json!({"id":"history:v1","revision":"1"}),
        true
    )
    .is_err());
    db.execute("UPDATE unified_source_versions SET revision=1", [])
        .unwrap();
    db.execute("DELETE FROM transcription_history", []).unwrap();
    assert_eq!(search(root, true)["items"], json!([]));
    assert!(handle(
        root,
        "read",
        &json!({"id":"history:v1","revision":"1"}),
        true
    )
    .is_err());
}
#[test]
fn missing_databases_are_not_created_by_reads() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    assert_eq!(search(root, false)["items"], json!([]));
    assert!(handle(
        root,
        "read",
        &json!({"id":"history:any","revision":"1"}),
        true
    )
    .is_err());
    augment(root, "status", &json!({}), false, json!({"sources":[]})).unwrap();
    assert_eq!(std::fs::read_dir(root).unwrap().count(), 0);
}
#[test]
fn sensitive_apps_filtered_and_remove_disables_without_deleting() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fixture(root);
    share(root, "clipboard", true);
    assert_eq!(search(root, true)["items"].as_array().unwrap().len(), 1);
    assert!(handle(
        root,
        "read",
        &json!({"id":"history:s1","revision":"1"}),
        true
    )
    .is_err());
    handle(
        root,
        "remove_source",
        &json!({"id":"history:clipboard"}),
        false,
    )
    .unwrap();
    assert!(search(root, true)["items"].as_array().unwrap().is_empty());
    assert_eq!(
        Connection::open(root.join("clipboard.db"))
            .unwrap()
            .query_row("SELECT count(*) FROM clipboard_history", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
}
#[test]
fn bounded_read_exposes_continuation_and_revision_must_match() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fixture(root);
    let first = handle(
        root,
        "read",
        &json!({"id":"history:v1","revision":"1","max_chars":2}),
        false,
    )
    .unwrap()
    .unwrap();
    assert_eq!(first["text"], "项目");
    assert_eq!(first["truncated"], true);
    assert_eq!(first["next_offset"], 2);
    assert!(handle(
        root,
        "read",
        &json!({"id":"history:v1","revision":"0"}),
        false
    )
    .is_err());
    let result = augment(
        root,
        "search",
        &json!({"query":"alpha","source_id":"history:voice","limit":1}),
        false,
        json!({"items":[]}),
    )
    .unwrap();
    assert_eq!(result["items"].as_array().unwrap().len(), 1);
    assert_eq!(result["items"][0]["source_id"], "history:voice");
}
#[test]
fn missing_source_schema_fails_closed_with_warning() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fixture(root);
    share(root, "voice", true);
    Connection::open(root.join("history.db"))
        .unwrap()
        .execute("DROP TABLE unified_source_meta", [])
        .unwrap();
    let result = search(root, true);
    assert_eq!(result["items"], json!([]));
    assert!(!result["warnings"].as_array().unwrap().is_empty());
}
#[test]
fn mixed_browse_round_robins_sources_and_preserves_more() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fixture(root);
    let files = json!({"items":[{"id":"f1"},{"id":"f2"},{"id":"f3"}],"has_more":false});
    let result = augment(root, "search", &json!({"query":"","limit":3}), false, files).unwrap();
    let items = result["items"].as_array().unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["id"], "f1");
    assert_eq!(items[1]["source_id"], "history:voice");
    assert_eq!(items[2]["source_id"], "history:clipboard");
    assert_eq!(result["has_more"], true);
    let result = augment(
        root,
        "search",
        &json!({"query":"","limit":1,"source_id":"history:clipboard"}),
        false,
        json!({"items":[],"has_more":false}),
    )
    .unwrap();
    assert_eq!(result["items"][0]["source_id"], "history:clipboard");
}
#[test]
fn keyword_and_matches_chinese_and_ascii_across_gaps() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fixture(root);
    Connection::open(root.join("integration.db")).unwrap().execute("UPDATE integration_items SET snapshot=?1 WHERE item_id='v1'", [json!({"text":"项目会议安排，相关知识 alpha","title":"测试","source_app":"com.example.editor"}).to_string()]).unwrap();
    let result = augment(
        root,
        "search",
        &json!({"query":"项目 知识 ALPHA","limit":10}),
        false,
        json!({"items":[]}),
    )
    .unwrap();
    assert_eq!(result["items"].as_array().unwrap().len(), 1);
    assert_eq!(result["items"][0]["id"], "history:v1");
    let result = augment(
        root,
        "search",
        &json!({"query":"项目 不存在","limit":10}),
        false,
        json!({"items":[]}),
    )
    .unwrap();
    assert!(result["items"].as_array().unwrap().is_empty());
    let result = augment(
        root,
        "search",
        &json!({"query":"词 ".repeat(33)}),
        false,
        json!({"items":[]}),
    );
    assert!(result.is_err());
}
#[test]
fn keyboard_source_is_available_with_capture_disabled_by_default() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fixture(root);
    let result = augment(root, "status", &json!({}), false, json!({"sources":[]})).unwrap();
    let source = result["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "history:saved_snippet")
        .unwrap();
    assert_eq!(source["status"], "ready");
    assert_eq!(source["capture_enabled"], false);
    assert_eq!(source["capture_epoch"], 1);
    assert_eq!(source["external_access"], false);
    assert!(!root.join("knowledge/typed.sqlite").exists());
}
