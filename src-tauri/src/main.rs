// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use clap::Parser;
use handy_app_lib::CliArgs;

fn main() {
    // 知识库客户端在 Tauri、录音及单实例初始化之前退出。
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|arg| arg == "--knowledge") {
        std::process::exit(inputia_handy_runtime::knowledge_cli::run(
            args[1..].to_vec(),
        ));
    }
    let cli_args = CliArgs::parse();

    #[cfg(target_os = "linux")]
    {
        // DMABUF renderer causes crashes on various GPU/display server configurations
        // See: https://github.com/tauri-apps/tauri/issues/9394
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    handy_app_lib::run(cli_args)
}
