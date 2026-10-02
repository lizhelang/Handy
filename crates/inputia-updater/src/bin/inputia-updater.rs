//! Inputia 独立更新器核心入口。
//!
//! 当前入口只提供只读诊断；任何写入、替换或恢复都必须由未来签名的
//! NativeAdapter/Installer 调用 `inputia-updater` 库完成。这里不执行 shell、
//! 不读取更新清单外的路径，也不把日志中的命令当作可执行内容。

use inputia_updater::{InstallAuthorization, InstallRequest, Phase, PreparedPlan, Updater};
use serde::de::DeserializeOwned;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{env, path::PathBuf, process::ExitCode};

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
struct Status {
    schema_version: u32,
    root: String,
    maintenance_present: bool,
    transaction_id: Option<String>,
    phase: Option<Phase>,
    writes_released: Option<bool>,
    rollback_requested: Option<bool>,
}

fn usage() {
    eprintln!(
        "用法：inputia-updater --status [--home <用户目录>]\n       inputia-updater --inspect <事务 UUID> [--home <用户目录>]\n       inputia-updater --prepare <authorized-request.json> [--home <用户目录>]\n       inputia-updater --authorization-for-request <request.json> --pair-sha <sha256> (--release-envelope <file> | --release-envelope-sha <sha256>)"
    );
}

fn json_file<T: DeserializeOwned>(value: &str) -> Result<T, String> {
    if !value.starts_with('/') || value.contains("//") || value.contains("..") {
        return Err("JSON 请求路径必须是绝对且不含路径穿越的普通文件".into());
    }
    let path = PathBuf::from(value);
    if path.is_symlink() || !path.is_file() {
        return Err("JSON 请求不是普通文件".into());
    }
    let metadata =
        std::fs::metadata(&path).map_err(|error| format!("读取请求元数据失败：{error}"))?;
    if metadata.len() > 1_048_576 {
        return Err("JSON 请求超过 1 MiB 限制".into());
    }
    let raw = std::fs::read(&path).map_err(|error| format!("读取 JSON 请求失败：{error}"))?;
    serde_json::from_slice(&raw).map_err(|error| format!("JSON 请求格式无效：{error}"))
}

fn home(args: &[String]) -> Result<PathBuf, String> {
    if let Some(index) = args.iter().position(|arg| arg == "--home") {
        let value = args
            .get(index + 1)
            .ok_or_else(|| "--home 缺少路径".to_string())?;
        if !value.starts_with('/') || value.contains("//") || value.contains("..") {
            return Err("--home 必须是绝对且不含路径穿越的目录".into());
        }
        return Ok(PathBuf::from(value));
    }
    env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "无法取得当前用户 HOME".into())
}

fn updater(args: &[String]) -> Result<Updater, String> {
    let home = home(args)?;
    let uid = unsafe { libc::geteuid() };
    Updater::new(home, uid).map_err(|error| format!("初始化更新器失败：{error}"))
}

fn print_status(updater: &Updater) -> Result<(), String> {
    let root = updater.root();
    let maintenance_present = updater.maintenance_path().is_file();
    let mut status = Status {
        schema_version: 1,
        root: root.to_string_lossy().into_owned(),
        maintenance_present,
        transaction_id: None,
        phase: None,
        writes_released: None,
        rollback_requested: None,
    };
    if maintenance_present {
        let raw = std::fs::read(updater.maintenance_path())
            .map_err(|error| format!("读取维护标记失败：{error}"))?;
        let marker: inputia_updater::MaintenanceMarker =
            serde_json::from_slice(&raw).map_err(|error| format!("维护标记格式无效：{error}"))?;
        let transaction_id = marker.transaction_id;
        let journal = updater
            .inspect(&transaction_id)
            .map_err(|error| format!("维护标记对应事务日志不可用：{error}"))?;
        if journal.subject.transaction_id != transaction_id {
            return Err("维护标记与事务日志身份不一致".into());
        }
        status.transaction_id = Some(transaction_id);
        status.phase = Some(journal.phase);
        status.writes_released = Some(journal.writes_released);
        status.rollback_requested = Some(journal.rollback_requested);
    }
    println!(
        "{}",
        serde_json::to_string(&status).map_err(|error| format!("编码状态失败：{error}"))?
    );
    Ok(())
}

fn inspect(updater: &Updater, id: &str) -> Result<(), String> {
    let journal = updater
        .inspect(id)
        .map_err(|error| format!("检查事务失败：{error}"))?;
    println!(
        "{}",
        serde_json::to_string(&journal).map_err(|error| format!("编码事务失败：{error}"))?
    );
    Ok(())
}

