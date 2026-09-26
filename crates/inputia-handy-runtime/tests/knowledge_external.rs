//! 用导出的 skill 脚本执行真实进程，不模拟 CLI 返回值。
#![cfg(unix)]
use inputia_handy_runtime::{
    knowledge::KnowledgeStore, knowledge_connection::export_connection,
    source::SOURCE_SCHEMA_VERSION,
};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::{os::unix::fs::PermissionsExt, path::Path, process::Command};

fn call(script: &Path, args: &[&str]) -> (bool, Value) {
    let output = Command::new("sh").arg(script).args(args).output().unwrap();
    let value = serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "non-JSON output: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    });
    (output.status.success(), value)
}

#[test]
fn exported_skill_reads_files_and_live_history_and_honors_revocation() {
    let tmp = tempfile::Builder::new()
        .prefix("inputia-知识 验收-")
        .tempdir()
        .unwrap();
    let root = tmp.path().join("data");
    let store = KnowledgeStore::open(&root).unwrap();
    store
        .dispatch(
            "save_note",
            json!({"title":"设计说明","text":"验收：文件和输入记录使用同一知识库。"}),
            false,
        )
        .unwrap();
    store
        .dispatch(
            "update_source",
            json!({"id":"managed","external_access":true}),
            false,
        )
        .unwrap();
    let index = Connection::open(root.join("integration.db")).unwrap();
    index.execute_batch("CREATE TABLE integration_items(item_id TEXT,store_id TEXT,record_id TEXT,revision INTEGER,source_kind TEXT,content_type TEXT,created_at_ms INTEGER,snapshot TEXT)").unwrap();
    for (kind, file, table, logical) in [
        ("voice", "history.db", "transcription_history", "history"),
        (
            "clipboard",
            "clipboard.db",
            "clipboard_history",
            "clipboard",
        ),
    ] {
        index.execute("INSERT INTO integration_items VALUES(?1,?2,'1',1,?1,'text',1,?3)", params![kind,format!("store-{kind}"),json!({"text":format!("验收：{kind}的补充记录"),"title":kind,"source_app":"com.example.editor"}).to_string()]).unwrap();
        let source = Connection::open(root.join(file)).unwrap();
        source.execute_batch(&format!("CREATE TABLE {table}(id TEXT); INSERT INTO {table} VALUES('1'); CREATE TABLE unified_source_meta(singleton INTEGER,schema_version INTEGER,store_id TEXT,logical_name TEXT); CREATE TABLE unified_source_versions(record_id TEXT,revision INTEGER); INSERT INTO unified_source_versions VALUES('1',1);")).unwrap();
        source
            .execute(
                "INSERT INTO unified_source_meta VALUES(1,?1,?2,?3)",
                params![SOURCE_SCHEMA_VERSION, format!("store-{kind}"), logical],
            )
            .unwrap();
        store
            .dispatch(
                "update_source",
                json!({"id":format!("history:{kind}"),"external_access":true}),
                false,
            )
            .unwrap();
    }
    // 默认使用同一 crate 的 CLI；集成验收传入完整原生 App 可执行文件。
    let executable = if let Some(path) = std::env::var_os("INPUTIA_KB_NATIVE_APP") {
        std::path::PathBuf::from(path)
    } else {
        let shim = tmp.path().join("app-adapter");
        let binary = env!("CARGO_BIN_EXE_inputia-kb").replace('\'', "'\"'\"'");
        std::fs::write(
            &shim,
            format!(
                "#!/bin/sh\n[ \"$1\" = --knowledge ] || exit 2\nshift\nexec '{binary}' \"$@\"\n"
            ),
        )
        .unwrap();
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o700)).unwrap();
        shim
    };
    let exported = export_connection(&root, &executable).unwrap();
    let script = Path::new(exported["script_path"].as_str().unwrap());
    let (ok, status) = call(script, &["status", "--json"]);
    assert!(ok, "{status}");
    assert_eq!(status["data"]["protocol_version"], 1);
    let (ok, found) = call(
        script,
        &["search", "--query", "验收", "--limit", "10", "--json"],
    );
    assert!(ok, "{found}");
    let items = found["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 3, "{found}");
    for kind in ["file", "voice", "clipboard"] {
        assert!(items.iter().any(|i| i["kind"] == kind));
    }
    let file = items.iter().find(|i| i["kind"] == "file").unwrap();
    let args = [
        "read",
        "--id",
        file["id"].as_str().unwrap(),
        "--revision",
        file["revision"].as_str().unwrap(),
        "--json",
    ];
    assert!(call(script, &args).0);
    store
        .dispatch(
            "update_source",
            json!({"id":"managed","external_access":false}),
            false,
        )
        .unwrap();
    assert!(
        !call(script, &args).0,
        "revoked references must not remain readable"
    );
    // 尚未从统一索引删除的源记录也不能再返回。
    Connection::open(root.join("history.db"))
        .unwrap()
        .execute("DELETE FROM transcription_history", [])
        .unwrap();
    let (ok, after) = call(script, &["search", "--query", "验收", "--json"]);
    assert!(ok);
    assert_eq!(after["data"]["items"].as_array().unwrap().len(), 1);
    assert_eq!(after["data"]["items"][0]["kind"], "clipboard");
    assert!(!call(script, &["remove_source", "--id", "managed"]).0);
}

