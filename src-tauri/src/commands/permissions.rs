//! 只导航到系统权限页面或应用包，不申请权限、不启动输入法、不修改安装。

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

fn find_enclosing_app_bundle(start: &Path) -> Option<PathBuf> {
    let mut cursor = Some(start.to_path_buf());
    while let Some(dir) = cursor {
        if dir.extension().and_then(|ext| ext.to_str()) == Some("app")
            && dir.join("Contents/Info.plist").is_file()
        {
            return Some(dir);
        }
        cursor = dir.parent().map(Path::to_path_buf);
    }
    None
}

/// 解析当前主程序 `.app` 包路径，供在 Finder / 辅助功能列表中拖放。
pub(crate) fn main_app_bundle_path() -> Result<PathBuf, String> {
    #[cfg(target_os = "macos")]
    {
        use objc2_foundation::NSBundle;
        let path = PathBuf::from(NSBundle::mainBundle().bundlePath().to_string());
        if let Some(bundle) = find_enclosing_app_bundle(&path) {
            return Ok(bundle);
        }
        // 开发态可能是裸可执行文件：沿可执行路径向上寻找 `.app`。
        let exe = std::env::current_exe().map_err(|_| "exe_unavailable")?;
        if let Some(bundle) = exe.parent().and_then(find_enclosing_app_bundle) {
            return Ok(bundle);
        }
        Err("bundle_unavailable".into())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err("unsupported_platform".into())
    }
}

fn reveal_path(app: &AppHandle, path: PathBuf) -> Result<(), String> {
    app.opener()
        .reveal_item_in_dir(path)
        .map_err(|_| "reveal_failed".into())
}

fn open_accessibility_settings(app: &AppHandle) -> Result<(), String> {
    app.opener()
        .open_url(
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
            None::<String>,
        )
        .map_err(|_| "settings_unavailable".into())
}

#[cfg(target_os = "macos")]
fn begin_bundle_drag_on_main_thread(bundle: PathBuf) -> Result<(), String> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSView};
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

    let mtm = MainThreadMarker::new().ok_or("main_thread_required")?;
    let ns_app = NSApplication::sharedApplication(mtm);
    let event = ns_app.currentEvent().ok_or("drag_event_missing")?;
    let window = ns_app
        .keyWindow()
        .or_else(|| ns_app.mainWindow())
        .ok_or("window_unavailable")?;
    let view = window.contentView().ok_or("view_unavailable")?;
    let filename = NSString::from_str(&bundle.to_string_lossy());
    let rect = NSRect {
        origin: NSPoint::new(0.0, 0.0),
        size: NSSize::new(0.0, 0.0),
    };
    #[allow(deprecated)]
    let started = NSView::dragFile_fromRect_slideBack_event(&view, &filename, rect, true, &event);
    if started {
        Ok(())
    } else {
        Err("drag_rejected".into())
    }
}

#[tauri::command]
#[specta::specta]
pub fn open_inputia_permission_help(app: AppHandle, action: String) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("unsupported_platform".into());
    }
    match action.as_str() {
        "settings" => open_accessibility_settings(&app),
        "app" => reveal_path(&app, main_app_bundle_path()?),
        "assist" => {
            let bundle = main_app_bundle_path()?;
            open_accessibility_settings(&app)?;
            reveal_path(&app, bundle)
        }
        "component" => {
            let home = app.path().home_dir().map_err(|_| "home_unavailable")?;
            let candidate = app.config().identifier == "com.pais.handy.UnifiedCandidate";
            let component = input_method_locations(&home, candidate)
                .into_iter()
                .find(|path| path.join("Contents/Info.plist").is_file())
                .ok_or("component_missing")?;
            reveal_path(&app, component)
        }
        _ => Err("unsupported_action".into()),
    }
}

/// 从当前按住的指针事件开始，把主程序 `.app` 拖进系统「辅助功能」列表。
#[tauri::command]
#[specta::specta]
pub fn begin_inputia_app_bundle_drag(app: AppHandle) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("unsupported_platform".into());
    }
    let bundle = main_app_bundle_path()?;
    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let result = begin_bundle_drag_on_main_thread(bundle);
        let _ = tx.send(result);
    })
    .map_err(|_| "main_thread_dispatch_failed".to_string())?;
    rx.recv()
        .map_err(|_| "main_thread_dispatch_failed".to_string())?
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

    #[test]
    fn enclosing_app_bundle_resolves_from_nested_executable_dir() {
        let temp = tempfile::tempdir().expect("tempdir");
        let app_dir = temp.path().join("Inputia.app");
        let macos_dir = app_dir.join("Contents/MacOS");
        std::fs::create_dir_all(&macos_dir).expect("create MacOS dir");
        std::fs::write(app_dir.join("Contents/Info.plist"), b"<plist/>").expect("write plist");

        assert_eq!(find_enclosing_app_bundle(&macos_dir), Some(app_dir.clone()));
        assert_eq!(find_enclosing_app_bundle(&app_dir), Some(app_dir));
        assert_eq!(find_enclosing_app_bundle(temp.path()), None);
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