fn prepare(updater: &Updater, request_path: &str) -> Result<(), String> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct AuthorizedRequest {
        request: inputia_updater::InstallRequest,
        authorization: InstallAuthorization,
    }
    let envelope: AuthorizedRequest = json_file(request_path)?;
    let (plan, _evidence): (PreparedPlan, _) = updater
        .prepare_authorized(envelope.request, envelope.authorization)
        .map_err(|error| format!("安装预检失败：{error}"))?;
    println!(
        "{}",
        serde_json::to_string(&plan).map_err(|error| format!("编码安装预检失败：{error}"))?
    );
    Ok(())
}

fn authorization_for_request(
    request_path: &str,
    pair_sha: &str,
    release_envelope: &str,
    release_envelope_sha: Option<&str>,
) -> Result<(), String> {
    let request: InstallRequest = json_file(request_path)?;
    let envelope_sha = match (release_envelope, release_envelope_sha) {
        (path, None) if !path.is_empty() => digest_file(path)?,
        ("", Some(value)) => value.to_owned(),
        _ => return Err("必须且只能提供一个发布信封来源".into()),
    };
    let authorization =
        InstallAuthorization::for_request(&request, pair_sha.to_owned(), envelope_sha)
            .map_err(|error| format!("生成安装授权候选失败：{error}"))?;
    println!(
        "{}",
        serde_json::to_string(&authorization)
            .map_err(|error| format!("编码安装授权候选失败：{error}"))?
    );
    Ok(())
}

fn run(args: &[String]) -> Result<(), String> {
    let command = args
        .iter()
        .find(|arg| {
            matches!(
                arg.as_str(),
                "--status" | "--inspect" | "--prepare" | "--authorization-for-request"
            )
        })
        .ok_or_else(|| "缺少 --status 或 --inspect".to_string())?;
    match command.as_str() {
        "--authorization-for-request" => {
            let index = args
                .iter()
                .position(|arg| arg == "--authorization-for-request")
                .ok_or_else(|| "缺少 request.json".to_string())?;
            let request = args
                .get(index + 1)
                .ok_or_else(|| "--authorization-for-request 缺少 request.json".to_string())?;
            let pair_sha = option_value(args, "--pair-sha")?;
            let envelope_path = optional_value(args, "--release-envelope")?;
            let envelope_sha = optional_value(args, "--release-envelope-sha")?;
            authorization_for_request(request, pair_sha, envelope_path.unwrap_or(""), envelope_sha)
        }
        _ => {
            let updater = updater(args)?;
            match command.as_str() {
                "--status" => print_status(&updater),
                "--inspect" => {
                    let index = args
                        .iter()
                        .position(|arg| arg == "--inspect")
                        .ok_or_else(|| "缺少事务 UUID".to_string())?;
                    let id = args
                        .get(index + 1)
                        .ok_or_else(|| "--inspect 缺少事务 UUID".to_string())?;
                    inspect(&updater, id)
                }
                "--prepare" => {
                    let index = args
                        .iter()
                        .position(|arg| arg == "--prepare")
                        .ok_or_else(|| "缺少 request.json".to_string())?;
                    let request = args
                        .get(index + 1)
                        .ok_or_else(|| "--prepare 缺少 request.json".to_string())?;
                    prepare(&updater, request)
                }
                _ => Err("未知命令".into()),
            }
        }
    }
}

fn optional_value<'a>(args: &'a [String], name: &str) -> Result<Option<&'a str>, String> {
    let Some(index) = args.iter().position(|arg| arg == name) else {
        return Ok(None);
    };
    let value = args
        .get(index + 1)
        .ok_or_else(|| format!("{name} 缺少值"))?;
    Ok(Some(value.as_str()))
}

fn digest_file(value: &str) -> Result<String, String> {
    if !value.starts_with('/') || value.contains("//") || value.contains("..") {
        return Err("发布信封路径必须是绝对且不含路径穿越的普通文件".into());
    }
    let path = PathBuf::from(value);
    if path.is_symlink() || !path.is_file() {
        return Err("发布信封不是普通文件".into());
    }
    let metadata =
        std::fs::metadata(&path).map_err(|error| format!("读取发布信封元数据失败：{error}"))?;
    if metadata.len() > 4 * 1024 * 1024 {
        return Err("发布信封超过 4 MiB 限制".into());
    }
    let bytes = std::fs::read(&path).map_err(|error| format!("读取发布信封失败：{error}"))?;
    Ok(Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

fn option_value<'a>(args: &'a [String], name: &str) -> Result<&'a str, String> {
    let index = args
        .iter()
        .position(|arg| arg == name)
        .ok_or_else(|| format!("缺少 {name}"))?;
    args.get(index + 1)
        .map(String::as_str)
        .ok_or_else(|| format!("{name} 缺少值"))
}

fn main() -> ExitCode {
    let args: Vec<_> = env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        usage();
        return ExitCode::SUCCESS;
    }
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            usage();
            ExitCode::from(2)
        }
    }
}
