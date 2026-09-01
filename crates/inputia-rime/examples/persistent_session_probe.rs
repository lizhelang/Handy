use std::path::PathBuf;
use std::time::Instant;

use inputia_rime::{RimeEngine, RimeEngineConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let schema = args.next().unwrap_or_else(|| "double_pinyin".to_string());
    let input = args.next().unwrap_or_else(|| "mlle".to_string());
    let iterations = args
        .next()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(20);
    let user_data_dir = std::env::var("INPUTIA_RIME_USER_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(format!(
                "/tmp/inputia-rime-persistent-probe-{}",
                std::process::id()
            ))
        });

    let mut config = RimeEngineConfig::squirrel_luna_pinyin_simp(user_data_dir)
        .with_schema(schema.clone())
        .with_spelling_correction(false);
    if let Ok(path) = std::env::var("INPUTIA_RIME_DYLIB") {
        config = config.with_dylib_path(path);
    }
    if let Ok(path) = std::env::var("INPUTIA_RIME_SHARED_DATA_DIR") {
        config = config.with_shared_data_dir(path);
    }

    let engine = RimeEngine::open(config)?;
    let prefixes = prefixes(&input);

    let cold_started = Instant::now();
    let mut cold_first = String::new();
    for _ in 0..iterations {
        for prefix in &prefixes {
            let snapshot = engine.evaluate(prefix)?;
            if cold_first.is_empty() {
                cold_first = first_candidate_text(&snapshot);
            }
        }
    }
    let cold_elapsed = cold_started.elapsed();

    let incremental_started = Instant::now();
    let mut incremental_first = String::new();
    for _ in 0..iterations {
        for prefix in &prefixes {
            let snapshot = engine.evaluate_incremental(prefix, 0)?;
            if incremental_first.is_empty() {
                incremental_first = first_candidate_text(&snapshot);
            }
        }
    }
    let incremental_elapsed = incremental_started.elapsed();

    println!("schema={schema}");
    println!("input={input}");
    println!("prefixes={}", prefixes.join(","));
    println!("iterations={iterations}");
    println!("cold_first={cold_first}");
    println!("incremental_first={incremental_first}");
    println!("cold_ms={:.3}", cold_elapsed.as_secs_f64() * 1000.0);
    println!(
        "incremental_ms={:.3}",
        incremental_elapsed.as_secs_f64() * 1000.0
    );
    if incremental_elapsed.as_nanos() > 0 {
        println!(
            "speedup={:.2}x",
            cold_elapsed.as_secs_f64() / incremental_elapsed.as_secs_f64()
        );
    }

    Ok(())
}

fn prefixes(input: &str) -> Vec<String> {
    let mut prefixes = Vec::new();
    for index in input
        .char_indices()
        .map(|(index, _)| index)
        .skip(1)
        .chain(std::iter::once(input.len()))
    {
        prefixes.push(input[..index].to_string());
    }
    prefixes
}

fn first_candidate_text(snapshot: &inputia_rime::RimeSnapshot) -> String {
    snapshot
        .candidates
        .first()
        .map(|candidate| candidate.text.clone())
        .unwrap_or_else(|| "<none>".to_string())
}
