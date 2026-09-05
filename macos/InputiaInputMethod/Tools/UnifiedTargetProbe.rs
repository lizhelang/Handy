//! 安全的独立原生 ABI / 身份探针；绝不调用 capture 或读取外部焦点。
#[allow(dead_code)]
#[path = "../../../src-tauri/src/unified_target.rs"]
mod unified_target;

fn main() {
    match unified_target::metadata_self_check() {
        Ok(()) => println!("unified_target_metadata_self_check=pass external_focus_read=false text_read=false input_posted=false"),
        Err(reason) => {
            eprintln!("unified_target_metadata_self_check=fail reason={reason:?}");
            std::process::exit(1);
        }
    }
}
