//! 无 session 的安装定位 ABI；基础输入初始化前即可调用，不加载 Rime。

use std::{ffi::CStr, os::raw::c_char};

/// 按来自构建常量/系统身份的上下文读取并验证固定路径安装收据。
///
/// # Safety
/// context_json 必须为有效 NUL 结尾 UTF-8 字符串，调用期间不可修改。
/// 返回 JSON 字符串由 inputia_string_free 释放；失败仅返回类型，不回显用户路径。
#[no_mangle]
pub unsafe extern "C" fn inputia_installation_load(context_json: *const c_char) -> *mut c_char {
    use inputia_settings::installation::{InstallationError, LocatorContext};
    let load = || {
        if context_json.is_null() {
            return Err(InstallationError::InvalidReceipt);
        }
        let bytes = unsafe { CStr::from_ptr(context_json) }.to_bytes();
        if bytes.len() > 4096 {
            return Err(InstallationError::InvalidReceipt);
        }
        let context: LocatorContext =
            serde_json::from_slice(bytes).map_err(|_| InstallationError::InvalidReceipt)?;
        #[cfg(unix)]
        {
            inputia_settings::installation::load(&context)
        }
        #[cfg(not(unix))]
        {
            let _ = context;
            Err::<inputia_settings::installation::LocatedInstallation, _>(
                InstallationError::Unavailable,
            )
        }
    };
    match load() {
        Ok(installation) => {
            super::string_json(&serde_json::json!({"ok":true,"installation":installation}))
        }
        Err(error) => super::string_json(&serde_json::json!({"ok":false,"code":error})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installation_abi_rejects_null_without_session_or_path_leak() {
        let raw = unsafe { inputia_installation_load(std::ptr::null()) };
        assert!(!raw.is_null());
        let value: serde_json::Value =
            serde_json::from_slice(unsafe { CStr::from_ptr(raw) }.to_bytes()).unwrap();
        unsafe { crate::inputia_string_free(raw) };
        assert_eq!(
            value,
            serde_json::json!({"ok":false,"code":"invalid_receipt"})
        );
    }

    #[cfg(unix)]
    #[test]
    fn installation_abi_loads_shared_receipt_without_rime_session() {
        use std::{
            ffi::CString,
            os::unix::fs::{MetadataExt, PermissionsExt},
        };
        let temporary = tempfile::tempdir().unwrap();
        let home = temporary.path().canonicalize().unwrap();
        let uid = home.metadata().unwrap().uid();
        let path = home.join("Library/Application Support/Inputia/installation.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let receipt = serde_json::json!({
            "schema_version":1, "product_id":"com.inputia", "release_id":"inputia-1.1.0-84-fixture",
            "installation_id":"01020304-0506-4708-a90a-0b0c0d0e0f10", "profile_id":"unified-candidate:trial-20260905",
            "uid":uid, "scope":"legacy_single_user", "data":{"kind":"legacy_candidate", "run_id":"trial-20260905"},
            "channel":"candidate", "components":{"control":"/Applications/Inputia.app", "ime":home.join("Library/Input Methods/InputiaUnifiedCandidate.app"), "settings":"/Applications/Inputia 设置.app"}
        });
        std::fs::write(&path, serde_json::to_vec(&receipt).unwrap()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let context = CString::new(serde_json::json!({"product_id":"com.inputia", "release_id":"inputia-1.1.0-84-fixture", "uid":uid, "home":home}).to_string()).unwrap();
        let raw = unsafe { inputia_installation_load(context.as_ptr()) };
        let value: serde_json::Value =
            serde_json::from_slice(unsafe { CStr::from_ptr(raw) }.to_bytes()).unwrap();
        unsafe { crate::inputia_string_free(raw) };
        assert_eq!(value["ok"], true);
        assert_eq!(
            value["installation"]["receipt"]["profile_id"],
            "unified-candidate:trial-20260905"
        );
        assert_eq!(
            value["installation"]["handy_root"],
            home.join("Library/Application Support/HandyUnifiedCandidate/trial-20260905/Handy")
                .to_str()
                .unwrap()
        );
    }
}
