use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

fn current_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "android") {
        "android"
    } else if cfg!(target_os = "ios") {
        "iOS"
    } else {
        panic!("unsupported target platform")
    }
}

fn permissions_for_window(window_label: &str) -> (HashSet<String>, Vec<Value>) {
    let capabilities_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities");
    let mut permissions = HashSet::new();
    let mut scoped_permissions = Vec::new();

    for entry in fs::read_dir(capabilities_dir).expect("failed to read capabilities directory") {
        let entry = entry.expect("failed to read capability entry");
        if entry.path().extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }

        let capability: Value = serde_json::from_slice(
            &fs::read(entry.path()).expect("failed to read capability file"),
        )
        .expect("failed to parse capability file");
        let applies_to_platform = capability["platforms"]
            .as_array()
            .map_or(true, |platforms| {
                platforms
                    .iter()
                    .any(|platform| platform == current_platform())
            });
        if !applies_to_platform {
            continue;
        }

        let covers_window = capability["windows"]
            .as_array()
            .is_some_and(|windows| windows.iter().any(|window| window == window_label));
        if !covers_window {
            continue;
        }

        if let Some(entries) = capability["permissions"].as_array() {
            for permission in entries {
                if let Some(identifier) = permission.as_str() {
                    permissions.insert(identifier.to_owned());
                } else {
                    scoped_permissions.push(permission.clone());
                }
            }
        }
    }

    (permissions, scoped_permissions)
}

#[test]
fn clipboard_overlay_has_required_core_permissions() {
    let (permissions, scoped_permissions) = permissions_for_window("clipboard_overlay");
    let expected = HashSet::from([
        "core:event:allow-listen".to_owned(),
        "core:event:allow-unlisten".to_owned(),
        "core:window:allow-start-dragging".to_owned(),
    ]);

    assert_eq!(
        permissions, expected,
        "clipboard_overlay must retain its exact least-privilege core permission set"
    );
    assert!(
        scoped_permissions.is_empty(),
        "clipboard_overlay must not receive additional scoped permissions"
    );
}
