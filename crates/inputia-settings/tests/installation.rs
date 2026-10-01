use inputia_settings::installation::*;
use std::path::PathBuf;

fn fixture() -> (LocatorContext, InstallationReceipt) {
    let home = PathBuf::from("/Users/fixture");
    let context = LocatorContext {
        product_id: PRODUCT_ID.into(),
        release_id: "inputia-release-1".into(),
        uid: 501,
        home: home.clone(),
    };
    let receipt = InstallationReceipt {
        schema_version: 1,
        product_id: PRODUCT_ID.into(),
        installation_id: "01020304-0506-4708-a90a-0b0c0d0e0f10".into(),
        profile_id: "aabbccdd-eeff-4011-a233-445566778899".into(),
        uid: 501,
        scope: InstallationScope::User,
        data: DataLocation::Managed,
        components: ComponentPaths {
            control: home.join("Applications/Inputia.app"),
            ime: home.join("Library/Input Methods/InputiaUnifiedCandidate.app"),
            settings: home.join("Applications/Inputia 设置.app"),
        },
        release_id: "inputia-release-1".into(),
        channel: UpdateChannel::Candidate,
    };
    (context, receipt)
}

#[test]
fn channel_change_preserves_data_and_binding() {
    let (context, mut receipt) = fixture();
    let candidate = receipt.clone().resolve(&context).unwrap();
    receipt.channel = UpdateChannel::Stable;
    let stable = receipt.resolve(&context).unwrap();
    assert_eq!(candidate.handy_root, stable.handy_root);
    assert_eq!(candidate.binding(), stable.binding());
    assert_eq!(
        stable.pair_manifest,
        context.home.join(
            "Library/Application Support/Inputia/Releases/inputia-release-1/pair-manifest.json"
        )
    );
}

#[test]
fn generated_release_id_with_version_dots_is_one_safe_path_segment() {
    let (mut context, mut receipt) = fixture();
    let release = "inputia-1.1.0-84-4eae6aed0000-0102030405060708090a0b0c0d0e0f10";
    context.release_id = release.into();
    receipt.release_id = release.into();
    assert!(receipt
        .clone()
        .resolve(&context)
        .unwrap()
        .pair_manifest
        .to_str()
        .unwrap()
        .contains(release));
    for invalid in [".", "..", "../escape", "a/b", "a\\b", "a\n"] {
        assert!(!valid_release_id(invalid));
    }
    receipt.data = DataLocation::LegacyCandidate {
        run_id: "trial.unsafe".into(),
    };
    receipt.profile_id = "unified-candidate:trial.unsafe".into();
    assert!(receipt.resolve(&context).is_err());
}

#[test]
fn legacy_trial_retains_exact_profile_and_both_data_roots() {
    let (context, mut receipt) = fixture();
    receipt.scope = InstallationScope::LegacySingleUser;
    receipt.components.control = "/Applications/Inputia.app".into();
    receipt.components.settings = "/Applications/Inputia 设置.app".into();
    receipt.data = DataLocation::LegacyCandidate {
        run_id: "trial-20260905".into(),
    };
    receipt.profile_id = "unified-candidate:trial-20260905".into();
    let located = receipt.resolve(&context).unwrap();
    assert_eq!(
        located.handy_root,
        context
            .home
            .join("Library/Application Support/HandyUnifiedCandidate/trial-20260905/Handy")
    );
    assert_eq!(
        located.inputia_root,
        context
            .home
            .join("Library/Application Support/HandyUnifiedCandidate/trial-20260905/Inputia")
    );
    assert_eq!(
        located.receipt.profile_id,
        "unified-candidate:trial-20260905"
    );
}

#[test]
fn rejects_wrong_identity_and_implicit_profile_migration() {
    let (context, receipt) = fixture();
    for field in [
        "product_id",
        "release_id",
        "uid",
        "installation_id",
        "profile_id",
    ] {
        let mut value = serde_json::to_value(&receipt).unwrap();
        value[field] = if field == "uid" {
            502.into()
        } else {
            "wrong".into()
        };
        let receipt = InstallationReceipt::parse(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(receipt.resolve(&context).is_err(), "{field}");
    }
    let mut changed = receipt;
    changed.data = DataLocation::LegacyCandidate {
        run_id: "trial-20260905".into(),
    };
    assert!(changed.resolve(&context).is_err());
}

#[test]
fn rejects_ambiguous_json_and_paths() {
    let (context, receipt) = fixture();
    let json = serde_json::to_string(&receipt).unwrap();
    assert!(InstallationReceipt::parse(json.replacen('{', "{\"uid\":501,", 1).as_bytes()).is_err());
    assert!(
        InstallationReceipt::parse(json.replacen('{', "{\"unknown\":true,", 1).as_bytes()).is_err()
    );
    assert!(InstallationReceipt::parse(&vec![b' '; MAX_RECEIPT_BYTES + 1]).is_err());
    for path in [
        "/Applications/Inputia.app",
        "/Users/fixture/Applications/../Inputia.app",
        "/Users/fixture//Applications/Inputia.app",
        "/Users/fixture/Applications/./Inputia.app",
        "relative.app",
    ] {
        let mut bad = receipt.clone();
        bad.components.control = path.into();
        assert!(bad.resolve(&context).is_err(), "{path}");
    }
}

#[cfg(unix)]
#[test]
fn private_receipt_read_rejects_links_permissions_and_oversize() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let path = root.join("receipt.json");
    let uid = unsafe { libc::geteuid() };
    std::fs::write(&path, b"{}").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(read_private_file(&path, uid, 100).unwrap(), b"{}");
    assert!(read_private_file(&path, uid, 1).is_err());
    assert!(read_private_file(&path, uid + 1, 100).is_err());
    let hard = root.join("hard");
    std::fs::hard_link(&path, &hard).unwrap();
    assert!(read_private_file(&path, uid, 100).is_err());
    std::fs::remove_file(hard).unwrap();
    let link = root.join("link");
    symlink(&path, &link).unwrap();
    assert!(read_private_file(&link, uid, 100).is_err());
    let linked_parent = root.join("linked-parent");
    symlink(&root, &linked_parent).unwrap();
    assert!(read_private_file(&linked_parent.join("receipt.json"), uid, 100).is_err());
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(read_private_file(&path, uid, 100).is_err());
}
