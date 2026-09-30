//! 私有通道会话：guardian是唯一STOP owner；父侧仅在准确旧owner死亡后接管。
use super::{
    protocol::{Binding, InitialState, Ledger, Message, Phase, Resolution},
    recovery::{Effects, ExecutionState, Executor, Peer, PeerState},
    transport::{Channel, TransportError},
    GuardianError, GuardianStatus, LeaseRequest,
};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::Receiver,
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

pub(super) fn monotonic_ms() -> u64 {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) } != 0 {
        return u64::MAX;
    }
    (time.tv_sec as u64)
        .saturating_mul(1000)
        .saturating_add(time.tv_nsec as u64 / 1_000_000)
}
pub(super) fn set_status(state: &Mutex<GuardianStatus>, status: GuardianStatus) {
    if let Ok(mut state) = state.lock() {
        *state = status;
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Point {
    Ready,
    ArmSent,
    ArmSaved,
    ArmAck,
    BeforeStop,
    AfterStop,
    Holding,
    BeforeCont,
    AfterCont,
    BeforeTerminal,
    Terminal,
    Disarmed,
}
pub(super) trait Trace {
    fn at(&mut self, point: Point);
}
pub(super) struct NoTrace;
impl Trace for NoTrace {
    fn at(&mut self, _: Point) {}
}

pub(super) fn guardian_loop<E: Effects, P: Peer>(
    mut channel: Channel,
    binding: Binding,
    effects: E,
    peer: P,
    mut marker_valid: impl FnMut() -> bool,
    mut trace: impl Trace,
) -> Result<(), GuardianError> {
    let mut executor = Executor {
        ledger: Ledger::new(binding.clone(), effects.entries().to_vec())?,
        effects,
    };
    let cancelled = AtomicBool::new(false);
    let deadline = binding.deadline_mono_ms;
    let setup = (|| -> Result<(), GuardianError> {
        if peer.state()? != PeerState::Alive || !marker_valid() || monotonic_ms() >= deadline {
            return Err(GuardianError::Cancelled);
        }
        channel.send(Message::Ready {
            entries: executor.ledger.entries(),
        })?;
        trace.at(Point::Ready);
        for entry in executor.ledger.entries() {
            if entry.initial_state == InitialState::ObservedStopped {
                continue;
            }
            let digest = entry.digest(&binding)?;
            channel.send(Message::Arm {
                index: entry.index,
                digest: digest.clone(),
            })?;
            trace.at(Point::ArmSent);
            loop {
                if peer.state()? != PeerState::Alive
                    || !marker_valid()
                    || monotonic_ms() >= deadline
                {
                    return Err(GuardianError::Cancelled);
                }
                if let Some((message, duplicate)) = channel.receive(Duration::from_millis(20))? {
                    if duplicate {
                        continue;
                    }
                    match message {
                        Message::ArmAck {
                            index,
                            digest: received,
                        } if index == entry.index && received == digest => {
                            executor.ledger.arm(index, &received)?;
                            trace.at(Point::ArmAck);
                            break;
                        }
                        Message::Release {} => return Err(GuardianError::Cancelled),
                        _ => return Err(GuardianError::Protocol("unexpected_arm_reply".into())),
                    }
                }
            }
            trace.at(Point::BeforeStop);
            executor.stop(entry.index, &cancelled, &mut || {
                marker_valid()
                    && monotonic_ms() < deadline
                    && matches!(peer.state(), Ok(PeerState::Alive))
            })?;
            trace.at(Point::AfterStop);
            channel.send(Message::Held { index: entry.index })?;
        }
        executor.holding()?;
        if !marker_valid() || peer.state()? != PeerState::Alive || monotonic_ms() >= deadline {
            return Err(GuardianError::Cancelled);
        }
        channel.send(Message::Holding {})?;
        trace.at(Point::Holding);
        loop {
            if peer.state()? != PeerState::Alive || !marker_valid() || monotonic_ms() >= deadline {
                return Err(GuardianError::Cancelled);
            }
            if let Some((message, duplicate)) = channel.receive(Duration::from_millis(20))? {
                if duplicate {
                    continue;
                }
                match message {
                    Message::Release {} => return Ok(()),
                    Message::Query {} => {
                        executor.effects.assert_holding()?;
                        if !marker_valid()
                            || peer.state()? != PeerState::Alive
                            || monotonic_ms() >= deadline
                        {
                            return Err(GuardianError::Cancelled);
                        }
                        channel.send(Message::Holding {})?;
                    }
                    _ => return Err(GuardianError::Protocol("unexpected_holding_message".into())),
                }
            }
        }
    })();
    // 任意错误/EOF/expiry都只让当前owner关闭STOP并恢复，绝不把控制权交给仍活的另一方。
    cancelled.store(true, Ordering::Release);
    executor.begin_recovery()?;
    let _ = channel.send(Message::Recovering {});
    loop {
        trace.at(Point::BeforeCont);
        let resolved = executor.recover_once()?;
        trace.at(Point::AfterCont);
        for (index, resolution) in resolved {
            let _ = channel.send(Message::Resolved { index, resolution });
        }
        if executor.ledger.phase() == Phase::Resumed {
            break;
        }
        let indices = executor
            .ledger
            .recovery_entries()?
            .iter()
            .map(|entry| entry.index)
            .collect();
        let _ = channel.send(Message::RecoveryRequired { indices });
        // 保留清单及共享flock；不将恢复失败变成退出成功。内存/重试频率有界。
        std::thread::sleep(Duration::from_millis(250));
    }
    trace.at(Point::BeforeTerminal);
    let _ = channel.send(Message::Resumed {});
    trace.at(Point::Terminal);
    let until = Instant::now() + Duration::from_secs(1);
    while Instant::now() < until {
        match channel.receive(Duration::from_millis(20)) {
            Ok(Some((Message::DisarmAck {}, _))) => {
                trace.at(Point::Disarmed);
                return Ok(());
            }
            Ok(Some((Message::Query {} | Message::Release {}, _))) => {
                let _ = channel.send(Message::Resumed {});
            }
            Err(TransportError::Closed) => return Ok(()),
            Err(_) => break,
            _ => {}
        }
    }
    // 所有效应已封闭且恢复完成，丢terminal ACK也可退出；父确认EXIT后只能做幂等恢复。
    let _ = setup;
    Ok(())
}

pub(super) struct ParentControl<'a> {
    pub state: Arc<Mutex<GuardianStatus>>,
    pub cancelled: Arc<AtomicBool>,
    pub requests: Receiver<LeaseRequest>,
    pub marker_valid: Box<dyn FnMut() -> bool + 'a>,
}

pub(super) fn parent_loop<E: Effects, P: Peer>(
    mut channel: Channel,
    binding: Binding,
    effects: E,
    peer: P,
    control: ParentControl<'_>,
    mut trace: impl Trace,
) -> Result<(), GuardianError> {
    let ParentControl {
        state,
        cancelled,
        requests,
        mut marker_valid,
    } = control;
    let mut executor = Executor {
        ledger: Ledger::new(binding.clone(), effects.entries().to_vec())?,
        effects,
    };
    let mut ready = false;
    let mut owner_gone = false;
    let mut release_sent = false;
    let mut channel_failed = false;
    let mut inspect: Option<std::sync::mpsc::SyncSender<Result<(), GuardianError>>> = None;
    loop {
        let peer_state = peer.state().unwrap_or(PeerState::Unknown);
        owner_gone |= peer_state.owner_gone();
        if owner_gone {
            cancelled.store(true, Ordering::Release);
            set_status(&state, GuardianStatus::Recovering);
            trace.at(Point::BeforeCont);
            // 精确EXIT/EXEC是一旦成立不会撤销的事实；暂时读取失败不丢恢复责任。
            if executor.recover_once().is_err() {
                set_status(&state, GuardianStatus::RecoveryRequired);
                std::thread::sleep(Duration::from_millis(250));
                continue;
            }
            trace.at(Point::AfterCont);
            if executor.ledger.phase() == Phase::Resumed {
                set_status(&state, GuardianStatus::Resumed);
                if let Some(reply) = inspect.take() {
                    let _ = reply.send(Err(GuardianError::Cancelled));
                }
                return Ok(());
            }
            set_status(&state, GuardianStatus::RecoveryRequired);
            std::thread::sleep(Duration::from_millis(250));
            continue;
        }
        if peer_state != PeerState::Alive {
            cancelled.store(true, Ordering::Release);
            set_status(&state, GuardianStatus::RecoveryRequired);
            channel_failed = true;
        }
        if monotonic_ms() >= binding.deadline_mono_ms {
            cancelled.store(true, Ordering::Release);
        }
        while let Ok(LeaseRequest::Inspect(reply)) = requests.try_recv() {
            if inspect.is_some()
                || cancelled.load(Ordering::Acquire)
                || executor.ledger.phase() != Phase::Holding
                || channel_failed
            {
                let _ = reply.send(Err(GuardianError::RecoveryPending));
            } else if channel.send(Message::Query {}).is_ok() {
                inspect = Some(reply);
            } else {
                channel_failed = true;
                let _ = reply.send(Err(GuardianError::RecoveryPending));
            }
        }
        if cancelled.load(Ordering::Acquire) && !release_sent {
            set_status(&state, GuardianStatus::Recovering);
            release_sent = channel.send(Message::Release {}).is_ok();
        }
        if channel_failed {
            // EOF并不证明guardian已经停止效应。准确EXIT/EXEC之前不发CONT。
            cancelled.store(true, Ordering::Release);
            set_status(&state, GuardianStatus::RecoveryRequired);
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }
        let received = match channel.receive(Duration::from_millis(20)) {
            Ok(Some((message, false))) => message,
            Ok(_) => continue,
            Err(_) => {
                channel_failed = true;
                continue;
            }
        };
        let result = (|| -> Result<(), GuardianError> {
            match received {
                Message::Ready { entries } if !ready => {
                    if entries != executor.effects.entries() {
                        return Err(GuardianError::Protocol("inventory_changed".into()));
                    }
                    ready = true;
                    trace.at(Point::Ready);
                }
                Message::Arm { index, digest } if ready && !cancelled.load(Ordering::Acquire) => {
                    executor.ledger.arm(index, &digest)?;
                    executor.ledger.begin_stop(index)?; // 父备份先标未知效应，之后才ACK；父不实际STOP。
                    trace.at(Point::ArmSaved);
                    channel.send(Message::ArmAck { index, digest })?;
                    trace.at(Point::ArmAck);
                }
                Message::Held { index } if ready => {
                    executor.ledger.held(index)?;
                }
                Message::Holding {} if ready && !cancelled.load(Ordering::Acquire) => {
                    executor.holding()?;
                    if !marker_valid()
                        || peer.state()? != PeerState::Alive
                        || cancelled.load(Ordering::Acquire)
                        || monotonic_ms() >= binding.deadline_mono_ms
                    {
                        return Err(GuardianError::Cancelled);
                    }
                    set_status(&state, GuardianStatus::Holding);
                    trace.at(Point::Holding);
                    if let Some(reply) = inspect.take() {
                        if !marker_valid()
                            || peer.state()? != PeerState::Alive
                            || cancelled.load(Ordering::Acquire)
                            || monotonic_ms() >= binding.deadline_mono_ms
                        {
                            let _ = reply.send(Err(GuardianError::Cancelled));
                            return Err(GuardianError::Cancelled);
                        }
                        let _ = reply.send(Ok(()));
                    }
                }
                Message::Recovering {} => {
                    executor.begin_recovery()?;
                    set_status(&state, GuardianStatus::Recovering);
                }
                Message::Resolved { index, resolution } => {
                    executor.begin_recovery()?;
                    let actual = executor.effects.state(index)?;
                    if !matches!(
                        (actual, resolution),
                        (
                            ExecutionState::Running,
                            Resolution::Running | Resolution::Resumed
                        ) | (ExecutionState::OriginalExited, Resolution::OriginalExited)
                    ) {
                        return Err(GuardianError::RecoveryPending);
                    }
                    executor.ledger.resolve(index, resolution)?;
                }
                Message::RecoveryRequired { .. } => {
                    executor.begin_recovery()?;
                    set_status(&state, GuardianStatus::RecoveryRequired);
                }
                Message::Resumed {} => {
                    executor.begin_recovery()?;
                    // 缺任意已ARM条目的真实readback不能只凭terminal phase假完成。
                    executor.ledger.resumed()?;
                    channel.send(Message::DisarmAck {})?;
                    set_status(&state, GuardianStatus::Resumed);
                    trace.at(Point::Disarmed);
                }
                _ => {
                    return Err(GuardianError::Protocol(
                        "unexpected_guardian_message".into(),
                    ))
                }
            }
            Ok(())
        })();
        if result.is_err() {
            cancelled.store(true, Ordering::Release);
            channel_failed = true;
        }
        if executor.ledger.phase() == Phase::Resumed {
            // 仍由运行的guardian关闭其FD；其已不可逆封STOP，父不需要接管。
            return Ok(());
        }
    }
}
