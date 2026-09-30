//! 不依赖 Rime 或主服务的基础设置 CAS。路径只能位于系统用户目录。
use std::{ffi::CStr, os::raw::c_char};

/// # Safety
/// request_json 是调用期间保持可读、NUL 结尾的 UTF-8 字符串；返回值用 inputia_string_free 释放。
#[no_mangle]
pub unsafe extern "C" fn inputia_settings_request(request_json: *const c_char) -> *mut c_char {
    if request_json.is_null() {
        return super::string_json(&serde_json::json!({"ok":false,"code":"invalid_request"}));
    }
    let raw = unsafe { CStr::from_ptr(request_json) }.to_bytes();
    if raw.len() > 512 * 1024 {
        return super::string_json(&serde_json::json!({"ok":false,"code":"invalid_request"}));
    }
    #[cfg(unix)]
    let reply = match inputia_settings::maintenance::current_user_context() {
        Ok(context) => handle(raw, &context),
        Err(_) => serde_json::json!({"ok":false,"code":"storage_unavailable"}),
    };
    #[cfg(not(unix))]
    let reply = serde_json::json!({"ok":false,"code":"unsupported_platform"});
    super::string_json(&reply)
}

#[cfg(unix)]
fn handle(raw: &[u8], context: &inputia_settings::maintenance::UserContext) -> serde_json::Value {
    use inputia_settings::store::{self, ImportRequest, PatchRequest, Store};
    #[derive(serde::Deserialize)]
    #[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
    enum Request {
        Read {
            path: std::path::PathBuf,
        },
        InspectExternal {
            path: std::path::PathBuf,
        },
        #[cfg(target_os = "macos")]
        ApplicationStatus {
            path: std::path::PathBuf,
        },
        ImportExternal {
            path: std::path::PathBuf,
            request: ImportRequest,
        },
        Apply {
            path: std::path::PathBuf,
            request: PatchRequest,
        },
    }
    let operation = || -> store::Result<serde_json::Value> {
        let request: Request = serde_json::from_value(store::strict_json(raw)?)
            .map_err(|_| store::Error::InvalidRequest)?;
        let path = match &request {
            #[cfg(target_os = "macos")]
            Request::ApplicationStatus { path } => path,
            Request::Read { path }
            | Request::Apply { path, .. }
            | Request::InspectExternal { path }
            | Request::ImportExternal { path, .. } => path,
        };
        let store = Store::open(path, &context.home, context.uid)?;
        match request {
            #[cfg(target_os = "macos")]
            Request::ApplicationStatus { .. } => {
                Ok(serde_json::json!({"ok":true,"application":store.application_status()?}))
            }
            Request::Read { .. } => Ok(serde_json::json!({"ok":true,"snapshot":store.read()?})),
            Request::InspectExternal { .. } => {
                Ok(serde_json::json!({"ok":true,"external":store.inspect_external()?}))
            }
            Request::ImportExternal { request, .. } => {
                Ok(serde_json::json!({"ok":true,"result":store.import_external(&request)?}))
            }
            Request::Apply { request, .. } => {
                Ok(serde_json::json!({"ok":true,"result":store.apply(&request)?}))
            }
        }
    };
    operation().unwrap_or_else(|error| serde_json::json!({"ok":false,"code":error}))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use inputia_settings::{maintenance::UserContext, store::Snapshot};
    use serde_json::json;
    #[test]
    fn offline_snapshot_and_cas_need_no_session_and_reject_ambiguous_requests() {
        let temporary = tempfile::tempdir().unwrap();
        let home = temporary.path().canonicalize().unwrap();
        let context = UserContext {
            home: home.clone(),
            uid: inputia_settings::maintenance::current_user_context()
                .unwrap()
                .uid,
        };
        let path = home.join("profile/settings.json");
        let call =
            |value: serde_json::Value| handle(&serde_json::to_vec(&value).unwrap(), &context);
        let read = call(json!({"action":"read","path":path}));
        assert_eq!(read["ok"], true);
        let snapshot: Snapshot = serde_json::from_value(read["snapshot"].clone()).unwrap();
        let request = json!({"operation_id":snapshot.operation_id(),"expected_store_id":snapshot.store_id,"expected_revision":snapshot.revision,"patch":{"memory_enabled":false}});
        let accepted = call(json!({"action":"apply","path":path,"request":request}));
        assert_eq!(accepted["result"]["status"], "saved");
        assert_eq!(
            accepted["result"]["current"]["values"]["memory_enabled"],
            false
        );
        assert_eq!(
            call(json!({"action":"read","path":path,"extra":true}))["ok"],
            false
        );
        let raw = unsafe { inputia_settings_request(std::ptr::null()) };
        let reply: serde_json::Value =
            serde_json::from_slice(unsafe { CStr::from_ptr(raw) }.to_bytes()).unwrap();
        unsafe { crate::inputia_string_free(raw) };
        assert_eq!(reply["code"], "invalid_request");
    }
}

