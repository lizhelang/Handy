use std::path::PathBuf;

use inputia_core::{ChineseEngine, CoreSettings, InputiaCore, Key};
use inputia_rime::{RimeEngine, RimeEngineConfig};

static RIME_SCHEMA_SMOKE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(feature = "bundled-static-rime")]
#[test]
fn explicit_hotword_promotes_late_native_candidate_and_selects_its_original_address() {
    use inputia_handy_runtime::personalization::{
        prioritize_explicit_candidates, Candidate as PersonalCandidate,
    };
    let _guard = RIME_SCHEMA_SMOKE_LOCK.lock().unwrap();
    let shared = bundled_shared_data_dir().unwrap();
    let user = tempfile::tempdir().unwrap();
    let engine = RimeEngine::open(
        RimeEngineConfig::squirrel_luna_pinyin_simp(user.path())
            .with_shared_data_dir(shared)
            .with_schema("double_pinyin"),
    )
    .unwrap();
    let pool = engine.candidates_up_to("zhongguo", 64);
    let selected = pool
        .iter()
        .skip(10)
        .find(|c| c.consumed_len == Some(8) && c.text.chars().count() == 2)
        .unwrap();
    let hotwords = vec![selected.text.clone()];
    let personal: Vec<_> = pool
        .iter()
        .enumerate()
        .map(|(rank, c)| PersonalCandidate {
            id: c.id.clone(),
            text: c.text.clone(),
            base_rank: rank,
            consumed_len: c.consumed_len.unwrap_or(0),
            match_type: "exact".into(),
        })
        .collect();
    let ids: Vec<_> = pool.iter().map(|c| c.id.clone()).collect();
    let promoted = prioritize_explicit_candidates(&personal, &ids, &hotwords).unwrap();
    assert_eq!(promoted[0], selected.id);
    let shared_order = inputia_core::shared_candidate_order("zhongguo", &pool, &hotwords);
    assert_eq!(pool[shared_order[0]].id, selected.id);
    assert_eq!(
        prioritize_explicit_candidates(&personal, &ids, &[]).unwrap(),
        ids
    );
    eprintln!(
        "explicit_hotword={} native_rank={} native_id={}",
        selected.text,
        personal.iter().position(|c| c.id == selected.id).unwrap(),
        selected.id
    );
    let committed = engine.select_candidate("zhongguo", 0, 0, selected).unwrap();
    assert_eq!(committed.commit, selected.text);
    assert!(committed.composing.is_empty());
}

#[cfg(feature = "bundled-static-rime")]
#[test]
fn natural_code_accepts_full_pinyin_and_keeps_native_partial_selection() {
    let _guard = RIME_SCHEMA_SMOKE_LOCK.lock().unwrap();
    let shared_data_dir = bundled_shared_data_dir().expect("explicit bundled resources required");
    let user_temp = tempfile::tempdir().unwrap();
    let config = RimeEngineConfig::squirrel_luna_pinyin_simp(user_temp.path())
        .with_shared_data_dir(&shared_data_dir)
        .with_schema("double_pinyin");
    let engine = RimeEngine::open(config.clone()).expect("natural-code schema should open");
    for (code, expected) in [
        ("edu", "额度"),
        ("vsgo", "中国"),
        ("zhongguo", "中国"),
        ("nihaoma", "你好吗"),
    ] {
        let candidates = engine.candidates_up_to(code, 256);
        let selected = candidates
            .iter()
            .find(|c| c.text == expected)
            .unwrap_or_else(|| panic!("{code} missing {expected}: {candidates:?}"));
        assert_eq!(
            engine.candidate_consumed_len(code, selected),
            Some(code.len())
        );
        let committed = engine.select_candidate(code, 0, 0, selected).unwrap();
        assert_eq!(committed.commit, expected);
        assert!(committed.composing.is_empty(), "{code}: {committed:?}");
    }
    for (code, consumed, rest) in [
        ("zhongguo", 5, "guo"),
        ("vsgo", 2, "go"),
        ("zhonggo", 5, "go"),
        ("vsguo", 2, "guo"),
    ] {
        let candidates = engine.candidates_up_to(code, 256);
        let selected = candidates
            .iter()
            .find(|c| c.text == "中")
            .unwrap_or_else(|| panic!("{code} missing prefix 中: {candidates:?}"));
        assert_eq!(
            engine.candidate_consumed_len(code, selected),
            Some(consumed)
        );
        let partial = engine.select_candidate(code, 0, 0, selected).unwrap();
        assert_eq!(partial.commit, "中");
        assert_eq!(partial.composing, rest);
        let country = partial.candidates.iter().find(|c| c.text == "国").unwrap();
        let completed = engine.select_candidate(rest, 0, 0, country).unwrap();
        assert_eq!(completed.commit, "国");
        assert!(completed.composing.is_empty());
    }
    // 选择非首位原生候选，确认全拼兼容没有绕过 Rime 用户词典学习。
    for _ in 0..3 {
        let candidates = engine.candidates_up_to("zhongguo", 256);
        let selected = candidates.iter().find(|c| c.text == "种果").unwrap();
        engine.select_candidate("zhongguo", 0, 0, selected).unwrap();
    }
    drop(engine);
    let reopened = RimeEngine::open(config).unwrap();
    assert_eq!(reopened.candidates("zhongguo")[0].text, "种果");
}

