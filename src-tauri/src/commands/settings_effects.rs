//! 保存确认与原生应用是两个步骤；只有数据补偿与原生恢复均确认才称恢复完成。
use crate::settings::{self, AppSettings};
use tauri::AppHandle;

pub(crate) fn save_and_apply(
    app: &AppHandle,
    desired: AppSettings,
    undo_fields: impl FnOnce(&mut AppSettings),
    apply: impl FnOnce() -> Result<(), String>,
    restore_native: impl FnOnce() -> Result<(), String>,
    disable: impl FnOnce(),
) -> Result<(), String> {
    run(
        || {
            #[cfg(unix)]
            {
                settings::write_settings_with_snapshot(app, desired).map(Some)
            }
            #[cfg(not(unix))]
            {
                settings::write_settings(app, desired).map(|()| None::<AppSettings>)
            }
        },
        apply,
        |saved| {
            // Windows 旧存储没有原版本 CAS，不能用全表补偿覆盖并发设置。
            let mut saved = saved.ok_or_else(|| "settings_compensation_unavailable".to_owned())?;
            undo_fields(&mut saved);
            settings::write_settings(app, saved)
        },
        restore_native,
        disable,
    )
}
fn run<T>(
    save: impl FnOnce() -> Result<T, String>,
    apply: impl FnOnce() -> Result<(), String>,
    compensate: impl FnOnce(T) -> Result<(), String>,
    restore_native: impl FnOnce() -> Result<(), String>,
    disable: impl FnOnce(),
) -> Result<(), String> {
    let saved = save()?;
    if apply().is_ok() {
        return Ok(());
    }
    if compensate(saved).is_err() {
        disable();
        return Err("settings_saved_runtime_unconfirmed".into());
    }
    if restore_native().is_err() {
        disable();
        return Err("settings_restored_runtime_unconfirmed".into());
    }
    Err("settings_runtime_apply_failed_restored".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    #[test]
    fn failed_or_uncertain_save_never_applies_or_reissues() {
        for failure in ["settings_conflict", "settings_commit_uncertain"] {
            let events = RefCell::new(vec![]);
            assert_eq!(
                run::<()>(
                    || {
                        events.borrow_mut().push("save");
                        Err(failure.into())
                    },
                    || {
                        events.borrow_mut().push("apply");
                        Ok(())
                    },
                    |()| {
                        events.borrow_mut().push("compensate");
                        Ok(())
                    },
                    || {
                        events.borrow_mut().push("restore");
                        Ok(())
                    },
                    || {
                        events.borrow_mut().push("disable");
                    }
                )
                .unwrap_err(),
                failure
            );
            assert_eq!(*events.borrow(), ["save"]);
        }
    }
    #[test]
    fn conflict_during_exact_compensation_disables_without_claiming_native_restore() {
        let events = RefCell::new(vec![]);
        let error = run(
            || Ok(7),
            || {
                events.borrow_mut().push("apply");
                Err("native".into())
            },
            |ticket| {
                assert_eq!(ticket, 7);
                events.borrow_mut().push("compensate");
                Err("conflict".into())
            },
            || {
                events.borrow_mut().push("restore");
                Ok(())
            },
            || events.borrow_mut().push("disable"),
        )
        .unwrap_err();
        assert_eq!(error, "settings_saved_runtime_unconfirmed");
        assert_eq!(*events.borrow(), ["apply", "compensate", "disable"]);
    }
    #[test]
    fn restored_data_does_not_claim_restored_native_component() {
        assert_eq!(
            run(
                || Ok(()),
                || Err("native".into()),
                |()| Ok(()),
                || Err("restore".into()),
                || {}
            )
            .unwrap_err(),
            "settings_restored_runtime_unconfirmed"
        );
    }
    #[test]
    fn confirmed_restore_still_reports_original_action_failure() {
        assert_eq!(
            run(
                || Ok(()),
                || Err("native".into()),
                |()| Ok(()),
                || Ok(()),
                || panic!("should not disable")
            )
            .unwrap_err(),
            "settings_runtime_apply_failed_restored"
        );
    }
}