/// 用经过协调器核对的精确版本创建 session；运行时资源路径不回写用户设置。
///
/// # Safety
/// request_json 满足只读 NUL 结尾 UTF-8 合同，Rime 调用须与其他 session 操作串行。
#[no_mangle]
pub unsafe extern "C" fn inputia_session_new_from_settings_snapshot(
    request_json: *const c_char,
) -> *mut super::InputiaSession {
    #[cfg(unix)]
    {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Request {
            path: std::path::PathBuf,
            store_id: String,
            revision: String,
            values_digest: String,
            rime_shared_data_dir: Option<std::path::PathBuf>,
            without_memory: bool,
        }
        let operation = || -> Option<*mut super::InputiaSession> {
            if request_json.is_null() {
                return None;
            }
            let raw = unsafe { CStr::from_ptr(request_json) }.to_bytes();
            let request: Request =
                serde_json::from_value(inputia_settings::store::strict_json(raw).ok()?).ok()?;
            let context = inputia_settings::maintenance::current_user_context().ok()?;
            let snapshot = {
                let store =
                    inputia_settings::store::Store::open(&request.path, &context.home, context.uid)
                        .ok()?;
                store.read().ok()?
            };
            if snapshot.store_id != request.store_id
                || snapshot.revision != request.revision
                || snapshot.values_digest != request.values_digest
            {
                return None;
            }
            let mut settings = snapshot.settings().ok()?;
            if let Some(shared) = request.rime_shared_data_dir {
                if !shared.is_absolute() {
                    return None;
                }
                settings.rime_shared_data_dir = Some(shared);
            }
            let mut options = super::session_options_from_settings(settings)?;
            if request.without_memory {
                options.memory_db_path = None;
            }
            // 不跨 Rime 初始化持有设置文件锁。回执只能声称这里实际读取的版本。
            inputia_settings::maintenance::ensure_current_normal_start().ok()?;
            let session = super::new_session_with_options_and_readiness(options, true);
            #[cfg(target_os = "macos")]
            if let Some(session) = unsafe { session.as_mut() } {
                session.settings_binding = Some(SessionBinding {
                    path: request.path,
                    entry: inputia_settings::store::application::ApplicationEntry::new(&snapshot),
                    spelling_degraded: snapshot.settings().ok()?.spelling_correction_enabled
                        && !super::effective_spelling_correction(
                            &snapshot.settings().ok()?.schema_id,
                            true,
                        ),
                    memory_degraded: request.without_memory
                        && snapshot.settings().ok()?.memory_enabled
                        && snapshot.settings().ok()?.privacy_learning_enabled,
                });
            }
            Some(session)
        };
        operation().unwrap_or(std::ptr::null_mut())
    }
    #[cfg(not(unix))]
    std::ptr::null_mut()
}

#[cfg(target_os = "macos")]
#[derive(Clone)]
pub(super) struct SessionBinding {
    path: std::path::PathBuf,
    entry: inputia_settings::store::application::ApplicationEntry,
    memory_degraded: bool,
    spelling_degraded: bool,
}
#[cfg(target_os = "macos")]
fn registry() -> &'static std::sync::Mutex<std::collections::BTreeMap<String, SessionBinding>> {
    static REGISTRY: std::sync::OnceLock<
        std::sync::Mutex<std::collections::BTreeMap<String, SessionBinding>>,
    > = std::sync::OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}
#[cfg(target_os = "macos")]
pub(super) fn retire(binding: &SessionBinding) {
    if let Ok(mut entries) = registry().lock() {
        entries.remove(&binding.entry.instance_id);
    }
}

