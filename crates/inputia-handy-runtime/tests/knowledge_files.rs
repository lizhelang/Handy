use inputia_handy_runtime::knowledge::KnowledgeStore;
use serde_json::{json, Value};
use std::fs;

fn call(store: &KnowledgeStore, action: &str, p: Value, external: bool) -> Value {
    store.dispatch(action, p, external).unwrap()
}
#[test]
fn file_sources_permissions_revision_deletion_and_offline() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("应用 数据");
    let dir = temp.path().join("中文 知识");
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("接口 笔记.md");
    fs::write(&file, "# 接口\n项目接口需要保存中文引用\n").unwrap();
    let store = KnowledgeStore::open(&root).unwrap();
    let added = call(&store, "add_directory", json!({"path":dir}), false);
    let id = added["id"].as_str().unwrap();
    assert!(
        call(&store, "search", json!({"query":"接口"}), true)["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    call(
        &store,
        "update_source",
        json!({"id":id,"external_access":true}),
        false,
    );
    let result = call(&store, "search", json!({"query":"中文"}), true);
    let item = result["items"][0].clone();
    assert_eq!(item["title"], "接口 笔记.md");
    assert!(item["locator"].as_str().unwrap().contains("#L"));
    assert_eq!(
        call(
            &store,
            "read",
            json!({"id":item["id"],"revision":item["revision"]}),
            true
        ),
        item
    );
    fs::write(&file, "新的接口版本").unwrap();
    assert!(store
        .dispatch(
            "read",
            json!({"id":item["id"],"revision":item["revision"]}),
            true
        )
        .is_err());
    assert!(
        call(&store, "search", json!({"query":"中文"}), true)["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    call(&store, "sync", json!({}), false);
    assert_eq!(
        call(&store, "search", json!({"query":"新的"}), true)["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    fs::rename(&dir, temp.path().join("offline")).unwrap();
    assert!(
        call(&store, "search", json!({"query":"新的"}), true)["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    fs::rename(temp.path().join("offline"), &dir).unwrap();
    fs::remove_file(file).unwrap();
    call(&store, "sync", json!({}), false);
    assert!(
        call(&store, "search", json!({"query":"接口"}), true)["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(store.dispatch("sync", json!({}), true).is_err());
}
#[test]
fn managed_notes_import_unsupported_and_disable() {
    let temp = tempfile::tempdir().unwrap();
    let store = KnowledgeStore::open(&temp.path().join("app")).unwrap();
    call(
        &store,
        "save_note",
        json!({"title":"每日 笔记","text":"今天计划知识检索"}),
        false,
    );
    assert_eq!(
        call(&store, "search", json!({"query":"知识"}), false)["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    call(
        &store,
        "update_source",
        json!({"id":"managed","enabled":false,"external_access":true}),
        false,
    );
    assert!(
        call(&store, "search", json!({"query":"知识"}), true)["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let pdf = temp.path().join("report.pdf");
    fs::write(&pdf, "not really pdf").unwrap();
    assert!(store
        .dispatch("import_files", json!({"paths":[pdf]}), false)
        .is_err());
}
#[test]
#[cfg(unix)]
fn symlinks_hidden_and_replaced_parent_are_not_read() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let dir = temp.path().join("docs");
    let sub = dir.join("nested");
    fs::create_dir_all(&sub).unwrap();
    fs::write(sub.join("read.md"), "可见知识").unwrap();
    fs::write(dir.join(".hidden.md"), "隐藏知识").unwrap();
    let secret = temp.path().join("secret.md");
    fs::write(&secret, "秘密知识").unwrap();
    symlink(secret, dir.join("link.md")).unwrap();
    let store = KnowledgeStore::open(&temp.path().join("app")).unwrap();
    let added = call(&store, "add_directory", json!({"path":dir}), false);
    call(
        &store,
        "update_source",
        json!({"id":added["id"],"external_access":true}),
        false,
    );
    assert_eq!(
        call(&store, "search", json!({"query":"知识"}), true)["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let moved = temp.path().join("moved");
    fs::rename(&sub, &moved).unwrap();
    symlink(moved, &sub).unwrap();
    assert!(
        call(&store, "search", json!({"query":"可见"}), true)["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn bounded_chunks_and_cli_readonly_protocol() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("app");
    let store = KnowledgeStore::open(&root).unwrap();
    call(
        &store,
        "save_note",
        json!({"title":"长行","text":"知识".repeat(2500)}),
        false,
    );
    call(
        &store,
        "update_source",
        json!({"id":"managed","external_access":true}),
        false,
    );
    let found = call(&store, "search", json!({"query":"知识"}), true);
    assert!(found["items"].as_array().unwrap().len() > 1);
    for item in found["items"].as_array().unwrap() {
        assert!(item["text"].as_str().unwrap().chars().count() <= 1600);
    }
    let bin = env!("CARGO_BIN_EXE_inputia-kb");
    let out = std::process::Command::new(bin)
        .args([
            "--root",
            root.to_str().unwrap(),
            "search",
            "--query",
            "知识",
            "--limit",
            "1",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let result: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(result["ok"], true);
    assert_eq!(result["data"]["items"].as_array().unwrap().len(), 1);
    let out = std::process::Command::new(bin)
        .args(["--root", root.to_str().unwrap(), "sync"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let out = std::process::Command::new(bin)
        .args(["--root", root.to_str().unwrap(), "status", "--query", "bad"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let readonly = KnowledgeStore::open_readonly(&root).unwrap();
    assert_eq!(
        call(&readonly, "status", json!({}), true)["protocol_version"],
        1
    );
}
#[test]
fn custom_managed_directory_import_and_unchanged_sync() {
    let temp = tempfile::tempdir().unwrap();
    let store = KnowledgeStore::open(&temp.path().join("app")).unwrap();
    let managed = temp.path().join("资料 自定");
    call(&store, "set_managed_path", json!({"path":managed}), false);
    let input = temp.path().join("输入.txt");
    fs::write(&input, "导入文本资料").unwrap();
    call(&store, "import_files", json!({"paths":[input]}), false);
    assert!(managed.join("输入.txt").exists());
    assert_eq!(call(&store, "sync", json!({}), false)["updated"], 0);
    assert_eq!(
        call(&store, "search", json!({"query":"导入"}), false)["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn empty_query_browses_and_reports_more() {
    let temp = tempfile::tempdir().unwrap();
    let store = KnowledgeStore::open(temp.path()).unwrap();
    for title in ["第一条", "第二条"] {
        call(
            &store,
            "save_note",
            json!({"title":title,"text":"共同知识"}),
            false,
        );
    }
    let result = call(&store, "search", json!({"query":"","limit":1}), false);
    assert_eq!(result["items"].as_array().unwrap().len(), 1);
    assert_eq!(result["has_more"], true);
    let result = call(&store, "search", json!({"query":"共同","limit":1}), false);
    assert_eq!(result["has_more"], true);
    let result = call(&store, "search", json!({"query":"不存在","limit":1}), false);
    assert_eq!(result["has_more"], false);
}
#[test]
fn batch_import_validates_every_file_before_writing() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("app");
    let store = KnowledgeStore::open(&root).unwrap();
    let text = temp.path().join("valid.txt");
    fs::write(&text, "合法知识").unwrap();
    let pdf = temp.path().join("bad.pdf");
    fs::write(&pdf, "unsupported").unwrap();
    assert!(store
        .dispatch("import_files", json!({"paths":[text,pdf]}), false)
        .is_err());
    assert!(!root.join("knowledge/files/valid.txt").exists());
}
#[test]
#[cfg(unix)]
fn managed_writes_do_not_follow_dangling_symlinks() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("app");
    let store = KnowledgeStore::open(&root).unwrap();
    let outside = temp.path().join("outside.md");
    std::os::unix::fs::symlink(&outside, root.join("knowledge/files/笔记.md")).unwrap();
    call(
        &store,
        "save_note",
        json!({"title":"笔记","text":"安全知识"}),
        false,
    );
    assert!(!outside.exists());
    assert!(root.join("knowledge/files/笔记-1.md").exists());
    let input = temp.path().join("笔记.md");
    fs::write(&input, "导入安全知识").unwrap();
    call(&store, "import_files", json!({"paths":[input]}), false);
    assert!(!outside.exists());
    assert!(root.join("knowledge/files/笔记-2.md").exists());
}
