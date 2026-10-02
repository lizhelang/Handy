//! Inputia 独立更新器核心入口。
//!
//! 当前入口只提供只读诊断；任何写入、替换或恢复都必须由未来签名的
//! NativeAdapter/Installer 调用 `inputia-updater` 库完成。这里不执行 shell、
//! 不读取更新清单外的路径，也不把日志中的命令当作可执行内容。

use inputia_updater::{Phase, Updater};
use serde::Serialize;
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
        "用法：inputia-updater --status [--home <用户目录>]\n       inputia-updater --inspect <事务 UUID> [--home <用户目录>]"
    );
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
        let marker: inputia_updater::MaintenanceMarker = serde_json::from_slice(&raw)
            .map_err(|error| format!("维护标记格式无效：{error}"))?;
        status.transaction_id = Some(marker.transaction_id);
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

fn run(args: &[String]) -> Result<(), String> {
    let command = args
        .iter()
        .find(|arg| arg.as_str() == "--status" || arg.as_str() == "--inspect")
        .ok_or_else(|| "缺少 --status 或 --inspect".to_string())?;
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
        _ => Err("未知命令".into()),
    }
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