/// Swift 真正更新原生缓存后确认字段；版本与 Rime 字段只来自这个存活 session 的内部绑定。
///
/// # Safety
/// session 必须仍存活且独占串行使用；native_json 为有效 NUL 结尾 UTF-8 字符串。
#[no_mangle]
pub unsafe extern "C" fn inputia_session_settings_applied(
    session: *mut super::InputiaSession,
    native_json: *const c_char,
) -> *mut c_char {
    #[cfg(target_os = "macos")]
    let result = (|| -> Option<serde_json::Value> {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Native {
            applied_fields: std::collections::BTreeSet<String>,
            unavailable_fields: std::collections::BTreeSet<String>,
        }
        let session = unsafe { session.as_mut() }?;
        let mut binding = session.settings_binding.clone()?;
        if native_json.is_null() {
            return None;
        }
        let native: Native = serde_json::from_value(
            inputia_settings::store::strict_json(unsafe { CStr::from_ptr(native_json) }.to_bytes())
                .ok()?,
        )
        .ok()?;
        let allowed = [
            "candidate_font_size",
            "input_mode_toggle_shortcut",
            "script_toggle_shortcut",
            "shift_toggle_enabled",
        ];
        if native
            .applied_fields
            .iter()
            .chain(&native.unavailable_fields)
            .any(|field| !allowed.contains(&field.as_str()))
            || native
                .applied_fields
                .intersection(&native.unavailable_fields)
                .next()
                .is_some()
        {
            return None;
        }
        let fields = [
            "schema_id",
            "candidate_page_size",
            "chinese_script",
            "punctuation_preference",
            "character_width_preference",
            "spelling_correction_enabled",
            "memory_enabled",
            "privacy_learning_enabled",
            "sensitive_bundle_ids",
        ];
        binding.entry.applied_fields = fields
            .into_iter()
            .map(String::from)
            .chain(native.applied_fields)
            .collect();
        binding.entry.unavailable_fields = native.unavailable_fields;
        if binding.spelling_degraded {
            binding
                .entry
                .applied_fields
                .remove("spelling_correction_enabled");
            binding
                .entry
                .unavailable_fields
                .insert("spelling_correction_enabled".into());
        }
        if binding.memory_degraded {
            for field in ["memory_enabled", "privacy_learning_enabled"] {
                binding.entry.applied_fields.remove(field);
                binding.entry.unavailable_fields.insert(field.into());
            }
        }
        let reply = serde_json::json!({"ok":true,"application":binding.entry});
        let mut entries = registry().lock().ok()?;
        if entries.len() >= 1024 && !entries.contains_key(&binding.entry.instance_id) {
            return None;
        }
        entries.insert(binding.entry.instance_id.clone(), binding);
        Some(reply)
    })();
    #[cfg(not(target_os = "macos"))]
    let result: Option<serde_json::Value> = None;
    super::string_json(
        &result.unwrap_or_else(|| serde_json::json!({"ok":false,"code":"application_unavailable"})),
    )
}

/// 在后台轮询里发布该路径实际存活的会话观察；不依赖 Rime 全局调用或主线程。
///
/// # Safety
/// path_json 为 NUL 结尾 UTF-8 JSON，形如 {"path":".../settings.json"}。
#[no_mangle]
pub unsafe extern "C" fn inputia_settings_flush_applications(
    path_json: *const c_char,
) -> *mut c_char {
    #[cfg(target_os = "macos")]
    let result = (|| -> inputia_settings::store::Result<()> {
        use inputia_settings::store::{self, Store};
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Request {
            path: std::path::PathBuf,
        }
        if path_json.is_null() {
            return Err(store::Error::InvalidRequest);
        }
        let request: Request = serde_json::from_value(store::strict_json(
            unsafe { CStr::from_ptr(path_json) }.to_bytes(),
        )?)
        .map_err(|_| store::Error::InvalidRequest)?;
        let context = inputia_settings::maintenance::current_user_context()
            .map_err(|_| store::Error::StorageUnavailable)?;
        let entries = registry()
            .lock()
            .map_err(|_| store::Error::StorageUnavailable)?
            .values()
            .filter(|entry| entry.path == request.path)
            .map(|entry| entry.entry.clone())
            .collect();
        Store::open(&request.path, &context.home, context.uid)?.publish_applications(entries)
    })();
    #[cfg(not(target_os = "macos"))]
    let result: Result<(), &'static str> = Err("unsupported_platform");
    super::string_json(&match result {
        Ok(()) => serde_json::json!({"ok":true}),
        Err(error) => serde_json::json!({"ok":false,"code":error}),
    })
}
