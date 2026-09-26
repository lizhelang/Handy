use inputia_handy_runtime::knowledge_connection::export_connection;

#[test]
fn connection_is_portable_between_ai_skill_libraries() {
    let tmp = tempfile::tempdir().unwrap();
    let executable = std::env::current_exe().unwrap();
    let result = export_connection(tmp.path(), &executable).unwrap();
    let skill = std::path::Path::new(result["skill_path"].as_str().unwrap());
    assert!(skill.exists());
    assert!(skill.parent().unwrap().join("scripts/query.ps1").exists());
    let text = std::fs::read_to_string(skill).unwrap();
    assert!(text.starts_with("---\nname: inputia-knowledge\n"));
    let prompt = result["prompt"].as_str().unwrap();
    assert!(prompt.contains(skill.to_str().unwrap()));
    assert!(!prompt.contains("~/.codex"));
    // 再次导出更新同一接入包，保留固定skill位置。
    let second = export_connection(tmp.path(), &executable).unwrap();
    assert_eq!(result["skill_path"], second["skill_path"]);
}

#[cfg(unix)]
#[test]
fn shell_adapter_preserves_unicode_spaces_quotes_and_literal_query_arguments() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("资料 ' root");
    std::fs::create_dir_all(&root).unwrap();
    let executable = tmp.path().join("应用 ' executable");
    std::fs::write(&executable, "#!/bin/sh\nprintf '%s\\n' \"$@\"\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let exported = export_connection(&root, &executable).unwrap();
    let query = "测试 ' $(do-not-execute) ; `also-not-execute`";
    let result = std::process::Command::new("sh")
        .arg(exported["script_path"].as_str().unwrap())
        .args(["search", "--query", query, "--json"])
        .output()
        .unwrap();
    assert!(result.status.success());
    let text = String::from_utf8(result.stdout).unwrap();
    let lines: Vec<_> = text.lines().collect();
    assert_eq!(lines[0], "--knowledge");
    assert_eq!(lines[1], "--root");
    assert_eq!(lines[2], root.canonicalize().unwrap().to_str().unwrap());
    assert_eq!(lines[5], query);
}
