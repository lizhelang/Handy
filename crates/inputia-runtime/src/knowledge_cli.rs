//! 供外部 AI 调用的只读知识接口。
use super::knowledge::KnowledgeStore;
use serde_json::{json, Value};
use std::path::PathBuf;

/// 执行 CLI 参数（不含可执行文件名），以 JSON 返回结果与退出状态。
pub fn run(args: Vec<String>) -> i32 {
    match execute(args) {
        Ok(data) => {
            println!("{}", json!({"ok":true,"data":data}));
            0
        }
        Err(error) => {
            println!("{}", json!({"ok":false,"error":error}));
            1
        }
    }
}
fn execute(args: Vec<String>) -> Result<Value, String> {
    let mut root = None;
    let mut action = None;
    let mut payload = json!({});
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "--json" => {}
            "status" | "sources" | "search" | "read" => {
                if action.replace(arg.clone()).is_some() {
                    return Err("只能指定一个操作".into());
                }
            }
            "--root" | "--query" | "--limit" | "--id" | "--revision" | "--source-id"
            | "--offset" | "--max-chars" => {
                i += 1;
                let value = args.get(i).ok_or_else(|| format!("{arg} 缺少参数"))?;
                match arg.as_str() {
                    "--root" => {
                        if root.replace(PathBuf::from(value)).is_some() {
                            return Err("重复 root 参数".into());
                        }
                    }
                    _ => {
                        let key = arg.trim_start_matches("--").replace('-', "_");
                        if payload.get(&key).is_some() {
                            return Err(format!("重复参数 {arg}"));
                        }
                        payload[&key] = if matches!(key.as_str(), "limit" | "offset" | "max_chars")
                        {
                            json!(value.parse::<u64>().map_err(|_| "limit 必须为正整数")?)
                        } else {
                            json!(value)
                        };
                    }
                }
            }
            _ => return Err(format!("未知参数：{arg}")),
        }
        i += 1;
    }
    let action = action.ok_or("需要 status/sources/search/read 操作")?;
    let allowed: &[&str] = match action.as_str() {
        "search" => &["query", "limit", "source_id"],
        "read" => &["id", "revision", "offset", "max_chars"],
        _ => &[],
    };
    for key in payload.as_object().ok_or("参数无效")?.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(format!("{action} 不支持 {key}"));
        }
    }
    let root = root.ok_or("需要 --root 指定 Inputia 数据目录")?;
    if !root.join("knowledge/index.sqlite").is_file() {
        return Err("知识库尚未初始化，请先打开 Inputia 知识库设置".into());
    }
    KnowledgeStore::open_readonly(&root)?.dispatch(&action, payload, true)
}
