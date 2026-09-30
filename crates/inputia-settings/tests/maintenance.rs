#![cfg(unix)]

use inputia_settings::{installation::*, maintenance::*};
use std::{
    fs,
    os::unix::fs::{symlink, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};

fn marker() -> MaintenanceMarker {
    MaintenanceMarker {
        schema_version: 1,
        transaction_id: "11111111-1111-4111-8111-111111111111".into(),
        installation_id: "22222222-2222-4222-8222-222222222222".into(),
        old_release_id: Some("inputia-old".into()),
        new_release_id: "inputia-1.1.0-new".into(),
        epoch: "33333333-3333-4333-8333-333333333333".into(),
        plan_sha256: "a".repeat(64),
    }
}
fn write(path: &Path, bytes: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}
fn fixture() -> (tempfile::TempDir, PathBuf, u32) {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().canonicalize().unwrap().join("home");
    fs::create_dir(&home).unwrap();
    let uid = home.metadata().unwrap().uid();
    (temp, home, uid)
}

#[test]
fn no_receipt_or_pair_version_is_needed_to_stop_startup_and_inspection_is_read_only() {
    let (_temp, home, uid) = fixture();
    assert_eq!(inspect(&home, uid).unwrap(), None);
    ensure_normal_start(&home, uid).unwrap();
    assert!(!home.join("Library").exists());
    let path = marker_path(&home);
    write(&path, &serde_json::to_vec(&marker()).unwrap());
    assert_eq!(inspect(&home, uid).unwrap(), Some(marker()));
    assert_eq!(
        ensure_normal_start(&home, uid),
        Err(MaintenanceError::MaintenanceActive)
    );
    assert!(!home
        .join("Library/Application Support/Inputia/installation.json")
        .exists());
    fs::write(&path, b"{").unwrap();
    assert_eq!(
        ensure_normal_start(&home, uid),
        Err(MaintenanceError::InvalidMarker)
    );
    assert_eq!(fs::read(&path).unwrap(), b"{");
}

#[test]
fn marker_rejects_duplicates_unknown_missing_fields_and_invalid_binding_values() {
    let mut value = serde_json::to_value(marker()).unwrap();
    let parse =
        |value: &serde_json::Value| MaintenanceMarker::parse(&serde_json::to_vec(value).unwrap());
    value["command"] = "ignored-no-longer".into();
    assert!(parse(&value).is_err());
    value.as_object_mut().unwrap().remove("command");
    value.as_object_mut().unwrap().remove("old_release_id");
    assert!(parse(&value).is_err());
    value["old_release_id"] = serde_json::Value::Null;
    assert!(parse(&value).is_ok());
    for (field, bad) in [
        ("schema_version", serde_json::json!(2)),
        ("transaction_id", serde_json::json!("../outside")),
        (
            "installation_id",
            serde_json::json!("00000000-0000-0000-0000-000000000000"),
        ),
        ("epoch", serde_json::json!("stale")),
        ("new_release_id", serde_json::json!("untrusted")),
        ("old_release_id", serde_json::json!("inputia-1.1.0-new")),
        ("plan_sha256", serde_json::json!("A".repeat(64))),
    ] {
        let mut invalid = value.clone();
        invalid[field] = bad;
        assert!(parse(&invalid).is_err(), "{field}");
    }
    let text = serde_json::to_string(&marker()).unwrap();
    let duplicate = format!("{{\"epoch\":\"{}\",{}", marker().epoch, &text[1..]);
    assert!(MaintenanceMarker::parse(duplicate.as_bytes()).is_err());
    assert!(MaintenanceMarker::parse(&vec![b' '; MAX_MARKER_BYTES + 1]).is_err());
}

#[test]
fn marker_mode_hardlinks_leaf_and_ancestor_symlinks_fail_closed() {
    let (temp, home, uid) = fixture();
    let path = marker_path(&home);
    let bytes = serde_json::to_vec(&marker()).unwrap();
    write(&path, &bytes);
    for mode in [0o644, 0o700, 0o660, 0o4600] {
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        assert_eq!(inspect(&home, uid), Err(MaintenanceError::UnsafePath));
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let link = temp.path().join("hardlink");
    fs::hard_link(&path, &link).unwrap();
    assert_eq!(inspect(&home, uid), Err(MaintenanceError::UnsafePath));
    fs::remove_file(&link).unwrap();
    fs::rename(&path, &link).unwrap();
    symlink(&link, &path).unwrap();
    assert_eq!(inspect(&home, uid), Err(MaintenanceError::UnsafePath));
    fs::remove_file(&path).unwrap();
    fs::rename(&link, &path).unwrap();
    let real = temp.path().join("moved-library");
    fs::rename(home.join("Library"), &real).unwrap();
    symlink(&real, home.join("Library")).unwrap();
    assert_eq!(inspect(&home, uid), Err(MaintenanceError::UnsafePath));
}

#[test]
fn invalid_home_wrong_uid_directory_and_oversized_marker_are_not_absent() {
    let (_temp, home, uid) = fixture();
    assert_eq!(
        inspect(&home.join(".."), uid),
        Err(MaintenanceError::UnsafePath)
    );
    assert_eq!(
        inspect(&home, uid.wrapping_add(1)),
        Err(MaintenanceError::UnsafePath)
    );
    let path = marker_path(&home);
    fs::create_dir_all(&path).unwrap();
    assert_eq!(inspect(&home, uid), Err(MaintenanceError::UnsafePath));
    fs::remove_dir(&path).unwrap();
    write(&path, &vec![0; MAX_MARKER_BYTES + 1]);
    assert_eq!(inspect(&home, uid), Err(MaintenanceError::UnsafePath));
}

#[test]
fn read_only_postcheck_requires_exact_marker_new_receipt_and_build_binding() {
    let (_temp, home, uid) = fixture();
    let marker = marker();
    write(&marker_path(&home), &serde_json::to_vec(&marker).unwrap());
    let context = LocatorContext {
        product_id: PRODUCT_ID.into(),
        release_id: marker.new_release_id.clone(),
        home: home.clone(),
        uid,
    };
    let receipt = InstallationReceipt {
        schema_version: 1,
        product_id: PRODUCT_ID.into(),
        installation_id: marker.installation_id.clone(),
        profile_id: "44444444-4444-4444-8444-444444444444".into(),
        uid,
        scope: InstallationScope::User,
        data: DataLocation::Managed,
        components: ComponentPaths {
            control: home.join("Applications/Inputia.app"),
            ime: home.join("Library/Input Methods/InputiaUnifiedCandidate.app"),
            settings: home.join("Applications/Inputia 设置.app"),
        },
        release_id: marker.new_release_id.clone(),
        channel: UpdateChannel::Candidate,
    };
    let request = ReadOnlyPostcheckRequest {
        transaction_id: marker.transaction_id.clone(),
        installation_id: marker.installation_id.clone(),
        epoch: marker.epoch.clone(),
        plan_sha256: marker.plan_sha256.clone(),
    };
    assert!(inspect_read_only_postcheck(&context, &request).is_err());
    write(
        &context.receipt_path(),
        &serde_json::to_vec(&receipt).unwrap(),
    );
    let report = inspect_read_only_postcheck(&context, &request).unwrap();
    assert!(
        !report.user_databases_opened
            && !report.runtime_handshake_verified
            && !report.code_signature_verified
    );
    assert!(!report.installation.inputia_root.exists());
    assert_eq!(
        ensure_normal_start(&home, uid),
        Err(MaintenanceError::MaintenanceActive)
    );
    let mut bad = request.clone();
    bad.epoch = "55555555-5555-4555-8555-555555555555".into();
    assert!(inspect_read_only_postcheck(&context, &bad).is_err());
    bad = request.clone();
    bad.transaction_id = bad.epoch.clone();
    assert!(inspect_read_only_postcheck(&context, &bad).is_err());
    bad = request.clone();
    bad.plan_sha256 = "b".repeat(64);
    assert!(inspect_read_only_postcheck(&context, &bad).is_err());
    let mut wrong_build = context.clone();
    wrong_build.release_id = "inputia-old".into();
    assert!(inspect_read_only_postcheck(&wrong_build, &request).is_err());
    wrong_build = context.clone();
    wrong_build.product_id = "other-product".into();
    assert!(inspect_read_only_postcheck(&wrong_build, &request).is_err());
    let mut old = receipt.clone();
    old.release_id = "inputia-old".into();
    write(&context.receipt_path(), &serde_json::to_vec(&old).unwrap());
    assert!(inspect_read_only_postcheck(&context, &request).is_err());
    write(
        &context.receipt_path(),
        &serde_json::to_vec(&receipt).unwrap(),
    );
    fs::remove_file(marker_path(&home)).unwrap();
    assert!(matches!(
        inspect_read_only_postcheck(&context, &request),
        Err(MaintenanceError::MissingMarker)
    ));
}
