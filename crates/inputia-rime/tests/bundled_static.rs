#![cfg(feature = "bundled-static-rime")]

use inputia_rime::{RimeBackendKind, RimeEngine, RimeEngineConfig};
use std::path::PathBuf;

#[test]
fn static_api_ignores_dylib_paths_and_preserves_multi_session_lifetime() {
    let shared = PathBuf::from(
        std::env::var_os("INPUTIA_RIME_SHARED_DATA_DIR")
            .expect("static Rime tests require explicit candidate RimeData"),
    );
    assert!(shared.is_absolute() && shared.join("luna_pinyin_simp.schema.yaml").is_file());
    let first_profile = tempfile::tempdir().unwrap();
    let other_profile = tempfile::tempdir().unwrap();
    let first_config = RimeEngineConfig::squirrel_luna_pinyin_simp(first_profile.path())
        .with_shared_data_dir(&shared)
        .with_dylib_path("/synthetic/never-open-A.dylib");
    let first = RimeEngine::open(first_config.clone()).unwrap();
    assert_eq!(first.backend_kind(), RimeBackendKind::Static);
    let second =
        RimeEngine::open(first_config.with_dylib_path("/synthetic/never-open-B.dylib")).unwrap();
    assert_eq!(second.backend_kind(), RimeBackendKind::Static);
    let other = RimeEngineConfig::squirrel_luna_pinyin_simp(other_profile.path())
        .with_shared_data_dir(&shared)
        .with_dylib_path("/synthetic/never-open-C.dylib");
    assert!(
        RimeEngine::open(other.clone()).is_err(),
        "live Rime instances cannot switch global data domains"
    );
    drop(first);
    assert_eq!(
        second
            .evaluate_incremental("zhongguo", 0)
            .unwrap()
            .candidates[0]
            .text,
        "中国"
    );
    drop(second);
    let reopened = RimeEngine::open(other).unwrap();
    assert_eq!(
        reopened.evaluate("zhongguo").unwrap().candidates[0].text,
        "中国"
    );
}