#[test]
fn bundled_rime_schemas_commit_zhongguo_when_available() {
    let _guard = RIME_SCHEMA_SMOKE_LOCK.lock().unwrap();
    let Some(shared_data_dir) = bundled_shared_data_dir() else {
        eprintln!("skip: Inputia bundled RimeData is not available");
        return;
    };

    let dylib_path =
        PathBuf::from("/Library/Input Methods/Squirrel.app/Contents/Frameworks/librime.1.dylib");
    if !cfg!(feature = "bundled-static-rime") && !dylib_path.exists() {
        eprintln!("skip: Squirrel librime runtime is not installed on this machine");
        return;
    }

    let cases = [
        SchemaSmokeCase {
            schema: "luna_pinyin_simp",
            keys: "zhongguo",
        },
        SchemaSmokeCase {
            schema: "double_pinyin",
            keys: "vsgo",
        },
        SchemaSmokeCase {
            schema: "double_pinyin_flypy",
            keys: "vsgo",
        },
        SchemaSmokeCase {
            schema: "double_pinyin_sogou",
            keys: "vsgo",
        },
        SchemaSmokeCase {
            schema: "double_pinyin_mspy",
            keys: "vsgo",
        },
        SchemaSmokeCase {
            schema: "double_pinyin_abc",
            keys: "asgo",
        },
        SchemaSmokeCase {
            schema: "double_pinyin_pyjj",
            keys: "vygo",
        },
        SchemaSmokeCase {
            schema: "double_pinyin_st",
            keys: "aygo",
        },
        SchemaSmokeCase {
            schema: "guobiao_bispell",
            keys: "vsgo",
        },
    ];

    for case in cases {
        let schema_file = shared_data_dir.join(format!("{}.schema.yaml", case.schema));
        assert!(
            schema_file.exists(),
            "schema file is missing for {} at {}",
            case.schema,
            schema_file.display()
        );

        let user_temp = tempfile::tempdir().unwrap();
        let user_data_dir = user_temp.path().to_path_buf();
        let config = RimeEngineConfig::squirrel_luna_pinyin_simp(user_data_dir)
            .with_dylib_path(&dylib_path)
            .with_shared_data_dir(&shared_data_dir)
            .with_schema(case.schema);
        let engine = RimeEngine::open(config).expect("schema should open");
        let mut core = InputiaCore::new(CoreSettings::default(), engine);

        core.handle_key(Key::Shift);
        let mut outcome = core.snapshot_outcome();
        for ch in case.keys.chars() {
            outcome = core.handle_key(Key::Char(ch));
        }

        assert_eq!(
            outcome
                .snapshot
                .visible_candidates
                .first()
                .map(|candidate| candidate.text.as_str()),
            Some("中国"),
            "{} should rank 中国 first for {}",
            case.schema,
            case.keys
        );

        let committed = core.handle_key(Key::Space);
        assert_eq!(
            committed.commit.as_deref(),
            Some("中国"),
            "{} should commit 中国 for {}",
            case.schema,
            case.keys
        );
    }
}

