#![cfg(not(feature = "bundled-static-rime"))]

use inputia_rime::{RimeEngine, RimeEngineConfig};

#[test]
fn default_dynamic_mode_still_rejects_a_missing_external_library() {
    let user = tempfile::tempdir().unwrap();
    let config = RimeEngineConfig::squirrel_luna_pinyin_simp(user.path())
        .with_dylib_path(user.path().join("does-not-exist.dylib"));
    assert!(RimeEngine::open(config).is_err());
}
