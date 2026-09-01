use std::path::PathBuf;

use inputia_core::{CoreSettings, InputMode, InputiaCore, Key};
use inputia_rime::{RimeEngine, RimeEngineConfig};

static RIME_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn rime_engine_drives_inputia_core_full_pinyin_flow_when_available() {
    let _guard = RIME_TEST_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let config = RimeEngineConfig::squirrel_luna_pinyin_simp(temp.path().join("rime-user"));
    if !config.dylib_path.exists() || !config.shared_data_dir.exists() {
        eprintln!("skip: Squirrel librime runtime is not installed on this machine");
        return;
    }

    let engine = RimeEngine::open(config).unwrap();
    let mut core = InputiaCore::new(
        CoreSettings {
            candidate_page_size: 2,
            ..CoreSettings::default()
        },
        engine,
    );

    let switched = core.handle_key(Key::Shift);
    assert_eq!(switched.snapshot.mode, InputMode::Chinese);

    let mut outcome = switched;
    for ch in "zhongguo".chars() {
        outcome = core.handle_key(Key::Char(ch));
    }

    assert_eq!(outcome.snapshot.composing, "zhongguo");
    assert_eq!(outcome.snapshot.visible_candidates.len(), 2);
    assert_eq!(outcome.snapshot.visible_candidates[0].text, "中国");

    let page_down = core.handle_key(Key::PageDown);
    assert_eq!(page_down.snapshot.page, 1);
    assert_eq!(page_down.snapshot.visible_candidates.len(), 2);
    assert_ne!(page_down.snapshot.visible_candidates[0].text, "中国");

    let page_up = core.handle_key(Key::PageUp);
    assert_eq!(page_up.snapshot.page, 0);
    assert_eq!(page_up.snapshot.visible_candidates[0].text, "中国");

    let committed = core.handle_key(Key::Space);
    assert_eq!(committed.commit.as_deref(), Some("中国"));
    assert!(committed.snapshot.composing.is_empty());
    assert!(committed.snapshot.visible_candidates.is_empty());
}

#[test]
fn rime_engine_exposes_rime_second_page_to_core_paging_when_available() {
    let _guard = RIME_TEST_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let config = RimeEngineConfig::squirrel_luna_pinyin_simp(temp.path().join("rime-user"));
    if !config.dylib_path.exists() || !config.shared_data_dir.exists() {
        eprintln!("skip: Squirrel librime runtime is not installed on this machine");
        return;
    }

    let engine = RimeEngine::open(config).unwrap();
    let mut core = InputiaCore::new(
        CoreSettings {
            candidate_page_size: 5,
            ..CoreSettings::default()
        },
        engine,
    );

    core.handle_key(Key::Shift);
    let mut outcome = core.snapshot();
    for ch in "ni".chars() {
        outcome = core.handle_key(Key::Char(ch)).snapshot;
    }
    assert_eq!(outcome.visible_candidates[0].text, "你");
    assert_eq!(outcome.visible_candidates.len(), 5);

    let page_down = core.handle_key(Key::PageDown);
    assert_eq!(page_down.snapshot.page, 1);
    assert_eq!(page_down.snapshot.visible_candidates.len(), 5);
    assert_ne!(page_down.snapshot.visible_candidates[0].text, "你");
}

#[test]
fn rime_engine_exposes_deeper_single_character_candidates_when_available() {
    let _guard = RIME_TEST_LOCK.lock().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let config = RimeEngineConfig::squirrel_luna_pinyin_simp(temp.path().join("rime-user"));
    if !config.dylib_path.exists() || !config.shared_data_dir.exists() {
        eprintln!("skip: Squirrel librime runtime is not installed on this machine");
        return;
    }

    let engine = RimeEngine::open(config).unwrap();
    let mut core = InputiaCore::new(
        CoreSettings {
            candidate_page_size: 8,
            ..CoreSettings::default()
        },
        engine,
    );

    core.handle_key(Key::Shift);
    let mut outcome = core.snapshot();
    for ch in "ba".chars() {
        outcome = core.handle_key(Key::Char(ch)).snapshot;
    }
    assert_eq!(outcome.visible_candidates[0].text, "吧");
    assert!(!outcome
        .visible_candidates
        .iter()
        .any(|candidate| candidate.text == "叭"));

    let page_down = core.handle_key(Key::PageDown);
    assert!(
        page_down
            .snapshot
            .visible_candidates
            .iter()
            .any(|candidate| candidate.text == "叭"),
        "ba should expose 叭 after paging instead of truncating after the first two Rime pages"
    );
}

#[test]
fn rime_engine_drives_inputia_core_double_pinyin_flow_when_prepared() {
    let _guard = RIME_TEST_LOCK.lock().unwrap();
    let shared_data_dir = PathBuf::from("/tmp/inputia-rime-shared-double-pinyin");
    let user_data_dir = PathBuf::from("/tmp/inputia-rime-user-double-pinyin");
    if !shared_data_dir
        .join("double_pinyin_flypy.schema.yaml")
        .exists()
    {
        eprintln!(
            "skip: run spikes/inputia-rime/prepare-double-pinyin-data.sh double_pinyin_flypy first"
        );
        return;
    }

    let config = RimeEngineConfig::squirrel_luna_pinyin_simp(user_data_dir)
        .with_shared_data_dir(shared_data_dir)
        .with_schema("double_pinyin_flypy");
    if !config.dylib_path.exists() {
        eprintln!("skip: Squirrel librime runtime is not installed on this machine");
        return;
    }

    let engine = RimeEngine::open(config).unwrap();
    let mut core = InputiaCore::new(CoreSettings::default(), engine);

    core.handle_key(Key::Shift);
    let mut outcome = core.snapshot();
    for ch in "vsgo".chars() {
        outcome = core.handle_key(Key::Char(ch)).snapshot;
    }

    assert_eq!(outcome.composing, "vsgo");
    assert_eq!(outcome.visible_candidates[0].text, "中国");

    let committed = core.handle_key(Key::Space);
    assert_eq!(committed.commit.as_deref(), Some("中国"));
}