#[test]
fn bundled_double_pinyin_reports_prefix_consumption_for_partial_candidate_commit() {
    let _guard = RIME_SCHEMA_SMOKE_LOCK.lock().unwrap();
    let Some(shared_data_dir) = bundled_shared_data_dir() else {
        eprintln!("skip: Inputia bundled RimeData is not available");
        return;
    };

    let dylib_path =
        PathBuf::from("/Library/Input Methods/Squirrel.app/Contents/Frameworks/librime.1.dylib");
    if !cfg!(feature = "bundled-static-rime") && !dylib_path.exists() {
        eprintln!("skip: Squirrel librime runtime is not installed on this machine");
        return;
    }

    let user_temp = tempfile::tempdir().unwrap();
    let user_data_dir = user_temp.path().to_path_buf();
    let config = RimeEngineConfig::squirrel_luna_pinyin_simp(user_data_dir)
        .with_dylib_path(&dylib_path)
        .with_shared_data_dir(&shared_data_dir)
        .with_schema("double_pinyin");
    let engine = RimeEngine::open(config).expect("schema should open");
    let candidates = engine.candidates("nilllema");
    let single = candidates
        .iter()
        .find(|candidate| candidate.text == "你")
        .expect("single-character prefix candidate should be present");
    let phrase = candidates
        .iter()
        .find(|candidate| candidate.text == "你来了吗")
        .expect("full phrase candidate should be present");

    assert_eq!(engine.candidate_consumed_len("nilllema", single), Some(2));
    assert_eq!(
        engine.candidate_consumed_len("nilllema", phrase),
        Some("nilllema".len())
    );
}

#[test]
fn bundled_double_pinyin_selection_preserves_remaining_input_for_partial_commit() {
    let _guard = RIME_SCHEMA_SMOKE_LOCK.lock().unwrap();
    let Some(shared_data_dir) = bundled_shared_data_dir() else {
        eprintln!("skip: Inputia bundled RimeData is not available");
        return;
    };

    let dylib_path =
        PathBuf::from("/Library/Input Methods/Squirrel.app/Contents/Frameworks/librime.1.dylib");
    if !cfg!(feature = "bundled-static-rime") && !dylib_path.exists() {
        eprintln!("skip: Squirrel librime runtime is not installed on this machine");
        return;
    }

    let user_temp = tempfile::tempdir().unwrap();
    let user_data_dir = user_temp.path().to_path_buf();
    let config = RimeEngineConfig::squirrel_luna_pinyin_simp(user_data_dir)
        .with_dylib_path(&dylib_path)
        .with_shared_data_dir(&shared_data_dir)
        .with_schema("double_pinyin");
    let engine = RimeEngine::open(config).expect("schema should open");
    let candidates = engine.candidates("nilllema");
    let single = candidates
        .iter()
        .find(|candidate| candidate.text == "你")
        .expect("single-character prefix candidate should be present")
        .clone();

    let partial = engine
        .select_candidate("nilllema", 0, 0, &single)
        .expect("single-character selection should keep live Rime input");
    assert_eq!(partial.commit, "你");
    assert_eq!(partial.composing, "lllema");
    assert_eq!(
        partial
            .candidates
            .first()
            .map(|candidate| candidate.text.as_str()),
        Some("来了吗")
    );

    let rest = partial
        .candidates
        .first()
        .expect("remaining input should still have candidates")
        .clone();
    let committed_rest = engine
        .select_candidate(&partial.composing, 0, 0, &rest)
        .expect("remaining input should commit normally");
    assert_eq!(committed_rest.commit, "来了吗");
    assert!(committed_rest.composing.is_empty());
    assert!(committed_rest.candidates.is_empty());
}

#[test]
fn bundled_full_pinyin_promotes_spelling_corrections_when_available() {
    let _guard = RIME_SCHEMA_SMOKE_LOCK.lock().unwrap();
    let Some(shared_data_dir) = bundled_shared_data_dir() else {
        eprintln!("skip: Inputia bundled RimeData is not available");
        return;
    };

    let dylib_path =
        PathBuf::from("/Library/Input Methods/Squirrel.app/Contents/Frameworks/librime.1.dylib");
    if !cfg!(feature = "bundled-static-rime") && !dylib_path.exists() {
        eprintln!("skip: Squirrel librime runtime is not installed on this machine");
        return;
    }

    let cases = [
        ("zhonguo", "中国"),
        ("dagn", "当"),
        ("hoa", "好"),
        ("tain", "天"),
    ];
    for (keys, expected) in cases {
        let user_temp = tempfile::tempdir().unwrap();
        let user_data_dir = user_temp.path().to_path_buf();
        let config = RimeEngineConfig::squirrel_luna_pinyin_simp(user_data_dir)
            .with_dylib_path(&dylib_path)
            .with_shared_data_dir(&shared_data_dir)
            .with_schema("luna_pinyin_simp")
            .with_spelling_correction(true);
        let engine = RimeEngine::open(config).expect("schema should open");
        let candidates = engine.candidates(keys);

        assert_eq!(
            candidates.first().map(|candidate| candidate.text.as_str()),
            Some(expected),
            "{} should promote corrected candidate {}",
            keys,
            expected
        );
        assert!(
            candidates
                .first()
                .map(|candidate| candidate.id.starts_with("rime-correction:"))
                .unwrap_or(false),
            "{} should come from the correction path",
            keys
        );
    }
}

