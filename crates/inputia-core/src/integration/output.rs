//! 输出的单次派发合同。状态须先持久化，再调用平台适配器；未知回执不可重派。

use super::events::Identifier;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputAction {
    InsertText,
    PasteAsset,
    Copy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputOwner {
    InputMethod,
    Platform,
}

/// 由平台持有并再次核实的目标，不以 App bundle ID 代替控件身份。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetToken {
    pub host_instance_id: Identifier,
    pub controller_id: Identifier,
    pub target_pid: u32,
    pub process_start_id: Identifier,
    pub field_id: Identifier,
    pub focus_generation: u64,
    pub edit_generation: u64,
    pub expires_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DispatchContext {
    pub now_ms: u64,
    pub policy_epoch: u64,
    pub target: Option<TargetToken>,
    pub composition_active: bool,
    pub output_allowed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputState {
    Prepared,
    PendingTarget,
    Dispatched,
    Confirmed,
    FailedBeforeDispatch,
    Uncertain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DispatchRejection {
    AlreadyAttempted,
    InvalidOwner,
    StalePolicy,
    Forbidden,
    MissingTarget,
    ExpiredTarget,
    ChangedTarget,
    CompositionActive,
}

/// 平台回执必须说明实际证明了哪一层，IMK 调用返回不等于应用确认。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputReceipt {
    DispatchedOnly,
    ConfirmedByTarget,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputOperation {
    pub operation_id: Identifier,
    action: OutputAction,
    owner: OutputOwner,
    state: OutputState,
    policy_epoch: u64,
    target: Option<TargetToken>,
}

impl OutputOperation {
    pub fn new(
        operation_id: Identifier,
        action: OutputAction,
        owner: OutputOwner,
        policy_epoch: u64,
        target: Option<TargetToken>,
    ) -> Result<Self, DispatchRejection> {
        if owner == OutputOwner::InputMethod && action != OutputAction::InsertText {
            return Err(DispatchRejection::InvalidOwner);
        }
        Ok(Self {
            operation_id,
            action,
            owner,
            state: OutputState::Prepared,
            policy_epoch,
            target,
        })
    }

    pub fn state(&self) -> OutputState {
        self.state
    }

    pub fn owner(&self) -> OutputOwner {
        self.owner
    }

    pub fn action(&self) -> OutputAction {
        self.action
    }

    /// 在持久事务中调用；返回成功后落盘 Dispatched，再执行外部副作用。
    pub fn authorize_dispatch(
        &mut self,
        context: &DispatchContext,
    ) -> Result<(), DispatchRejection> {
        if self.state != OutputState::Prepared {
            return Err(DispatchRejection::AlreadyAttempted);
        }
        let rejected = if self.policy_epoch != context.policy_epoch {
            Some(DispatchRejection::StalePolicy)
        } else if !context.output_allowed {
            Some(DispatchRejection::Forbidden)
        } else if self.action == OutputAction::Copy {
            None
        } else {
            self.check_target(context).err()
        };
        if let Some(reason) = rejected {
            self.state = OutputState::PendingTarget;
            return Err(reason);
        }
        self.state = OutputState::Dispatched;
        Ok(())
    }

    fn check_target(&self, context: &DispatchContext) -> Result<(), DispatchRejection> {
        let expected = self
            .target
            .as_ref()
            .ok_or(DispatchRejection::MissingTarget)?;
        let actual = context
            .target
            .as_ref()
            .ok_or(DispatchRejection::MissingTarget)?;
        if expected.expires_at_ms <= context.now_ms || actual.expires_at_ms <= context.now_ms {
            return Err(DispatchRejection::ExpiredTarget);
        }
        if expected != actual {
            return Err(DispatchRejection::ChangedTarget);
        }
        if context.composition_active {
            return Err(DispatchRejection::CompositionActive);
        }
        Ok(())
    }

    /// 只有适配器明确报告尚未执行时才能选择另一条路线。
    pub fn reject_before_dispatch(&mut self) -> Result<(), DispatchRejection> {
        if self.state != OutputState::Prepared {
            return Err(DispatchRejection::AlreadyAttempted);
        }
        self.state = OutputState::FailedBeforeDispatch;
        Ok(())
    }

    pub fn select_fallback(&mut self, owner: OutputOwner) -> Result<(), DispatchRejection> {
        if self.state != OutputState::FailedBeforeDispatch {
            return Err(DispatchRejection::AlreadyAttempted);
        }
        if owner == OutputOwner::InputMethod && self.action != OutputAction::InsertText {
            return Err(DispatchRejection::InvalidOwner);
        }
        self.owner = owner;
        self.state = OutputState::Prepared;
        Ok(())
    }

    pub fn receive(&mut self, receipt: OutputReceipt) -> Result<(), DispatchRejection> {
        if self.state != OutputState::Dispatched {
            return Err(DispatchRejection::AlreadyAttempted);
        }
        self.state = match receipt {
            OutputReceipt::DispatchedOnly => OutputState::Dispatched,
            OutputReceipt::ConfirmedByTarget => OutputState::Confirmed,
            OutputReceipt::Unknown => OutputState::Uncertain,
        };
        Ok(())
    }

    /// 恢复时已经派发的操作不能假定失败并自动重新尝试。
    pub fn recover_after_disconnect(&mut self) {
        if self.state == OutputState::Dispatched {
            self.state = OutputState::Uncertain;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(value: &str) -> Identifier {
        Identifier::parse(value).unwrap()
    }

    fn target() -> TargetToken {
        TargetToken {
            host_instance_id: id("host-1"),
            controller_id: id("controller-1"),
            target_pid: 42,
            process_start_id: id("process-start-1"),
            field_id: id("field-1"),
            focus_generation: 10,
            edit_generation: 20,
            expires_at_ms: 5_000,
        }
    }

    fn operation() -> OutputOperation {
        OutputOperation::new(
            id("op-1"),
            OutputAction::InsertText,
            OutputOwner::InputMethod,
            7,
            Some(target()),
        )
        .unwrap()
    }

    fn context() -> DispatchContext {
        DispatchContext {
            now_ms: 100,
            policy_epoch: 7,
            target: Some(target()),
            composition_active: false,
            output_allowed: true,
        }
    }

    #[test]
    fn a_lost_receipt_never_selects_a_second_output_owner() {
        for _ in 0..100 {
            let mut op = operation();
            op.authorize_dispatch(&context()).unwrap();
            op.receive(OutputReceipt::Unknown).unwrap();
            assert_eq!(op.state(), OutputState::Uncertain);
            assert!(op.select_fallback(OutputOwner::Platform).is_err());
            assert!(op.authorize_dispatch(&context()).is_err());
        }
    }

    #[test]
    fn disconnect_after_dispatch_is_not_safe_to_retry() {
        let mut op = operation();
        op.authorize_dispatch(&context()).unwrap();
        op.recover_after_disconnect();
        assert_eq!(op.state(), OutputState::Uncertain);
        assert!(op.reject_before_dispatch().is_err());
    }

    #[test]
    fn explicit_rejection_before_side_effect_allows_fallback() {
        let mut op = operation();
        op.reject_before_dispatch().unwrap();
        op.select_fallback(OutputOwner::Platform).unwrap();
        op.authorize_dispatch(&context()).unwrap();
        assert_eq!(op.owner(), OutputOwner::Platform);
        assert!(op.select_fallback(OutputOwner::InputMethod).is_err());
    }

    #[test]
    fn same_app_different_field_edit_or_process_cannot_receive_old_text() {
        let mut variants = Vec::new();
        let mut changed = target();
        changed.field_id = id("field-2");
        variants.push(changed);
        let mut changed = target();
        changed.edit_generation += 1;
        variants.push(changed);
        let mut changed = target();
        changed.process_start_id = id("reused-pid");
        variants.push(changed);
        let mut changed = target();
        changed.focus_generation += 1;
        variants.push(changed);
        let mut changed = target();
        changed.host_instance_id = id("restarted-host");
        variants.push(changed);
        for changed in variants {
            let mut op = operation();
            assert_eq!(
                op.authorize_dispatch(&DispatchContext {
                    target: Some(changed),
                    ..context()
                }),
                Err(DispatchRejection::ChangedTarget)
            );
            assert_eq!(op.state(), OutputState::PendingTarget);
        }
    }

    #[test]
    fn composition_expiry_privacy_and_missing_target_remain_pending() {
        let cases = [
            (
                DispatchContext {
                    composition_active: true,
                    ..context()
                },
                DispatchRejection::CompositionActive,
            ),
            (
                DispatchContext {
                    now_ms: 5_000,
                    ..context()
                },
                DispatchRejection::ExpiredTarget,
            ),
            (
                DispatchContext {
                    policy_epoch: 8,
                    ..context()
                },
                DispatchRejection::StalePolicy,
            ),
            (
                DispatchContext {
                    output_allowed: false,
                    ..context()
                },
                DispatchRejection::Forbidden,
            ),
            (
                DispatchContext {
                    target: None,
                    ..context()
                },
                DispatchRejection::MissingTarget,
            ),
        ];
        for (ctx, expected) in cases {
            let mut op = operation();
            assert_eq!(op.authorize_dispatch(&ctx), Err(expected));
            assert_eq!(op.state(), OutputState::PendingTarget);
        }
    }

    #[test]
    fn dispatched_does_not_pretend_target_confirmed() {
        let mut op = operation();
        op.authorize_dispatch(&context()).unwrap();
        op.receive(OutputReceipt::DispatchedOnly).unwrap();
        assert_eq!(op.state(), OutputState::Dispatched);
        op.receive(OutputReceipt::ConfirmedByTarget).unwrap();
        assert_eq!(op.state(), OutputState::Confirmed);
        assert!(op.authorize_dispatch(&context()).is_err());
    }

    #[test]
    fn copy_is_separate_and_assets_cannot_use_text_ime_adapter() {
        assert!(OutputOperation::new(
            id("asset-1"),
            OutputAction::PasteAsset,
            OutputOwner::InputMethod,
            7,
            Some(target())
        )
        .is_err());
        let mut copy = OutputOperation::new(
            id("copy-1"),
            OutputAction::Copy,
            OutputOwner::Platform,
            7,
            None,
        )
        .unwrap();
        copy.authorize_dispatch(&DispatchContext {
            target: None,
            ..context()
        })
        .unwrap();
        assert_eq!(copy.state(), OutputState::Dispatched);
    }
}