#[test]
fn markdown_and_txt_import_and_directory_are_searchable_and_readable_via_external_cli() {
    let tmp = tempfile::Builder::new()
        .prefix("inputia-md-txt-中文 验收-")
        .tempdir()
        .unwrap();
    let root = tmp.path().join("应用 数据");
    let imports = tmp.path().join("待导入 文件");
    let directory = tmp.path().join("外部 资料目录");
    std::fs::create_dir_all(&imports).unwrap();
    std::fs::create_dir_all(&directory).unwrap();
    let store = KnowledgeStore::open(&root).unwrap();
    let mut expected = Vec::new();
    let mut import_paths = Vec::new();
    for extension in ["md", "txt"] {
        for (entry, path) in [("导入", &imports), ("目录", &directory)] {
            let name = format!("{entry} 中文资料.{extension}");
            let body = format!("# 中文资料\n跨格式知识库：{entry}的{extension}正文。\n完整验收应保留原文与引用。\n");
            let file = path.join(&name);
            std::fs::write(&file, &body).unwrap();
            if entry == "导入" {
                import_paths.push(file);
            }
            expected.push((name, body, entry));
        }
    }
    let imported = store
        .dispatch("import_files", json!({"paths":import_paths}), false)
        .unwrap();
    assert_eq!(imported["paths"].as_array().unwrap().len(), 2);
    assert_eq!(imported["partial"], false);
    assert_eq!(imported["errors"], json!([]));
    for (name, body, entry) in &expected {
        if *entry == "导入" {
            assert_eq!(
                std::fs::read_to_string(root.join("knowledge/files").join(name)).unwrap(),
                *body
            );
        }
    }
    let source = store
        .dispatch("add_directory", json!({"path":directory}), false)
        .unwrap();
    let source_id = source["id"].as_str().unwrap();
    let query = "跨格式 完整验收";
    assert_eq!(
        store
            .dispatch("search", json!({"query":query}), false)
            .unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert!(store
        .dispatch("search", json!({"query":query}), true)
        .unwrap()["items"]
        .as_array()
        .unwrap()
        .is_empty());
    for id in ["managed", source_id] {
        store
            .dispatch(
                "update_source",
                json!({"id":id,"external_access":true}),
                false,
            )
            .unwrap();
    }
    let executable = if let Some(path) = std::env::var_os("INPUTIA_KB_NATIVE_APP") {
        std::path::PathBuf::from(path)
    } else {
        let shim = tmp.path().join("app-adapter");
        let binary = env!("CARGO_BIN_EXE_inputia-kb").replace('\'', "'\"'\"'");
        std::fs::write(
            &shim,
            format!(
                "#!/bin/sh\n[ \"$1\" = --knowledge ] || exit 2\nshift\nexec '{binary}' \"$@\"\n"
            ),
        )
        .unwrap();
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o700)).unwrap();
        shim
    };
    let exported = export_connection(&root, &executable).unwrap();
    let script = Path::new(exported["script_path"].as_str().unwrap());
    let (ok, found) = call(script, &["search", "--query", query, "--json"]);
    assert!(ok, "{found}");
    let items = found["data"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 4, "{found}");
    for (name, body, entry) in &expected {
        let item = items.iter().find(|item| item["title"] == *name).unwrap();
        assert_eq!(item["text"], *body);
        assert_eq!(
            item["source_id"],
            if *entry == "导入" {
                "managed"
            } else {
                source_id
            }
        );
        assert!(item["locator"].as_str().unwrap().contains(name));
        let (ok, read) = call(
            script,
            &[
                "read",
                "--id",
                item["id"].as_str().unwrap(),
                "--revision",
                item["revision"].as_str().unwrap(),
                "--json",
            ],
        );
        assert!(ok, "{read}");
        assert_eq!(read["data"], *item);
    }
}