#[test]
fn bundled_inputia_extension_lexicons_promote_poetry_idiom_and_rare_char_when_available() {
    let _guard = RIME_SCHEMA_SMOKE_LOCK.lock().unwrap();
    let Some(shared_data_dir) = bundled_shared_data_dir() else {
        eprintln!("skip: Inputia bundled RimeData is not available");
        return;
    };

    let dylib_path =
        PathBuf::from("/Library/Input Methods/Squirrel.app/Contents/Frameworks/librime.1.dylib");
    if !cfg!(feature = "bundled-static-rime") && !dylib_path.exists() {
        eprintln!("skip: Squirrel librime runtime is not installed on this machine");
        return;
    }

    for dict in [
        "inputia_luna_pinyin.dict.yaml",
        "inputia_idiom.dict.yaml",
        "inputia_poetry.dict.yaml",
        "inputia_classical.dict.yaml",
        "inputia_ext_chars.dict.yaml",
    ] {
        assert!(
            shared_data_dir.join(dict).exists(),
            "Inputia extension dictionary is missing: {dict}"
        );
    }

    let user_temp = tempfile::tempdir().unwrap();
    let user_data_dir = user_temp.path().to_path_buf();
    let config = RimeEngineConfig::squirrel_luna_pinyin_simp(user_data_dir)
        .with_dylib_path(&dylib_path)
        .with_shared_data_dir(&shared_data_dir)
        .with_schema("luna_pinyin_simp")
        .with_spelling_correction(false);
    let engine = RimeEngine::open(config).expect("schema should open");

    let extension_cases = [
        ("changfengpolang", "长风破浪"),
        ("huiyoushi", "会有时"),
        ("qianfanguo", "千帆过"),
        ("lankeren", "烂柯人"),
        ("yindizhiyi", "因地制宜"),
        ("biang", "𰻞"),
    ];

    for (keys, expected) in extension_cases {
        let candidates = engine.candidates(keys);
        assert!(
            candidates
                .iter()
                .take(10)
                .any(|candidate| candidate.text == expected),
            "{keys} should include {expected} in the first two pages with Inputia extension lexicons"
        );
    }

    for (keys, expected_first) in [("nihao", "你好"), ("f", "发"), ("da", "打")] {
        let candidates = engine.candidates(keys);
        assert_eq!(
            candidates.first().map(|candidate| candidate.text.as_str()),
            Some(expected_first),
            "{keys} should keep the common base candidate {expected_first} first"
        );
    }
}

#[test]
fn bundled_double_pinyin_schemas_expose_maile_candidates_when_available() {
    let _guard = RIME_SCHEMA_SMOKE_LOCK.lock().unwrap();
    let Some(shared_data_dir) = bundled_shared_data_dir() else {
        eprintln!("skip: Inputia bundled RimeData is not available");
        return;
    };

    let dylib_path =
        PathBuf::from("/Library/Input Methods/Squirrel.app/Contents/Frameworks/librime.1.dylib");
    if !cfg!(feature = "bundled-static-rime") && !dylib_path.exists() {
        eprintln!("skip: Squirrel librime runtime is not installed on this machine");
        return;
    }

    let cases = [
        MaileSmokeCase {
            schema: "double_pinyin",
            keys: "mlle",
            expected_first: Some("买了"),
            expected_present: "买了",
        },
        MaileSmokeCase {
            schema: "guobiao_bispell",
            keys: "mlle",
            expected_first: None,
            expected_present: "买了",
        },
        MaileSmokeCase {
            schema: "guobiao_bispell",
            keys: "mkle",
            expected_first: Some("买了"),
            expected_present: "买了",
        },
    ];

    for case in cases {
        let user_temp = tempfile::tempdir().unwrap();
        let user_data_dir = user_temp.path().to_path_buf();
        let config = RimeEngineConfig::squirrel_luna_pinyin_simp(user_data_dir)
            .with_dylib_path(&dylib_path)
            .with_shared_data_dir(&shared_data_dir)
            .with_schema(case.schema);
        let engine = RimeEngine::open(config).expect("schema should open");
        let candidates = engine.candidates(case.keys);

        if let Some(expected_first) = case.expected_first {
            assert_eq!(
                candidates.first().map(|candidate| candidate.text.as_str()),
                Some(expected_first),
                "{} should rank {} first for {}",
                case.schema,
                expected_first,
                case.keys
            );
        }
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.text == case.expected_present),
            "{} should include {} for {} instead of falling back to raw letters",
            case.schema,
            case.expected_present,
            case.keys
        );
    }
}

