//! 差量修改真实注册，恢复只反转本次已收到成功回执的步骤。
use super::{handy_keys, tauri_impl};
use crate::settings::{AppSettings, KeyboardImplementation, ShortcutBinding};
use std::cell::{Cell, RefCell};
use tauri::AppHandle;

#[derive(Clone)]
struct Step {
    register: bool,
    binding: ShortcutBinding,
}

pub(crate) struct RegistrationDelta {
    backend: KeyboardImplementation,
    steps: Vec<Step>,
    applied: RefCell<Vec<Step>>,
    uncertain: Cell<bool>,
}
impl RegistrationDelta {
    pub(crate) fn suspend(before: &AppSettings) -> Self {
        let mut bindings = crate::settings::get_default_settings().bindings;
        bindings.extend(before.bindings.clone());
        let mut steps = bindings
            .into_iter()
            .filter(|(id, _)| super::binding_enabled(before, id))
            .map(|(_, binding)| Step {
                register: false,
                binding,
            })
            .collect::<Vec<_>>();
        steps.sort_by(|a, b| a.binding.id.cmp(&b.binding.id));
        Self {
            backend: before.keyboard_implementation,
            steps,
            applied: RefCell::new(vec![]),
            uncertain: Cell::new(false),
        }
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    pub(crate) fn new(before: &AppSettings, after: &AppSettings) -> Self {
        let active = |settings: &AppSettings| {
            let mut bindings = crate::settings::get_default_settings().bindings;
            bindings.extend(settings.bindings.clone());
            bindings.retain(|id, _| super::binding_enabled(settings, id));
            bindings
        };
        let old = active(before);
        let new = active(after);
        let same = |a: &ShortcutBinding, b: &ShortcutBinding| {
            a.id == b.id && a.current_binding == b.current_binding
        };
        let mut remove = old
            .values()
            .filter(|a| new.get(&a.id).is_none_or(|b| !same(a, b)))
            .cloned()
            .collect::<Vec<_>>();
        let mut add = new
            .values()
            .filter(|b| old.get(&b.id).is_none_or(|a| !same(a, b)))
            .cloned()
            .collect::<Vec<_>>();
        remove.sort_by(|a, b| a.id.cmp(&b.id));
        add.sort_by(|a, b| a.id.cmp(&b.id));
        Self {
            backend: before.keyboard_implementation,
            steps: remove
                .into_iter()
                .map(|binding| Step {
                    register: false,
                    binding,
                })
                .chain(add.into_iter().map(|binding| Step {
                    register: true,
                    binding,
                }))
                .collect(),
            applied: RefCell::new(vec![]),
            uncertain: Cell::new(false),
        }
    }
    fn native(&self, app: &AppHandle, step: &Step) -> Result<(), String> {
        match (self.backend, step.register) {
            (KeyboardImplementation::HandyKeys, true) => {
                handy_keys::register_shortcut(app, step.binding.clone())
            }
            (KeyboardImplementation::HandyKeys, false) => {
                handy_keys::unregister_shortcut(app, step.binding.clone())
            }
            (KeyboardImplementation::Tauri, true) => {
                tauri_impl::register_shortcut(app, step.binding.clone())
            }
            (KeyboardImplementation::Tauri, false) => {
                tauri_impl::unregister_shortcut(app, step.binding.clone())
            }
        }
    }
    pub(crate) fn apply(&self, app: &AppHandle) -> Result<(), String> {
        self.apply_with(|step| self.native(app, step))?;
        crate::secure_input::reconcile_fallback_checked(app)
    }
    pub(crate) fn restore(&self, app: &AppHandle) -> Result<(), String> {
        self.restore_with(|step| self.native(app, step))?;
        crate::secure_input::reconcile_fallback_checked(app)
    }
    fn apply_with(
        &self,
        mut native: impl FnMut(&Step) -> Result<(), String>,
    ) -> Result<(), String> {
        for step in &self.steps {
            if let Err(error) = native(step) {
                self.uncertain.set(true);
                return Err(error);
            }
            self.applied.borrow_mut().push(step.clone());
        }
        Ok(())
    }
    fn restore_with(
        &self,
        mut native: impl FnMut(&Step) -> Result<(), String>,
    ) -> Result<(), String> {
        // Err 可能发生在原生操作已完成但回执丢失之后；不能猜测注册清单。
        if self.uncertain.get() {
            return Err("shortcut_native_state_unconfirmed".into());
        }
        let mut applied = self.applied.borrow_mut();
        while let Some(step) = applied.last() {
            native(&Step {
                register: !step.register,
                binding: step.binding.clone(),
            })?;
            applied.pop();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_changed_registration_is_retired_and_restored_in_reverse_order() {
        let before = crate::settings::get_default_settings();
        let mut after = before.clone();
        after
            .bindings
            .get_mut("transcribe")
            .unwrap()
            .current_binding = "F8".into();
        let delta = RegistrationDelta::new(&before, &after);
        let observed = RefCell::new(vec![]);
        delta
            .apply_with(|step| {
                observed
                    .borrow_mut()
                    .push((step.register, step.binding.id.clone()));
                Ok(())
            })
            .unwrap();
        delta
            .restore_with(|step| {
                observed
                    .borrow_mut()
                    .push((step.register, step.binding.id.clone()));
                Ok(())
            })
            .unwrap();
        assert_eq!(
            *observed.borrow(),
            vec![
                (false, "transcribe".into()),
                (true, "transcribe".into()),
                (false, "transcribe".into()),
                (true, "transcribe".into())
            ]
        );
    }
    #[test]
    fn capture_retires_only_enabled_primary_bindings_and_restores_exact_set_once() {
        let mut before = crate::settings::get_default_settings();
        before.clipboard_hotkey_enabled = false;
        let delta = RegistrationDelta::suspend(&before);
        let removed = RefCell::new(vec![]);
        delta
            .apply_with(|step| {
                assert!(!step.register);
                assert_ne!(step.binding.id, "cancel");
                assert_ne!(step.binding.id, "clipboard_history");
                removed.borrow_mut().push(step.binding.id.clone());
                Ok(())
            })
            .unwrap();
        assert!(!removed.borrow().is_empty());
        let restored = RefCell::new(vec![]);
        delta
            .restore_with(|step| {
                assert!(step.register);
                restored.borrow_mut().push(step.binding.id.clone());
                Ok(())
            })
            .unwrap();
        assert_eq!(
            *restored.borrow(),
            removed.borrow().iter().rev().cloned().collect::<Vec<_>>()
        );
        delta
            .restore_with(|_| panic!("already restored entries cannot be registered twice"))
            .unwrap();
    }
    #[test]
    fn lost_native_receipt_cannot_be_claimed_restored() {
        let before = crate::settings::get_default_settings();
        let mut after = before.clone();
        after
            .bindings
            .get_mut("transcribe")
            .unwrap()
            .current_binding = "F8".into();
        let delta = RegistrationDelta::new(&before, &after);
        assert!(delta.apply_with(|_| Err("unknown".into())).is_err());
        assert!(delta
            .restore_with(|_| panic!("must disable rather than guess"))
            .is_err());
    }
}
