use std::path::PathBuf;

use inputia_core::{CoreSettings, InputMode, InputiaCore, Key};
use inputia_rime::{RimeEngine, RimeEngineConfig};

static RIME_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn rime_engine_drives_inputia_core_full_pinyin_flow_when_available() {
    let _guard = RIME_TEST_LOCK.lock().unwrap();
    let Some(shared_data_dir) = bundled_shared_data_dir() else {
        eprintln!("skip: Inputia bundled RimeData is not available");
        return;
    };
    let config = RimeEngineConfig::squirrel_luna_pinyin_simp(rime_user_data_dir())
        .with_shared_data_dir(shared_data_dir);
    if !config.dylib_path.exists() {
        eprintln!("skip: librime runtime is not installed on this machine");
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
    let Some(shared_data_dir) = bundled_shared_data_dir() else {
        eprintln!("skip: Inputia bundled RimeData is not available");
        return;
    };
    let config = RimeEngineConfig::squirrel_luna_pinyin_simp(rime_user_data_dir())
        .with_shared_data_dir(shared_data_dir);
    if !config.dylib_path.exists() {
        eprintln!("skip: librime runtime is not installed on this machine");
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
fn rime_engine_fills_core_page_beyond_rime_page_size_when_available() {
    let _guard = RIME_TEST_LOCK.lock().unwrap();
    let Some(shared_data_dir) = bundled_shared_data_dir() else {
        eprintln!("skip: Inputia bundled RimeData is not available");
        return;
    };
    let config = RimeEngineConfig::squirrel_luna_pinyin_simp(rime_user_data_dir())
        .with_shared_data_dir(shared_data_dir);
    if !config.dylib_path.exists() {
        eprintln!("skip: librime runtime is not installed on this machine");
        return;
    }

    let engine = RimeEngine::open(config).unwrap();
    let mut core = InputiaCore::new(
        CoreSettings {
            candidate_page_size: 9,
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
    assert_eq!(outcome.visible_candidates.len(), 9);
    assert!(outcome
        .visible_candidates
        .iter()
        .any(|candidate| candidate.text == "逆"));

    let page_down = core.handle_key(Key::PageDown);
    assert_eq!(page_down.snapshot.page, 1);
    assert!(!page_down.snapshot.visible_candidates.is_empty());
    assert_ne!(page_down.snapshot.visible_candidates[0].text, "你");
}

#[test]
fn rime_engine_keeps_remaining_input_after_partial_candidate_selection_when_available() {
    let _guard = RIME_TEST_LOCK.lock().unwrap();
    let Some(shared_data_dir) = bundled_shared_data_dir() else {
        eprintln!("skip: Inputia bundled RimeData is not available");
        return;
    };
    let config = RimeEngineConfig::squirrel_luna_pinyin_simp(rime_user_data_dir())
        .with_shared_data_dir(shared_data_dir)
        .with_schema("double_pinyin")
        .with_spelling_correction(false);
    if !config.dylib_path.exists() {
        eprintln!("skip: librime runtime is not installed on this machine");
        return;
    }

    let direct_engine = RimeEngine::open(config.clone()).unwrap();
    let direct_before = direct_engine.evaluate_incremental("nillem", 0).unwrap();
    let direct_selected = direct_engine
        .select_live_candidate("nillem", &direct_before.candidates[1], 0, 1)
        .unwrap();
    assert_eq!(direct_selected.commit, None);
    assert_eq!(direct_selected.input, "nillem");
    assert_eq!(direct_selected.preedit, "你laiem");
    assert_eq!(direct_selected.candidates[0].text, "来");

    let engine = RimeEngine::open(config).unwrap();
    let mut core = InputiaCore::new(CoreSettings::default(), engine);

    core.handle_key(Key::Shift);
    let mut outcome = core.snapshot();
    for ch in "nillem".chars() {
        outcome = core.handle_key(Key::Char(ch)).snapshot;
    }
    assert_eq!(outcome.composing, "nillem");
    assert_eq!(outcome.visible_candidates[0].text, "你来");
    assert_eq!(outcome.visible_candidates[1].text, "你");

    let selected = core.handle_key(Key::Digit(2));

    assert_eq!(selected.commit, None);
    assert_eq!(selected.snapshot.composing, "你laiem");
    assert!(!selected.snapshot.visible_candidates.is_empty());
    assert_eq!(selected.snapshot.visible_candidates[0].text, "来");
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
        eprintln!("skip: librime runtime is not installed on this machine");
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

fn bundled_shared_data_dir() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("INPUTIA_RIME_SHARED_DATA_DIR") {
        let path = PathBuf::from(path);
        if path.exists() {
            return Some(path);
        }
    }

    for path in [
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../macos/InputiaInputMethod/build/RimeData"),
        PathBuf::from("/Library/Input Methods/InputiaInputMethod.app/Contents/Resources/RimeData"),
    ] {
        if path.exists() {
            return Some(path);
        }
    }

    None
}

fn rime_user_data_dir() -> PathBuf {
    let path = std::env::temp_dir().join("inputia-rime-core-flow-user");
    std::fs::create_dir_all(&path).expect("rime user data dir should be writable");
    path
}
