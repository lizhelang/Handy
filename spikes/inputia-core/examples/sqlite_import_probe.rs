#[cfg(feature = "sqlite-memory")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use inputia_core_spike::{AppContext, AppPolicy, SqliteMemory};

    let mut args = std::env::args().skip(1);
    let memory_db = args
        .next()
        .unwrap_or_else(|| "/tmp/inputia-memory-import-probe.db".to_string());
    let history_db = args.next().unwrap_or_else(|| {
        format!(
            "{}/Library/Application Support/com.pais.handy/history.db",
            std::env::var("HOME").unwrap_or_else(|_| ".".to_string())
        )
    });
    let clipboard_db = args.next().unwrap_or_else(|| {
        format!(
            "{}/Library/Application Support/com.pais.handy/clipboard.db",
            std::env::var("HOME").unwrap_or_else(|_| ".".to_string())
        )
    });

    let context = AppContext::new("com.pais.handy");
    let mut memory = SqliteMemory::open(memory_db, AppPolicy::default())?;
    let history_imported = memory.import_handy_history(history_db, &context, 2_000)?;
    let clipboard_imported = memory.import_handy_clipboard(clipboard_db, &context, 2_000)?;
    let completion_count_for_zhongguo = memory.completion_candidates("中国", 20)?.len();

    println!("history_imported={history_imported}");
    println!("clipboard_imported={clipboard_imported}");
    println!("inputia_terms={}", memory.term_count()?);
    println!("completion_count_for_prefix_中国={completion_count_for_zhongguo}");
    Ok(())
}

#[cfg(not(feature = "sqlite-memory"))]
fn main() {
    eprintln!("enable with: cargo run --features sqlite-memory --example sqlite_import_probe");
    std::process::exit(2);
}
