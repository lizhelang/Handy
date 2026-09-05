//! 排队任务过期后不能迟到执行；已开始任务的回执超时仍保持不确定。

use std::sync::{
    atomic::{AtomicU8, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

const PENDING: u8 = 0;
const STARTED: u8 = 1;
const CANCELLED: u8 = 2;

/// 平台目标查询可等待；返回时必须再次核验短期许可，不能只验证查询前状态。
pub(crate) fn validate_output_boundary(
    check_permission: impl Fn() -> Result<(), String>,
    check_target: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    check_permission()?;
    check_target()?;
    check_permission()
}

#[derive(Clone)]
pub(crate) struct DispatchGate {
    state: Arc<AtomicU8>,
    deadline: Instant,
}

impl DispatchGate {
    pub(crate) fn new(timeout: Duration) -> Self {
        Self {
            state: Arc::new(AtomicU8::new(PENDING)),
            deadline: Instant::now() + timeout,
        }
    }

    pub(crate) fn start(&self) -> bool {
        if Instant::now() >= self.deadline {
            self.cancel_pending();
            return false;
        }
        self.state
            .compare_exchange(PENDING, STARTED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    /// true 表示任务确定没有开始；false 表示不能宣称没有副作用。
    pub(crate) fn cancel_pending(&self) -> bool {
        match self
            .state
            .compare_exchange(PENDING, CANCELLED, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) | Err(CANCELLED) => true,
            Err(_) => false,
        }
    }

    /// 慢准备阶段结束后再次检查，过期任务不得开始新的副作用。
    pub(crate) fn check(&self) -> Result<(), String> {
        if self.state.load(Ordering::Acquire) == STARTED && Instant::now() < self.deadline {
            Ok(())
        } else {
            Err("output dispatch deadline expired".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revocation_during_target_query_prevents_output() {
        let revoked = std::cell::Cell::new(false);
        let result = validate_output_boundary(
            || {
                if revoked.get() {
                    Err("revoked".into())
                } else {
                    Ok(())
                }
            },
            || {
                revoked.set(true);
                Ok(())
            },
        );
        assert!(result.is_err());
    }

    #[test]
    fn timed_out_queued_task_cannot_run_later() {
        let gate = DispatchGate::new(Duration::from_secs(1));
        let queued = gate.clone();
        assert!(gate.cancel_pending());
        assert!(!queued.start());
        assert!(queued.check().is_err());
    }

    #[test]
    fn already_started_timeout_is_not_claimed_as_cancelled() {
        let gate = DispatchGate::new(Duration::from_secs(1));
        assert!(gate.start());
        assert!(!gate.cancel_pending());
        assert!(!gate.start());
    }

    #[test]
    fn preparation_expiry_prevents_later_side_effect() {
        let mut gate = DispatchGate::new(Duration::from_secs(1));
        assert!(gate.start());
        gate.deadline = Instant::now();
        assert!(gate.check().is_err());
    }

    #[test]
    fn expired_task_never_starts() {
        let gate = DispatchGate::new(Duration::ZERO);
        assert!(!gate.start());
        assert!(gate.cancel_pending());
    }
}
