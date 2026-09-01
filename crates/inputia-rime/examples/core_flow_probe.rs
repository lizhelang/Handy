use std::path::PathBuf;

use inputia_core::{CoreSettings, InputMode, InputiaCore, Key};
use inputia_rime::{RimeEngine, RimeEngineConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let schema = args
        .next()
        .unwrap_or_else(|| "luna_pinyin_simp".to_string());
    let input = args.next().unwrap_or_else(|| "zhongguo".to_string());
    let page_size = args
        .next()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(2);
    let user_data_dir = std::env::var("INPUTIA_RIME_USER_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(format!(
                "/tmp/inputia-rime-core-flow-{}",
                std::process::id()
            ))
        });

    let mut config = RimeEngineConfig::squirrel_luna_pinyin_simp(user_data_dir).with_schema(schema);
    if let Ok(path) = std::env::var("INPUTIA_RIME_DYLIB") {
        config = config.with_dylib_path(path);
    }
    if let Ok(path) = std::env::var("INPUTIA_RIME_SHARED_DATA_DIR") {
        config = config.with_shared_data_dir(path);
    }

    let engine = RimeEngine::open(config)?;
    let mut core = InputiaCore::new(
        CoreSettings {
            candidate_page_size: page_size,
            ..CoreSettings::default()
        },
        engine,
    );

    let switched = core.handle_key(Key::Shift);
    println!("mode={:?}", switched.snapshot.mode);
    if switched.snapshot.mode != InputMode::Chinese {
        return Err("failed to switch into Chinese mode".into());
    }

    let mut outcome = switched;
    for ch in input.chars() {
        outcome = core.handle_key(Key::Char(ch));
    }
    print_snapshot("after_input", &outcome.snapshot);

    let paged = core.handle_key(Key::PageDown);
    print_snapshot("after_page_down", &paged.snapshot);

    let reset_page = core.handle_key(Key::PageUp);
    print_snapshot("after_page_up", &reset_page.snapshot);

    let committed = core.handle_key(Key::Space);
    println!(
        "commit={}",
        committed.commit.unwrap_or_else(|| "<none>".to_string())
    );
    print_snapshot("after_commit", &committed.snapshot);

    Ok(())
}

fn print_snapshot(label: &str, snapshot: &inputia_core::InputSnapshot) {
    println!(
        "{label}: composing={} page={} visible_candidates={}",
        snapshot.composing,
        snapshot.page,
        snapshot.visible_candidates.len()
    );
    for (index, candidate) in snapshot.visible_candidates.iter().enumerate() {
        println!(
            "{label}: candidate[{index}]={}\t{}",
            candidate.text, candidate.annotation
        );
    }
}
