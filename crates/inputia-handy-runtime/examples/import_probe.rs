use inputia_core::AppPolicy;
use inputia_handy_runtime::{HandyDataPaths, ImportLimits, InputiaHandyRuntime};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let memory_db = args
        .next()
        .unwrap_or_else(|| "/tmp/inputia-handy-runtime-import-probe.db".to_string());
    let app_data_dir = args.next().unwrap_or_else(|| {
        format!(
            "{}/Library/Application Support/com.pais.handy",
            std::env::var("HOME").unwrap_or_else(|_| ".".to_string())
        )
    });

    let paths = HandyDataPaths::new(app_data_dir).with_memory_db(memory_db);
    let mut runtime = InputiaHandyRuntime::open(paths, AppPolicy::default())?;
    let summary = runtime.import_existing_sources(ImportLimits::default(), "com.pais.handy")?;
    let completion_count_for_zhongguo = runtime.memory().completion_candidates("中国", 20)?.len();

    println!("history_imported={}", summary.history_imported);
    println!("clipboard_imported={}", summary.clipboard_imported);
    println!("inputia_terms={}", summary.term_count);
    println!("completion_count_for_prefix_中国={completion_count_for_zhongguo}");
    Ok(())
}