#[test]
fn bundled_incremental_session_matches_cold_evaluate_when_available() {
    let _guard = RIME_SCHEMA_SMOKE_LOCK.lock().unwrap();
    let Some(shared_data_dir) = bundled_shared_data_dir() else {
        eprintln!("skip: Inputia bundled RimeData is not available");
        return;
    };

    let dylib_path =
        PathBuf::from("/Library/Input Methods/Squirrel.app/Contents/Frameworks/librime.1.dylib");
    if !cfg!(feature = "bundled-static-rime") && !dylib_path.exists() {
        eprintln!("skip: Squirrel librime runtime is not installed on this machine");
        return;
    }

    let cases = [
        ("luna_pinyin_simp", "zhongguo"),
        ("double_pinyin", "mlle"),
        ("double_pinyin_sogou", "mlle"),
        ("guobiao_bispell", "mkle"),
    ];

    for (schema, keys) in cases {
        let user_temp = tempfile::tempdir().unwrap();
        let user_data_dir = user_temp.path().to_path_buf();
        let config = RimeEngineConfig::squirrel_luna_pinyin_simp(user_data_dir)
            .with_dylib_path(&dylib_path)
            .with_shared_data_dir(&shared_data_dir)
            .with_schema(schema)
            .with_spelling_correction(false);
        let engine = RimeEngine::open(config).expect("schema should open");

        let cold = engine.evaluate(keys).expect("cold evaluation should work");
        let incremental = engine
            .evaluate_incremental(keys, 0)
            .expect("incremental evaluation should work");

        assert_eq!(
            incremental
                .candidates
                .first()
                .map(|candidate| candidate.text.as_str()),
            cold.candidates
                .first()
                .map(|candidate| candidate.text.as_str()),
            "{schema} should keep the same first candidate for {keys}"
        );
        assert_eq!(
            incremental
                .candidates
                .iter()
                .take(5)
                .map(|candidate| candidate.text.as_str())
                .collect::<Vec<_>>(),
            cold.candidates
                .iter()
                .take(5)
                .map(|candidate| candidate.text.as_str())
                .collect::<Vec<_>>(),
            "{schema} should keep top candidates stable for {keys}"
        );
    }
}

#[derive(Clone, Copy)]
struct SchemaSmokeCase {
    schema: &'static str,
    keys: &'static str,
}

#[derive(Clone, Copy)]
struct MaileSmokeCase {
    schema: &'static str,
    keys: &'static str,
    expected_first: Option<&'static str>,
    expected_present: &'static str,
}

trait SnapshotOutcome {
    fn snapshot_outcome(&self) -> inputia_core::InputOutcome;
}

impl<E: inputia_core::ChineseEngine> SnapshotOutcome for InputiaCore<E> {
    fn snapshot_outcome(&self) -> inputia_core::InputOutcome {
        inputia_core::InputOutcome {
            consumed: false,
            commit: None,
            snapshot: self.snapshot(),
        }
    }
}

#[cfg(feature = "bundled-static-rime")]
fn bundled_shared_data_dir() -> Option<PathBuf> {
    let path = PathBuf::from(
        std::env::var_os("INPUTIA_RIME_SHARED_DATA_DIR")
            .expect("static schema tests require explicit candidate RimeData"),
    );
    assert!(path.is_absolute() && path.join("luna_pinyin_simp.schema.yaml").is_file());
    Some(path)
}

#[cfg(not(feature = "bundled-static-rime"))]
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
