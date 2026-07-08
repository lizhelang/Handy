use std::path::PathBuf;

use inputia_core::{ChineseEngine, CoreSettings, InputiaCore, Key};
use inputia_rime::{RimeEngine, RimeEngineConfig};

static RIME_SCHEMA_SMOKE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn bundled_rime_schemas_commit_zhongguo_when_available() {
    let _guard = RIME_SCHEMA_SMOKE_LOCK.lock().unwrap();
    let Some(shared_data_dir) = bundled_shared_data_dir() else {
        eprintln!("skip: Inputia bundled RimeData is not available");
        return;
    };

    let Some(dylib_path) = available_librime_dylib() else {
        eprintln!("skip: librime runtime is not installed on this machine");
        return;
    };

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

        let user_data_dir = std::env::temp_dir().join(format!(
            "inputia-rime-schema-smoke-{}-{}",
            std::process::id(),
            case.schema
        ));
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
fn bundled_rime_schemas_use_inputia_candidate_page_size_when_available() {
    let _guard = RIME_SCHEMA_SMOKE_LOCK.lock().unwrap();
    let Some(shared_data_dir) = bundled_shared_data_dir() else {
        eprintln!("skip: Inputia bundled RimeData is not available");
        return;
    };

    let Some(dylib_path) = available_librime_dylib() else {
        eprintln!("skip: librime runtime is not installed on this machine");
        return;
    };

    let cases = [
        ("luna_pinyin_simp", "ni"),
        ("double_pinyin", "nillem"),
        ("double_pinyin_sogou", "nillem"),
    ];
    for (schema, keys) in cases {
        let user_data_dir = std::env::temp_dir().join(format!(
            "inputia-rime-page-size-smoke-{}-{}-{}",
            std::process::id(),
            schema,
            keys
        ));
        let config = RimeEngineConfig::squirrel_luna_pinyin_simp(user_data_dir)
            .with_dylib_path(&dylib_path)
            .with_shared_data_dir(&shared_data_dir)
            .with_schema(schema);
        let engine = RimeEngine::open(config).expect("schema should open");
        let snapshot = engine.evaluate(keys).expect("schema should evaluate");

        assert_eq!(
            snapshot.page_size, 7,
            "{schema} should inherit Inputia's bundled menu/page_size for {keys}"
        );
        assert_eq!(
            snapshot.candidates.len(),
            7,
            "{schema} should expose seven candidates for {keys}"
        );
    }
}

#[test]
fn bundled_schemas_prioritize_segmented_phrase_candidates() {
    let _guard = RIME_SCHEMA_SMOKE_LOCK.lock().unwrap();
    let Some(shared_data_dir) = bundled_shared_data_dir() else {
        eprintln!("skip: Inputia bundled RimeData is not available");
        return;
    };

    let Some(dylib_path) = available_librime_dylib() else {
        eprintln!("skip: librime runtime is not installed on this machine");
        return;
    };

    let cases = [
        SegmentedPhraseSmokeCase {
            schema: "luna_pinyin_simp",
            keys: "nillem",
            expected_first: Some("你来了吗"),
        },
        SegmentedPhraseSmokeCase {
            schema: "double_pinyin",
            keys: "nillem",
            expected_first: Some("你来"),
        },
        SegmentedPhraseSmokeCase {
            schema: "double_pinyin_flypy",
            keys: "nillem",
            expected_first: None,
        },
        SegmentedPhraseSmokeCase {
            schema: "double_pinyin_sogou",
            keys: "nillem",
            expected_first: Some("你来"),
        },
        SegmentedPhraseSmokeCase {
            schema: "guobiao_bispell",
            keys: "nillem",
            expected_first: Some("你来了吗"),
        },
        SegmentedPhraseSmokeCase {
            schema: "double_pinyin_mspy",
            keys: "nillem",
            expected_first: Some("你来"),
        },
        SegmentedPhraseSmokeCase {
            schema: "double_pinyin_abc",
            keys: "nillem",
            expected_first: None,
        },
        SegmentedPhraseSmokeCase {
            schema: "double_pinyin_pyjj",
            keys: "nillem",
            expected_first: None,
        },
        SegmentedPhraseSmokeCase {
            schema: "double_pinyin_st",
            keys: "nillem",
            expected_first: None,
        },
    ];

    for case in cases {
        let user_data_dir = std::env::temp_dir().join(format!(
            "inputia-rime-segmented-phrase-smoke-{}-{}",
            std::process::id(),
            case.schema
        ));
        let config = RimeEngineConfig::squirrel_luna_pinyin_simp(user_data_dir)
            .with_dylib_path(&dylib_path)
            .with_shared_data_dir(&shared_data_dir)
            .with_schema(case.schema)
            .with_spelling_correction(false);
        let engine = RimeEngine::open(config).expect("schema should open");
        let candidates = engine.candidates(case.keys);
        let first = candidates
            .first()
            .expect("segmented keys should produce candidates");
        let first_len = first.text.chars().count();
        let first_single_index = candidates
            .iter()
            .position(|candidate| candidate.text.chars().count() == 1);

        assert!(
            first_len > 1,
            "{} should prefer a phrase candidate for segmented {}",
            case.schema,
            case.keys
        );
        if let Some(expected_first) = case.expected_first {
            assert_eq!(
                first.text, expected_first,
                "{} should keep the known phrase candidate first for {}",
                case.schema, case.keys
            );
        }
        if let Some(first_single_index) = first_single_index {
            assert!(
                first_single_index > 0,
                "{} should place single-character fallback after the first phrase for {}",
                case.schema,
                case.keys
            );
        }
    }
}

#[test]
fn bundled_full_pinyin_promotes_spelling_corrections_when_available() {
    let _guard = RIME_SCHEMA_SMOKE_LOCK.lock().unwrap();
    let Some(shared_data_dir) = bundled_shared_data_dir() else {
        eprintln!("skip: Inputia bundled RimeData is not available");
        return;
    };

    let Some(dylib_path) = available_librime_dylib() else {
        eprintln!("skip: librime runtime is not installed on this machine");
        return;
    };

    let cases = [
        ("zhonguo", "中国"),
        ("dagn", "当"),
        ("hoa", "好"),
        ("tain", "天"),
    ];
    for (keys, expected) in cases {
        let user_data_dir = std::env::temp_dir().join(format!(
            "inputia-rime-correction-smoke-{}-{}",
            std::process::id(),
            keys
        ));
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
fn bundled_double_pinyin_schemas_expose_maile_candidates_when_available() {
    let _guard = RIME_SCHEMA_SMOKE_LOCK.lock().unwrap();
    let Some(shared_data_dir) = bundled_shared_data_dir() else {
        eprintln!("skip: Inputia bundled RimeData is not available");
        return;
    };

    let Some(dylib_path) = available_librime_dylib() else {
        eprintln!("skip: librime runtime is not installed on this machine");
        return;
    };

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
        let user_data_dir = std::env::temp_dir().join(format!(
            "inputia-rime-maile-smoke-{}-{}-{}",
            std::process::id(),
            case.schema,
            case.keys
        ));
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

    let Some(dylib_path) = available_librime_dylib() else {
        eprintln!("skip: librime runtime is not installed on this machine");
        return;
    };

    let cases = [
        ("luna_pinyin_simp", "zhongguo"),
        ("double_pinyin", "mlle"),
        ("double_pinyin_sogou", "mlle"),
        ("guobiao_bispell", "mkle"),
    ];

    for (schema, keys) in cases {
        let user_data_dir = std::env::temp_dir().join(format!(
            "inputia-rime-incremental-smoke-{}-{}-{}",
            std::process::id(),
            schema,
            keys
        ));
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

#[derive(Clone, Copy)]
struct SegmentedPhraseSmokeCase {
    schema: &'static str,
    keys: &'static str,
    expected_first: Option<&'static str>,
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

fn available_librime_dylib() -> Option<PathBuf> {
    let config = RimeEngineConfig::squirrel_luna_pinyin_simp("/tmp/inputia-rime-runtime-probe");
    config.dylib_path.exists().then_some(config.dylib_path)
}
