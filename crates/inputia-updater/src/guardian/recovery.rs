//! STOP 与恢复在同一执行器串行；观察线程不能直接发送 CONT。
use super::{
    protocol::{Entry, Ledger, Phase, ProtocolError, Resolution},
    GuardianError,
};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PeerState {
    Alive,
    Exited,
    Exec,
    Unknown,
}
impl PeerState {
    pub fn owner_gone(self) -> bool {
        matches!(self, Self::Exited | Self::Exec)
    }
}
pub(super) trait Peer {
    fn state(&self) -> Result<PeerState, GuardianError>;
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ExecutionState {
    Running,
    Stopped,
    OriginalExited,
}
pub(super) trait Effects {
    fn entries(&self) -> &[Entry];
    fn stop(
        &mut self,
        index: u32,
        authorize: &mut dyn FnMut() -> bool,
    ) -> Result<(), GuardianError>;
    fn close_stop(&mut self) -> Result<(), GuardianError>;
    fn resume(&mut self, index: u32) -> Result<Resolution, GuardianError>;
    fn state(&self, index: u32) -> Result<ExecutionState, GuardianError>;
    fn assert_holding(&self) -> Result<(), GuardianError>;
}
pub(super) struct Executor<E: Effects> {
    pub ledger: Ledger,
    pub effects: E,
}
impl<E: Effects> Executor<E> {
    pub fn stop(
        &mut self,
        index: u32,
        cancelled: &AtomicBool,
        authorize: &mut dyn FnMut() -> bool,
    ) -> Result<(), GuardianError> {
        if cancelled.load(Ordering::Acquire) {
            self.begin_recovery()?;
            return Err(GuardianError::Cancelled);
        }
        let Some(_) = self.ledger.begin_stop(index)? else {
            return Ok(());
        };
        // cancel可能在最后授权后到来，但没有并行恢复者；即使syscall已开始，也先回到本执行器再恢复。
        let result = self.effects.stop(index, &mut || {
            !cancelled.load(Ordering::Acquire) && authorize()
        });
        if result.is_err() || cancelled.load(Ordering::Acquire) {
            self.begin_recovery()?;
            return result.and(Err(GuardianError::Cancelled));
        }
        self.ledger.held(index)?;
        Ok(())
    }
    pub fn begin_recovery(&mut self) -> Result<(), GuardianError> {
        self.ledger.recover(); // 先不可逆关闭协议效应，再关原生handle。
        self.effects.close_stop()
    }
    pub fn recover_once(&mut self) -> Result<Vec<(u32, Resolution)>, GuardianError> {
        self.begin_recovery()?;
        let mut resolved = Vec::new();
        for entry in self.ledger.recovery_entries()? {
            if let Ok(result) = self.effects.resume(entry.index) {
                self.ledger.resolve(entry.index, result)?;
                resolved.push((entry.index, result));
            }
        }
        if self.ledger.recovery_entries()?.is_empty() {
            self.ledger.resumed()?;
        }
        Ok(resolved)
    }
    /// 父只有在旧owner精确EXIT/EXEC后接管；EOF/timeout本身不能授权恢复。
    #[cfg(test)]
    pub fn take_over(&mut self, peer: &impl Peer) -> Result<Vec<(u32, Resolution)>, GuardianError> {
        if !peer.state()?.owner_gone() {
            return Err(GuardianError::PeerStillAlive);
        }
        self.recover_once()
    }
    pub fn holding(&mut self) -> Result<(), GuardianError> {
        if !matches!(
            self.ledger.phase(),
            Phase::Ready | Phase::Arming | Phase::Holding
        ) {
            return Err(ProtocolError::State.into());
        }
        self.effects.assert_holding()?;
        self.ledger.holding()?;
        Ok(())
    }
}
