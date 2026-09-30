//! 无 session 的共享维护 ABI；不加载 Rime、数据库、音频或运行时 manager。

use inputia_settings::maintenance::{self, MaintenanceError, UserContext};
use std::os::raw::c_char;

fn startup_result(context: Result<UserContext, MaintenanceError>) -> *mut c_char {
    let result =
        context.and_then(|context| maintenance::ensure_normal_start(&context.home, context.uid));
    match result {
        Ok(()) => super::string_json(&serde_json::json!({"ok":true,"normal_start_allowed":true})),
        Err(error) => super::string_json(
            &serde_json::json!({"ok":false,"normal_start_allowed":false,"code":error}),
        ),
    }
}

/// 只从系统 euid/账户目录读取固定维护标记；返回字符串用 inputia_string_free 释放。
#[no_mangle]
pub extern "C" fn inputia_maintenance_startup_check() -> *mut c_char {
    startup_result(maintenance::current_user_context())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        ffi::CStr,
        os::unix::fs::{MetadataExt, PermissionsExt},
    };

    #[test]
    fn maintenance_abi_denies_bad_and_active_markers_without_any_session() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().canonicalize().unwrap();
        let uid = home.metadata().unwrap().uid();
        let call = || {
            let pointer = startup_result(Ok(UserContext {
                home: home.clone(),
                uid,
            }));
            let value: serde_json::Value =
                serde_json::from_slice(unsafe { CStr::from_ptr(pointer) }.to_bytes()).unwrap();
            unsafe { crate::inputia_string_free(pointer) };
            value
        };
        assert_eq!(
            call(),
            serde_json::json!({"ok":true,"normal_start_allowed":true})
        );
        let path = maintenance::marker_path(&home);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"corrupt").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(
            call(),
            serde_json::json!({"ok":false,"normal_start_allowed":false,"code":"invalid_marker"})
        );
        std::fs::write(&path, serde_json::to_vec(&serde_json::json!({
            "schema_version":1,"transaction_id":"11111111-1111-4111-8111-111111111111",
            "installation_id":"22222222-2222-4222-8222-222222222222","old_release_id":null,
            "new_release_id":"inputia-1.1.0-test","epoch":"33333333-3333-4333-8333-333333333333",
            "plan_sha256":"a".repeat(64)
        })).unwrap()).unwrap();
        assert_eq!(
            call(),
            serde_json::json!({"ok":false,"normal_start_allowed":false,"code":"maintenance_active"})
        );
        assert!(!home
            .join("Library/Application Support/Inputia/installation.json")
            .exists());
    }
}
