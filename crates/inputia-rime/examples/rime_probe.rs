use std::path::PathBuf;

use inputia_rime::{RimeEngine, RimeEngineConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let schema = args
        .next()
        .unwrap_or_else(|| "luna_pinyin_simp".to_string());
    let keys = args.next().unwrap_or_else(|| "ni".to_string());
    let user_data_dir = std::env::var("INPUTIA_RIME_USER_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(format!("/tmp/inputia-rime-adapter-{}", std::process::id()))
        });

    let mut config = RimeEngineConfig::squirrel_luna_pinyin_simp(user_data_dir).with_schema(schema);
    if let Ok(path) = std::env::var("INPUTIA_RIME_DYLIB") {
        config = config.with_dylib_path(path);
    }
    if let Ok(path) = std::env::var("INPUTIA_RIME_SHARED_DATA_DIR") {
        config = config.with_shared_data_dir(path);
    }

    let engine = RimeEngine::open(config)?;
    let snapshot = engine.evaluate(&keys)?;

    println!("schema={}", snapshot.schema_id);
    println!("keys={keys}");
    println!("preedit={}", snapshot.preedit);
    println!(
        "page={} page_size={} candidates={} last={} highlighted={}",
        snapshot.page_no,
        snapshot.page_size,
        snapshot.candidates.len(),
        snapshot.is_last_page,
        snapshot.highlighted_candidate_index
    );
    for (index, candidate) in snapshot.candidates.iter().enumerate() {
        println!(
            "candidate[{index}]={}\t{}",
            candidate.text, candidate.annotation
        );
    }
    println!(
        "commit={}",
        snapshot.commit.unwrap_or_else(|| "<none>".to_string())
    );

    Ok(())
}
