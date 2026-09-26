//! 只导航到系统权限页面或输入法包，不申请权限、不启动输入法、不修改安装。

use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;

fn input_method_locations(home: &Path, candidate: bool) -> Vec<PathBuf> {
    if candidate {
        // 候选缺失不能回退到用户的日常输入法。
        vec![home.join("Library/Input Methods/InputiaUnifiedCandidate.app")]
    } else {
        vec![
            home.join("Library/Input Methods/InputiaInputMethod.app"),
            PathBuf::from("/Library/Input Methods/InputiaInputMethod.app"),
        ]
    }
}

#[tauri::command]
#[specta::specta]
pub fn open_inputia_permission_help(app: AppHandle, action: String) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("unsupported_platform".into());
    }
    match action.as_str() {
        "settings" => app
            .opener()
            .open_url(
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
                None::<String>,
            )
            .map_err(|_| "settings_unavailable".into()),
        "component" => {
            let home = app.path().home_dir().map_err(|_| "home_unavailable")?;
            let candidate = app.config().identifier == "com.pais.handy.UnifiedCandidate";
            let component = input_method_locations(&home, candidate)
                .into_iter()
                .find(|path| path.join("Contents/Info.plist").is_file())
                .ok_or("component_missing")?;
            app.opener()
                .reveal_item_in_dir(component)
                .map_err(|_| "reveal_failed".into())
        }
        _ => Err("unsupported_action".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_permission_help_never_targets_daily_installation() {
        assert_eq!(
            input_method_locations(Path::new("/test/user"), true),
            vec![PathBuf::from(
                "/test/user/Library/Input Methods/InputiaUnifiedCandidate.app"
            )]
        );
    }

    #[test]
    fn daily_permission_help_never_targets_candidate_installation() {
        let paths = input_method_locations(Path::new("/test/user"), false);
        assert_eq!(paths.len(), 2);
        assert!(paths
            .iter()
            .all(|path| path.ends_with("InputiaInputMethod.app")));
    }
}

#[tauri::command]
pub async fn inputia_permission_status(app: AppHandle) -> serde_json::Value {
    crate::input_permission::snapshot(&app)
}
#[tauri::command]
pub async fn inputia_permission_recheck(app: AppHandle) -> serde_json::Value {
    crate::input_permission::request_recheck(&app);
    crate::input_permission::snapshot(&app)
}
#[tauri::command]
pub async fn inputia_permission_prepare_maintenance(
    app: AppHandle,
) -> Result<serde_json::Value, String> {
    crate::input_permission::prepare_maintenance(&app)?;
    Ok(crate::input_permission::snapshot(&app))
}
#[tauri::command]
pub async fn inputia_permission_resume(app: AppHandle) -> Result<serde_json::Value, String> {
    crate::input_permission::resume(&app)?;
    Ok(crate::input_permission::snapshot(&app))
}
