//! 主应用基础输入设置协调入口。路径由已核实的安装域决定，前端不能指定文件。
use serde::Deserialize;
use serde_json::{json, Value};

#[cfg(unix)]
use inputia_settings::store::{ImportRequest, PatchRequest, Store};

#[cfg(unix)]
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Read {},
    Apply { request: PatchRequest },
    InspectExternal {},
    ImportExternal { request: ImportRequest },
    ApplicationStatus {},
}

#[tauri::command]
pub async fn input_settings_request(request: Value) -> Value {
    #[cfg(target_os = "macos")]
    {
        let result = tauri::async_runtime::spawn_blocking(move || {
            let context = inputia_settings::maintenance::current_user_context()
                .map_err(|_| inputia_settings::store::Error::StorageUnavailable)?;
            let root = crate::candidate_profile::current()
                .map(|profile| profile.inputia_root.clone())
                .unwrap_or_else(|| context.home.join("Library/Application Support/Inputia"));
            handle(request, &root.join("settings.json"), &context)
        })
        .await;
        match result {
            Ok(Ok(reply)) => reply,
            Ok(Err(error)) => json!({"ok":false,"code":error}),
            Err(_) => json!({"ok":false,"code":"storage_unavailable"}),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = request;
        json!({"ok":false,"code":"unsupported_platform"})
    }
}

#[cfg(unix)]
fn handle(
    request: Value,
    path: &std::path::Path,
    context: &inputia_settings::maintenance::UserContext,
) -> inputia_settings::store::Result<Value> {
    use inputia_settings::store::Error;
    if serde_json::to_vec(&request)
        .map_err(|_| Error::InvalidRequest)?
        .len()
        > 512 * 1024
    {
        return Err(Error::InvalidRequest);
    }
    let request: Request = serde_json::from_value(request).map_err(|_| Error::InvalidRequest)?;
    let store = Store::open(path, &context.home, context.uid)?;
    Ok(match request {
        Request::Read {} => json!({"ok":true,"snapshot":store.read()?}),
        Request::Apply { request } => json!({"ok":true,"result":store.apply(&request)?}),
        Request::InspectExternal {} => json!({"ok":true,"external":store.inspect_external()?}),
        Request::ImportExternal { request } => {
            json!({"ok":true,"result":store.import_external(&request)?})
        }
        Request::ApplicationStatus {} => {
            #[cfg(target_os = "macos")]
            {
                json!({"ok":true,"application":store.application_status()?})
            }
            #[cfg(not(target_os = "macos"))]
            {
                json!({"ok":false,"code":"unsupported_platform"})
            }
        }
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn callers_cannot_choose_a_path_and_conflicts_preserve_the_committed_value() {
        let temporary = tempfile::tempdir().unwrap();
        let context = inputia_settings::maintenance::UserContext {
            home: temporary.path().canonicalize().unwrap(),
            uid: inputia_settings::maintenance::current_user_context()
                .unwrap()
                .uid,
        };
        let path = context.home.join("Inputia/settings.json");
        assert!(handle(
            json!({"action":"read","path":"/wrong/settings.json"}),
            &path,
            &context
        )
        .is_err());
        let first = handle(json!({"action":"read"}), &path, &context).unwrap();
        let snapshot: inputia_settings::store::Snapshot =
            serde_json::from_value(first["snapshot"].clone()).unwrap();
        let request = |value| {
            json!({"action":"apply","request":{
            "operation_id":snapshot.operation_id(),"expected_store_id":snapshot.store_id,"expected_revision":snapshot.revision,
            "patch":{"memory_enabled":value}}})
        };
        assert_eq!(
            handle(request(false), &path, &context).unwrap()["result"]["status"],
            "saved"
        );
        let conflict = handle(request(true), &path, &context).unwrap();
        assert_eq!(conflict["result"]["status"], "conflict");
        assert_eq!(
            conflict["result"]["current"]["values"]["memory_enabled"],
            false
        );
    }
}
