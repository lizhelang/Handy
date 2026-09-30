//! 私有父子通道的有界状态合同。ARM 回执是恢复依据，STOPPED 回执只是观测。
use crate::{native_quiescence::WriterProcessIdentity, Subject};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, marker::PhantomData, rc::Rc};

pub(super) const MAX_WRITERS: usize = 64;
pub(super) const MAX_FRAME: usize = 128 * 1024;
pub(super) const MAX_SESSION_BYTES: usize = 2 * 1024 * 1024;
pub(super) const MAX_MESSAGES: u64 = 512;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Binding {
    pub lease_id: String,
    pub subject: Subject,
    pub epoch: String,
    pub deadline_mono_ms: u64,
}
impl Binding {
    pub fn validate(&self) -> bool {
        self.deadline_mono_ms > 0
            && inputia_settings::installation::valid_uuid(&self.lease_id)
            && inputia_settings::installation::valid_uuid(&self.epoch)
            && inputia_settings::installation::valid_uuid(&self.subject.transaction_id)
            && inputia_settings::installation::valid_uuid(&self.subject.installation_id)
            && !self.subject.new_release_id.is_empty()
            && self.subject.new_release_id.len() <= 128
            && self.subject.plan_sha256.len() == 64
            && self
                .subject
                .plan_sha256
                .bytes()
                .all(|v| v.is_ascii_hexdigit() && !v.is_ascii_uppercase())
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum InitialState {
    Running,
    ObservedStopped,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Entry {
    pub index: u32,
    pub identity: WriterProcessIdentity,
    pub initial_state: InitialState,
}
impl Entry {
    fn valid(&self) -> bool {
        let p = &self.identity;
        self.index < MAX_WRITERS as u32
            && p.pid > 1
            && p.pid_version > 0
            && p.start_seconds > 0
            && p.start_microseconds < 1_000_000
            && matches!(p.role.as_str(), "control" | "ime" | "settings")
            && !p.bundle_id.is_empty()
            && p.bundle_id.len() <= 256
            && !p.release_id.is_empty()
            && p.release_id.len() <= 128
            && p.executable_path.starts_with('/')
            && p.executable_path.len() <= 1024
            && p.cdhash.len() == 40
            && p.cdhash
                .bytes()
                .all(|v| v.is_ascii_hexdigit() && !v.is_ascii_uppercase())
            && [&p.bundle_id, &p.release_id, &p.executable_path]
                .iter()
                .all(|s| !s.chars().any(char::is_control))
    }
    pub fn digest(&self, binding: &Binding) -> Result<String, ProtocolError> {
        let bytes = serde_json::to_vec(&(binding, self)).map_err(|_| ProtocolError::Malformed)?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Resolution {
    Running,
    Resumed,
    OriginalExited,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Message {
    Ready { entries: Vec<Entry> },
    Arm { index: u32, digest: String },
    ArmAck { index: u32, digest: String },
    Held { index: u32 },
    Holding {},
    Release {},
    Query {},
    Recovering {},
    Resolved { index: u32, resolution: Resolution },
    RecoveryRequired { indices: Vec<u32> },
    Resumed {},
    DisarmAck {},
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Envelope {
    pub binding: Binding,
    pub sequence: u64,
    pub message: Message,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ProtocolError {
    Malformed,
    Budget,
    Binding,
    Sequence,
    State,
    Entry,
    Digest,
}
impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "guardian_{self:?}")
    }
}
impl std::error::Error for ProtocolError {}

pub(super) struct Receiver {
    binding: Binding,
    next: u64,
    bytes: usize,
    previous: Option<(Vec<u8>, Message)>,
}
impl Receiver {
    pub fn new(binding: Binding) -> Result<Self, ProtocolError> {
        if !binding.validate() {
            return Err(ProtocolError::Binding);
        }
        Ok(Self {
            binding,
            next: 1,
            bytes: 0,
            previous: None,
        })
    }
    pub fn accept(&mut self, bytes: &[u8]) -> Result<(Message, bool), ProtocolError> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .ok_or(ProtocolError::Budget)?;
        if bytes.is_empty() || bytes.len() > MAX_FRAME || self.bytes > MAX_SESSION_BYTES {
            return Err(ProtocolError::Budget);
        }
        let message: Envelope =
            serde_json::from_slice(bytes).map_err(|_| ProtocolError::Malformed)?;
        if message.binding != self.binding {
            return Err(ProtocolError::Binding);
        }
        // 只允许最近一帧的逐字节重试。重试也算预算，不能无限保持暂停。
        if message.sequence.checked_add(1) == Some(self.next) {
            if let Some((old, result)) = &self.previous {
                if bytes == old {
                    return Ok((result.clone(), true));
                }
            }
            return Err(ProtocolError::Sequence);
        }
        if message.sequence != self.next || self.next > MAX_MESSAGES {
            return Err(ProtocolError::Sequence);
        }
        match &message.message {
            Message::Ready { entries } => validate_entries(entries)?,
            Message::RecoveryRequired { indices } => {
                if indices.len() > MAX_WRITERS
                    || !indices.windows(2).all(|p| p[0] < p[1])
                    || indices.iter().any(|i| *i >= MAX_WRITERS as u32)
                {
                    return Err(ProtocolError::Entry);
                }
            }
            Message::Arm { index, digest } | Message::ArmAck { index, digest } => {
                if *index >= MAX_WRITERS as u32
                    || digest.len() != 64
                    || !digest
                        .bytes()
                        .all(|v| v.is_ascii_hexdigit() && !v.is_ascii_uppercase())
                {
                    return Err(ProtocolError::Entry);
                }
            }
            Message::Held { index } | Message::Resolved { index, .. }
                if *index >= MAX_WRITERS as u32 =>
            {
                return Err(ProtocolError::Entry)
            }
            _ => {}
        }
        self.next += 1;
        self.previous = Some((bytes.to_vec(), message.message.clone()));
        Ok((message.message, false))
    }
}
fn validate_entries(entries: &[Entry]) -> Result<(), ProtocolError> {
    if entries.len() > MAX_WRITERS
        || entries
            .iter()
            .enumerate()
            .any(|(i, e)| e.index as usize != i || !e.valid())
        || !entries
            .windows(2)
            .all(|p| p[0].identity.pid < p[1].identity.pid)
    {
        return Err(ProtocolError::Entry);
    }
    Ok(())
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Phase {
    Ready,
    Arming,
    Holding,
    Recovering,
    Resumed,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EntryState {
    ObservedOnly,
    Prepared,
    Armed,
    EffectUnknown,
    Held,
    Resolved(Resolution),
}
/// 只能由单个串行效应执行器持有；监视线程仅使用取消 latch。
pub(super) struct Ledger {
    binding: Binding,
    phase: Phase,
    entries: BTreeMap<u32, (Entry, EntryState)>,
    _single_executor: PhantomData<Rc<()>>,
}
impl Ledger {
    pub fn new(binding: Binding, entries: Vec<Entry>) -> Result<Self, ProtocolError> {
        if !binding.validate() {
            return Err(ProtocolError::Binding);
        }
        validate_entries(&entries)?;
        Ok(Self {
            binding,
            phase: Phase::Ready,
            entries: entries
                .into_iter()
                .map(|e| {
                    let state = if e.initial_state == InitialState::ObservedStopped {
                        EntryState::ObservedOnly
                    } else {
                        EntryState::Prepared
                    };
                    (e.index, (e, state))
                })
                .collect(),
            _single_executor: PhantomData,
        })
    }
    pub fn phase(&self) -> Phase {
        self.phase
    }
    pub fn entries(&self) -> Vec<Entry> {
        self.entries.values().map(|(e, _)| e.clone()).collect()
    }
    pub fn arm(&mut self, index: u32, digest: &str) -> Result<bool, ProtocolError> {
        if !matches!(self.phase, Phase::Ready | Phase::Arming) {
            return Err(ProtocolError::State);
        }
        let (entry, state) = self.entries.get_mut(&index).ok_or(ProtocolError::Entry)?;
        if entry.initial_state != InitialState::Running || entry.digest(&self.binding)? != digest {
            return Err(ProtocolError::Digest);
        }
        self.phase = Phase::Arming;
        match *state {
            EntryState::Prepared => {
                *state = EntryState::Armed;
                Ok(true)
            }
            EntryState::Armed | EntryState::EffectUnknown | EntryState::Held => Ok(false),
            _ => Err(ProtocolError::State),
        }
    }
    /// ARM_ACK 已保存后才能得到 STOP 候选；同entry重试从不重复效应。
    pub fn begin_stop(&mut self, index: u32) -> Result<Option<Entry>, ProtocolError> {
        if self.phase != Phase::Arming {
            return Err(ProtocolError::State);
        }
        let (entry, state) = self.entries.get_mut(&index).ok_or(ProtocolError::Entry)?;
        match state {
            EntryState::Armed => {
                *state = EntryState::EffectUnknown;
                Ok(Some(entry.clone()))
            }
            EntryState::EffectUnknown | EntryState::Held => Ok(None),
            _ => Err(ProtocolError::State),
        }
    }
    pub fn held(&mut self, index: u32) -> Result<(), ProtocolError> {
        if self.phase != Phase::Arming {
            return Err(ProtocolError::State);
        }
        let (_, state) = self.entries.get_mut(&index).ok_or(ProtocolError::Entry)?;
        match state {
            EntryState::EffectUnknown | EntryState::Held => {
                *state = EntryState::Held;
                Ok(())
            }
            _ => Err(ProtocolError::State),
        }
    }
    pub fn holding(&mut self) -> Result<(), ProtocolError> {
        if !matches!(self.phase, Phase::Ready | Phase::Arming | Phase::Holding)
            || self
                .entries
                .values()
                .any(|(_, s)| !matches!(s, EntryState::ObservedOnly | EntryState::Held))
        {
            return Err(ProtocolError::State);
        }
        self.phase = Phase::Holding;
        Ok(())
    }
    /// 不可逆关闭 STOP。所有已ACK ARM均可能有未知效应，必须进入恢复集。
    pub fn recover(&mut self) {
        if self.phase != Phase::Resumed {
            self.phase = Phase::Recovering;
        }
    }
    pub fn recovery_entries(&self) -> Result<Vec<Entry>, ProtocolError> {
        if !matches!(self.phase, Phase::Recovering | Phase::Resumed) {
            return Err(ProtocolError::State);
        }
        Ok(self
            .entries
            .values()
            .filter_map(|(e, s)| {
                matches!(
                    s,
                    EntryState::Armed | EntryState::EffectUnknown | EntryState::Held
                )
                .then_some(e.clone())
            })
            .collect())
    }
    pub fn resolve(&mut self, index: u32, result: Resolution) -> Result<(), ProtocolError> {
        if self.phase != Phase::Recovering {
            return Err(ProtocolError::State);
        }
        let (_, state) = self.entries.get_mut(&index).ok_or(ProtocolError::Entry)?;
        match *state {
            EntryState::Armed | EntryState::EffectUnknown | EntryState::Held => {
                *state = EntryState::Resolved(result);
                Ok(())
            }
            EntryState::Resolved(previous) if previous == result => Ok(()),
            _ => Err(ProtocolError::State),
        }
    }
    pub fn resumed(&mut self) -> Result<(), ProtocolError> {
        if self.phase == Phase::Resumed {
            return Ok(());
        }
        if self.phase != Phase::Recovering || !self.recovery_entries()?.is_empty() {
            return Err(ProtocolError::State);
        }
        self.phase = Phase::Resumed;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn binding() -> Binding {
        Binding {
            lease_id: "11111111-1111-4111-8111-111111111111".into(),
            epoch: "22222222-2222-4222-8222-222222222222".into(),
            deadline_mono_ms: 60_000,
            subject: Subject {
                transaction_id: "33333333-3333-4333-8333-333333333333".into(),
                installation_id: "44444444-4444-4444-8444-444444444444".into(),
                new_release_id: "inputia-new".into(),
                plan_sha256: "a".repeat(64),
            },
        }
    }
    fn entry(index: u32, state: InitialState) -> Entry {
        Entry {
            index,
            initial_state: state,
            identity: WriterProcessIdentity {
                pid: 100 + index as i32,
                uid: 501,
                start_seconds: 1,
                start_microseconds: 2,
                pid_version: 3,
                role: "control".into(),
                bundle_id: "com.inputia.control".into(),
                release_id: "inputia-old".into(),
                executable_path: "/synthetic/Inputia.app/Contents/MacOS/Inputia".into(),
                cdhash: "b".repeat(40),
            },
        }
    }
    #[test]
    fn arm_is_write_ahead_and_original_stopped_never_enters_recovery() {
        let b = binding();
        let entries = vec![
            entry(0, InitialState::Running),
            entry(1, InitialState::ObservedStopped),
        ];
        let mut ledger = Ledger::new(b.clone(), entries.clone()).unwrap();
        assert!(ledger.begin_stop(0).is_err());
        assert!(ledger.arm(1, &entries[1].digest(&b).unwrap()).is_err());
        assert!(ledger.arm(0, &entries[0].digest(&b).unwrap()).unwrap());
        assert!(!ledger.arm(0, &entries[0].digest(&b).unwrap()).unwrap());
        assert!(ledger.begin_stop(0).unwrap().is_some());
        ledger.recover(); // 模拟STOP效果未知，尚无HELD。
        assert_eq!(ledger.recovery_entries().unwrap(), entries[..1]);
        assert!(ledger.resumed().is_err());
        ledger.resolve(0, Resolution::Resumed).unwrap();
        ledger.resumed().unwrap();
        assert!(ledger.arm(0, &entries[0].digest(&b).unwrap()).is_err());
        assert!(ledger.begin_stop(0).is_err());
        assert!(ledger.holding().is_err());
        ledger.recover();
        assert_eq!(ledger.phase(), Phase::Resumed);
    }
    #[test]
    fn duplicate_stop_is_not_replayed_and_release_never_reopens_effects() {
        let b = binding();
        let e = entry(0, InitialState::Running);
        let mut ledger = Ledger::new(b.clone(), vec![e.clone()]).unwrap();
        ledger.arm(0, &e.digest(&b).unwrap()).unwrap();
        assert!(ledger.begin_stop(0).unwrap().is_some());
        ledger.held(0).unwrap();
        assert!(ledger.begin_stop(0).unwrap().is_none());
        ledger.holding().unwrap();
        ledger.recover();
        assert!(ledger.held(0).is_err());
        assert!(ledger.holding().is_err());
        ledger.resolve(0, Resolution::OriginalExited).unwrap();
        ledger.resumed().unwrap();
        ledger.resumed().unwrap();
    }
    #[test]
    fn strict_frames_bind_sequences_and_have_total_budget() {
        let b = binding();
        let mut receiver = Receiver::new(b.clone()).unwrap();
        let raw = serde_json::to_vec(&Envelope {
            binding: b.clone(),
            sequence: 1,
            message: Message::Release {},
        })
        .unwrap();
        assert_eq!(receiver.accept(&raw).unwrap(), (Message::Release {}, false));
        assert_eq!(receiver.accept(&raw).unwrap(), (Message::Release {}, true));
        let mut value: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        value["message"]["unexpected"] = 1.into();
        assert!(receiver
            .accept(&serde_json::to_vec(&value).unwrap())
            .is_err());
        value = serde_json::from_slice(&raw).unwrap();
        value["sequence"] = 3.into();
        assert_eq!(
            receiver.accept(&serde_json::to_vec(&value).unwrap()),
            Err(ProtocolError::Sequence)
        );
        let mut count = 0;
        while receiver.accept(&raw).is_ok() {
            count += 1;
            assert!(count < MAX_SESSION_BYTES);
        }
        assert!(count > 0);
    }
    #[test]
    fn identity_lists_are_bounded_unique_and_operation_bound() {
        let b = binding();
        let e = entry(0, InitialState::Running);
        assert!(Ledger::new(b.clone(), vec![e.clone(), e.clone()]).is_err());
        assert!(Ledger::new(b.clone(), vec![e.clone(); MAX_WRITERS + 1]).is_err());
        let mut ledger = Ledger::new(b.clone(), vec![e.clone()]).unwrap();
        let mut other = b;
        other.epoch = "55555555-5555-4555-8555-555555555555".into();
        assert_eq!(
            ledger.arm(0, &e.digest(&other).unwrap()),
            Err(ProtocolError::Digest)
        );
    }
}
