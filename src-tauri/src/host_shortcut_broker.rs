//! 已认证 Inputia host 与本机统一语音快捷键之间的短租约转发。
//! broker 不插入文字、不拥有录音状态；它只把快捷键边沿绑定到 host 预捕获目标。

use crate::settings::ShortcutActivation;
use inputia_handy_runtime::voice_protocol::{
    HostShortcutCommand, HostShortcutLease, HostShortcutReply, HostShortcutRequest,
    HostShortcutTrigger, VoiceCommand, VoiceReplyError, VoiceRequest, VoiceShortcutActivation,
};
use log::debug;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MAX_LEASE_TTL: Duration = Duration::from_millis(1500);
const MAX_POLL_WAIT: Duration = Duration::from_millis(1000);
const MAX_ISSUED_TRIGGERS: usize = 4096;
#[cfg(target_os = "macos")]
const INPUTIA_CANDIDATE_BUNDLE_ID: &str = "com.inputia.inputmethod.Inputia.UnifiedCandidate";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutRouting {
    Legacy,
    Forwarded,
    HostPending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CurrentInputSource {
    InputiaCandidate,
    Other,
    Unknown,
}

#[derive(Clone)]
struct LeaseRecord {
    lease: HostShortcutLease,
    server_instance: String,
    client_instance: String,
    policy_epoch: u64,
    expires_at: Instant,
}

#[derive(Clone)]
struct ActiveSession {
    session_id: String,
    pressed_at: Instant,
    lease: LeaseRecord,
}

#[derive(Default)]
struct BrokerState {
    connected: HashMap<String, usize>,
    leases: HashMap<String, LeaseRecord>,
    active: HashMap<(String, String), ActiveSession>,
    queue: VecDeque<HostShortcutTrigger>,
    issued: HashMap<String, HostShortcutTrigger>,
}

#[derive(Default)]
pub struct HostShortcutBroker {
    state: Mutex<BrokerState>,
    changed: Condvar,
    serial: AtomicU64,
}

impl HostShortcutBroker {
    pub fn note_connected(&self, client_instance: &str) {
        let mut state = self.state.lock().expect("host shortcut broker poisoned");
        let count = state
            .connected
            .entry(client_instance.to_owned())
            .or_default();
        *count = count.saturating_add(1);
    }

    pub fn note_disconnected(&self, client_instance: &str) -> bool {
        self.note_disconnected_and_notify(client_instance, || {})
    }

    /// on_last 只可向协调器入队；持锁保证断开通知先于重连后的新请求。
    pub fn note_disconnected_and_notify(
        &self,
        client_instance: &str,
        on_last: impl FnOnce(),
    ) -> bool {
        let mut state = self.state.lock().expect("host shortcut broker poisoned");
        let mut last_connection = false;
        if let Some(count) = state.connected.get_mut(client_instance) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                last_connection = true;
                state.connected.remove(client_instance);
                state
                    .leases
                    .retain(|_, lease| lease.client_instance != client_instance);
                state
                    .queue
                    .retain(|trigger| trigger.client_instance != client_instance);
                state
                    .issued
                    .retain(|_, trigger| trigger.client_instance != client_instance);
            }
        }
        self.changed.notify_all();
        if last_connection {
            on_last();
        }
        last_connection
    }

    pub fn process_request(&self, request: HostShortcutRequest) -> HostShortcutReply {
        let request_id = request.request_id.clone();
        match request.shortcut {
            HostShortcutCommand::Register { lease } => {
                match self.register(
                    request.client_instance,
                    request.server_instance,
                    request.policy_epoch,
                    lease,
                ) {
                    Ok((lease_id, lease_epoch)) => HostShortcutReply::Registered {
                        request_id,
                        lease_id,
                        lease_epoch,
                    },
                    Err(code) => HostShortcutReply::Rejected { request_id, code },
                }
            }
            HostShortcutCommand::Retire {
                lease_id,
                lease_epoch,
            } => {
                self.retire(&request.client_instance, &lease_id, lease_epoch);
                HostShortcutReply::Retired { request_id }
            }
            HostShortcutCommand::Reject { trigger_id } => {
                match self.reject_start_trigger(&request.client_instance, &trigger_id) {
                    Ok(()) => HostShortcutReply::Retired { request_id },
                    Err(code) => HostShortcutReply::Rejected { request_id, code },
                }
            }
            HostShortcutCommand::Poll { max_wait_ms } => {
                match self.poll(&request.client_instance, max_wait_ms) {
                    Some(trigger) => HostShortcutReply::Trigger {
                        request_id,
                        trigger: Box::new(trigger),
                    },
                    None => HostShortcutReply::Empty { request_id },
                }
            }
        }
    }

    pub fn route_shortcut_event(
        &self,
        binding_id: &str,
        hotkey_string: &str,
        is_pressed: bool,
        mode: ShortcutActivation,
        hold_threshold: Duration,
        policy_epoch: Option<u64>,
        current_source: CurrentInputSource,
    ) -> ShortcutRouting {
        self.route_shortcut_event_with_source(
            binding_id,
            hotkey_string,
            is_pressed,
            mode,
            hold_threshold,
            policy_epoch,
            current_source,
        )
    }

    fn route_shortcut_event_with_source(
        &self,
        binding_id: &str,
        hotkey_string: &str,
        is_pressed: bool,
        mode: ShortcutActivation,
        hold_threshold: Duration,
        policy_epoch: Option<u64>,
        current_source: CurrentInputSource,
    ) -> ShortcutRouting {
        let mut state = self.state.lock().expect("host shortcut broker poisoned");
        let now = Instant::now();
        state.purge_expired(now);
        let active = Self::active_lease_for(&state, binding_id);
        if let Some((key, active_session)) = active.as_ref() {
            if !state
                .connected
                .contains_key(&active_session.lease.client_instance)
            {
                if edge_clears_active(active_session, is_pressed, mode, hold_threshold, now) {
                    state.active.remove(key);
                }
                debug!(
                    "Inputia host shortcut session already owns the edge, but host is disconnected"
                );
                return ShortcutRouting::HostPending;
            }
        }
        let Some(lease) = active
            .as_ref()
            .map(|(_, active)| active.lease.clone())
            .or_else(|| policy_epoch.and_then(|epoch| Self::ready_lease_for(&state, epoch)))
        else {
            if policy_epoch.is_none() || current_source != CurrentInputSource::Other {
                debug!("Input source is Inputia or unknown and no ready host target lease exists");
                return ShortcutRouting::HostPending;
            }
            debug!("No ready Inputia host target lease for voice shortcut; using legacy path");
            return ShortcutRouting::Legacy;
        };
        if active.is_none() && !is_pressed {
            debug!("Ignoring host shortcut release without an active Inputia session");
            return ShortcutRouting::HostPending;
        }
        let key = active
            .map(|(key, _)| key)
            .unwrap_or_else(|| (lease.client_instance.clone(), binding_id.to_owned()));
        let (session_id, starts_session, clear_after) = next_session(
            &mut state.active,
            &key,
            lease.clone(),
            is_pressed,
            mode,
            hold_threshold,
            now,
            || self.next_id("host-session"),
        );
        let trigger = HostShortcutTrigger {
            trigger_id: self.next_id("host-trigger"),
            session_id,
            starts_session,
            lease_id: lease.lease.lease_id.clone(),
            lease_epoch: lease.lease.lease_epoch,
            target: lease.lease.target.clone(),
            binding_id: binding_id.to_owned(),
            hotkey_string: hotkey_string.to_owned(),
            is_pressed,
            activation: activation(mode),
            pressed_at_unix_ms: unix_ms(),
            hold_threshold_ms: hold_threshold.as_millis().min(u128::from(u64::MAX)) as u64,
            server_instance: lease.server_instance.clone(),
            client_instance: lease.client_instance.clone(),
            policy_epoch: lease.policy_epoch,
        };
        if clear_after {
            state.active.remove(&key);
        }
        state.queue.push_back(trigger);
        self.changed.notify_all();
        ShortcutRouting::Forwarded
    }

    pub fn consume_voice_command(&self, request: &VoiceRequest) -> Result<(), VoiceReplyError> {
        let VoiceCommand::HostShortcut { target, edge, .. } = &request.command else {
            return Ok(());
        };
        let mut state = self.state.lock().expect("host shortcut broker poisoned");
        let Some(trigger) = state.issued.remove(&edge.trigger_id) else {
            return Err(VoiceReplyError::Unauthorized);
        };
        if trigger.session_id != request.session_id
            || trigger.client_instance != request.client_instance
            || trigger.server_instance != request.server_instance
            || trigger.policy_epoch != request.policy_epoch
            || trigger.target != *target
            || trigger.starts_session != edge.starts_session
            || trigger.lease_id != edge.lease_id
            || trigger.lease_epoch != edge.lease_epoch
            || trigger.binding_id != edge.binding_id
            || trigger.hotkey_string != edge.hotkey_string
            || trigger.is_pressed != edge.is_pressed
            || trigger.activation != edge.activation
            || trigger.hold_threshold_ms != edge.hold_threshold_ms
        {
            return Err(VoiceReplyError::Unauthorized);
        }
        Ok(())
    }

    fn active_lease_for(
        state: &BrokerState,
        binding_id: &str,
    ) -> Option<((String, String), ActiveSession)> {
        state
            .active
            .iter()
            .find(|((_, active_binding), _)| active_binding == binding_id)
            .map(|(key, session)| (key.clone(), session.clone()))
    }

    fn ready_lease_for(state: &BrokerState, policy_epoch: u64) -> Option<LeaseRecord> {
        state
            .leases
            .values()
            .filter(|lease| lease.policy_epoch == policy_epoch)
            .max_by_key(|lease| lease.lease.lease_epoch)
            .cloned()
    }

    fn register(
        &self,
        client_instance: String,
        server_instance: String,
        policy_epoch: u64,
        lease: HostShortcutLease,
    ) -> Result<(String, u64), VoiceReplyError> {
        if lease.target.host_instance != client_instance || unix_ms() >= lease.expires_at_unix_ms {
            return Err(VoiceReplyError::Unauthorized);
        }
        let ttl_ms = lease
            .expires_at_unix_ms
            .saturating_sub(unix_ms())
            .min(MAX_LEASE_TTL.as_millis() as u64);
        if ttl_ms == 0 {
            return Err(VoiceReplyError::Unauthorized);
        }
        let expires_at = Instant::now() + Duration::from_millis(ttl_ms);
        let mut state = self.state.lock().expect("host shortcut broker poisoned");
        if !state.connected.contains_key(&client_instance) {
            return Err(VoiceReplyError::Unauthorized);
        }
        if let Some(previous) = state.leases.get(&client_instance) {
            if lease.lease_epoch < previous.lease.lease_epoch {
                return Err(VoiceReplyError::Unauthorized);
            }
            if lease.lease_epoch == previous.lease.lease_epoch
                && (lease.lease_id != previous.lease.lease_id
                    || lease.target != previous.lease.target
                    || server_instance != previous.server_instance
                    || policy_epoch != previous.policy_epoch)
            {
                return Err(VoiceReplyError::Unauthorized);
            }
        }
        let lease_id = lease.lease_id.clone();
        let lease_epoch = lease.lease_epoch;
        state.leases.insert(
            client_instance.clone(),
            LeaseRecord {
                lease,
                server_instance,
                client_instance,
                policy_epoch,
                expires_at,
            },
        );
        self.changed.notify_all();
        Ok((lease_id, lease_epoch))
    }

    fn retire(&self, client_instance: &str, lease_id: &str, lease_epoch: u64) {
        let mut state = self.state.lock().expect("host shortcut broker poisoned");
        if state.leases.get(client_instance).is_some_and(|record| {
            record.lease.lease_id == lease_id && record.lease.lease_epoch == lease_epoch
        }) {
            state.leases.remove(client_instance);
            state
                .active
                .retain(|(client, _), _| client.as_str() != client_instance);
            state
                .queue
                .retain(|trigger| trigger.client_instance != client_instance);
            state
                .issued
                .retain(|_, trigger| trigger.client_instance != client_instance);
        }
        self.changed.notify_all();
    }

    fn reject_start_trigger(
        &self,
        client_instance: &str,
        trigger_id: &str,
    ) -> Result<(), VoiceReplyError> {
        let mut state = self.state.lock().expect("host shortcut broker poisoned");
        let Some(trigger) = state.issued.get(trigger_id).cloned() else {
            return Err(VoiceReplyError::Unauthorized);
        };
        if trigger.client_instance != client_instance || !trigger.starts_session {
            return Err(VoiceReplyError::Unauthorized);
        }
        state.issued.remove(trigger_id);
        state.queue.retain(|queued| {
            queued.client_instance != client_instance || queued.session_id != trigger.session_id
        });
        state.active.retain(|(client, _), active| {
            client.as_str() != client_instance || active.session_id != trigger.session_id
        });
        self.changed.notify_all();
        Ok(())
    }

    fn poll(&self, client_instance: &str, max_wait_ms: u64) -> Option<HostShortcutTrigger> {
        let wait = Duration::from_millis(max_wait_ms).min(MAX_POLL_WAIT);
        let deadline = Instant::now() + wait;
        let mut state = self.state.lock().expect("host shortcut broker poisoned");
        loop {
            state.purge_expired(Instant::now());
            if let Some(index) = state
                .queue
                .iter()
                .position(|trigger| trigger.client_instance == client_instance)
            {
                let trigger = state.queue.remove(index)?;
                if state.issued.len() >= MAX_ISSUED_TRIGGERS {
                    state.issued.clear();
                }
                state
                    .issued
                    .insert(trigger.trigger_id.clone(), trigger.clone());
                return Some(trigger);
            }
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            let timeout = deadline.saturating_duration_since(now);
            let (next, result) = self
                .changed
                .wait_timeout(state, timeout)
                .expect("host shortcut broker poisoned");
            state = next;
            if result.timed_out() {
                return None;
            }
        }
    }

    fn next_id(&self, prefix: &str) -> String {
        let value = self.serial.fetch_add(1, Ordering::Relaxed) + 1;
        format!("{prefix}-{value}")
    }
}

impl BrokerState {
    fn purge_expired(&mut self, now: Instant) {
        let expired = self
            .leases
            .iter()
            .filter_map(|(client, lease)| (lease.expires_at <= now).then_some(client.clone()))
            .collect::<Vec<_>>();
        for client in expired {
            debug!("Expiring Inputia host target lease");
            self.leases.remove(&client);
        }
    }
}

fn next_session(
    active: &mut HashMap<(String, String), ActiveSession>,
    key: &(String, String),
    lease: LeaseRecord,
    is_pressed: bool,
    mode: ShortcutActivation,
    hold_threshold: Duration,
    now: Instant,
    next_id: impl FnOnce() -> String,
) -> (String, bool, bool) {
    if let Some(session) = active.get(key) {
        let clear = match mode {
            ShortcutActivation::Toggle => is_pressed,
            ShortcutActivation::PushToTalk => !is_pressed,
            ShortcutActivation::HoldOrToggle => {
                if is_pressed {
                    true
                } else {
                    now.saturating_duration_since(session.pressed_at) >= hold_threshold
                }
            }
        };
        return (session.session_id.clone(), false, clear);
    }
    let session_id = next_id();
    if is_pressed {
        active.insert(
            key.clone(),
            ActiveSession {
                session_id: session_id.clone(),
                pressed_at: now,
                lease,
            },
        );
    }
    (session_id, true, false)
}

fn edge_clears_active(
    session: &ActiveSession,
    is_pressed: bool,
    mode: ShortcutActivation,
    hold_threshold: Duration,
    now: Instant,
) -> bool {
    match mode {
        ShortcutActivation::Toggle => is_pressed,
        ShortcutActivation::PushToTalk => !is_pressed,
        ShortcutActivation::HoldOrToggle => {
            is_pressed || now.saturating_duration_since(session.pressed_at) >= hold_threshold
        }
    }
}

fn activation(value: ShortcutActivation) -> VoiceShortcutActivation {
    match value {
        ShortcutActivation::Toggle => VoiceShortcutActivation::Toggle,
        ShortcutActivation::PushToTalk => VoiceShortcutActivation::PushToTalk,
        ShortcutActivation::HoldOrToggle => VoiceShortcutActivation::HoldOrToggle,
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn current_input_source() -> CurrentInputSource {
    if objc2::MainThreadMarker::new().is_none() {
        // 防止未来调用者绕过主线程交接：未知不能降级到普通粘贴。
        return CurrentInputSource::Unknown;
    }
    use std::ffi::{c_char, c_void, CStr};

    type CFRef = *const c_void;

    const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

    #[link(name = "Carbon", kind = "framework")]
    unsafe extern "C" {
        fn TISCopyCurrentKeyboardInputSource() -> CFRef;
        fn TISGetInputSourceProperty(source: CFRef, key: CFRef) -> CFRef;
        static kTISPropertyBundleID: CFRef;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(value: CFRef);
        fn CFStringGetCString(
            value: CFRef,
            buffer: *mut c_char,
            buffer_size: isize,
            encoding: u32,
        ) -> u8;
    }

    struct InputSource(CFRef);

    impl Drop for InputSource {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: TISCopyCurrentKeyboardInputSource 返回拥有的引用。
                unsafe { CFRelease(self.0) };
            }
        }
    }

    // SAFETY: TIS Copy 遵循 CoreFoundation Create Rule；bundle 属性只在
    // source 生命周期内读取为 UTF-8 文本，不写系统状态、不申请权限。
    unsafe {
        let source = InputSource(TISCopyCurrentKeyboardInputSource());
        if source.0.is_null() {
            return CurrentInputSource::Unknown;
        }
        let bundle = TISGetInputSourceProperty(source.0, kTISPropertyBundleID);
        if bundle.is_null() {
            return CurrentInputSource::Unknown;
        }
        let mut buffer = [0 as c_char; 512];
        if CFStringGetCString(
            bundle,
            buffer.as_mut_ptr(),
            buffer.len() as isize,
            K_CF_STRING_ENCODING_UTF8,
        ) == 0
        {
            return CurrentInputSource::Unknown;
        }
        match CStr::from_ptr(buffer.as_ptr()).to_str() {
            Ok(INPUTIA_CANDIDATE_BUNDLE_ID) => CurrentInputSource::InputiaCandidate,
            Ok(_) => CurrentInputSource::Other,
            Err(_) => CurrentInputSource::Unknown,
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn current_input_source() -> CurrentInputSource {
    CurrentInputSource::Unknown
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_last_connection_marks_the_inputia_process_disconnected() {
        let broker = HostShortcutBroker::default();
        broker.note_connected("same-host");
        broker.note_connected("same-host");
        assert!(!broker.note_disconnected("same-host"));
        assert!(broker.note_disconnected("same-host"));
        assert!(!broker.note_disconnected("same-host"));
    }
    use inputia_handy_runtime::voice_protocol::HostTargetToken;

    fn lease(id: &str, epoch: u64) -> HostShortcutLease {
        HostShortcutLease {
            lease_id: id.into(),
            lease_epoch: epoch,
            target: HostTargetToken {
                target_id: "target-one".into(),
                host_instance: "host-one".into(),
                controller_id: "controller-one".into(),
                activation_generation: 9,
                field_id: Some("field-one".into()),
                selection_generation: 3,
                composition_generation: 4,
                source_app: Some("synthetic.editor".into()),
            },
            issued_at_unix_ms: unix_ms(),
            expires_at_unix_ms: unix_ms() + 10_000,
        }
    }

    fn register_default_lease(broker: &HostShortcutBroker) {
        broker.note_connected("host-one");
        assert!(matches!(
            broker.process_request(HostShortcutRequest {
                request_id: "register".into(),
                client_instance: "host-one".into(),
                server_instance: "server-one".into(),
                policy_epoch: 4,
                shortcut: HostShortcutCommand::Register {
                    lease: lease("lease-one", 1)
                },
            }),
            HostShortcutReply::Registered { .. }
        ));
    }

    fn route_with_source(
        broker: &HostShortcutBroker,
        is_pressed: bool,
        mode: ShortcutActivation,
        source: CurrentInputSource,
    ) -> ShortcutRouting {
        broker.route_shortcut_event_with_source(
            "transcribe",
            "Option+Space",
            is_pressed,
            mode,
            Duration::ZERO,
            Some(4),
            source,
        )
    }

    #[test]
    fn no_host_keeps_legacy_path() {
        let broker = HostShortcutBroker::default();
        assert_eq!(
            route_with_source(
                &broker,
                true,
                ShortcutActivation::Toggle,
                CurrentInputSource::Other,
            ),
            ShortcutRouting::Legacy
        );
    }

    #[test]
    fn connected_host_without_lease_keeps_legacy_path() {
        let broker = HostShortcutBroker::default();
        broker.note_connected("host-one");
        assert_eq!(
            route_with_source(
                &broker,
                true,
                ShortcutActivation::Toggle,
                CurrentInputSource::Other,
            ),
            ShortcutRouting::Legacy
        );
        assert!(broker.poll("host-one", 0).is_none());
    }

    #[test]
    fn unknown_source_without_lease_does_not_fall_back_to_paste() {
        let broker = HostShortcutBroker::default();
        assert_eq!(
            route_with_source(
                &broker,
                true,
                ShortcutActivation::Toggle,
                CurrentInputSource::Unknown
            ),
            ShortcutRouting::HostPending
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn background_input_source_probe_does_not_call_carbon() {
        assert_eq!(
            std::thread::spawn(current_input_source).join().unwrap(),
            CurrentInputSource::Unknown
        );
    }

    #[test]
    fn selected_inputia_without_lease_reports_host_pending() {
        let broker = HostShortcutBroker::default();
        assert_eq!(
            route_with_source(
                &broker,
                true,
                ShortcutActivation::Toggle,
                CurrentInputSource::InputiaCandidate,
            ),
            ShortcutRouting::HostPending
        );
        broker.note_connected("host-one");
        assert_eq!(
            route_with_source(
                &broker,
                true,
                ShortcutActivation::Toggle,
                CurrentInputSource::InputiaCandidate,
            ),
            ShortcutRouting::HostPending
        );
        assert!(broker.poll("host-one", 0).is_none());
    }

    #[test]
    fn missing_policy_does_not_start_or_fall_back_but_can_stop_owned_session() {
        let broker = HostShortcutBroker::default();
        register_default_lease(&broker);
        let without_policy = || {
            broker.route_shortcut_event_with_source(
                "transcribe",
                "Option+Space",
                true,
                ShortcutActivation::Toggle,
                Duration::ZERO,
                None,
                CurrentInputSource::Other,
            )
        };
        assert_eq!(without_policy(), ShortcutRouting::HostPending);
        assert!(broker.poll("host-one", 0).is_none());
        assert_eq!(
            route_with_source(
                &broker,
                true,
                ShortcutActivation::Toggle,
                CurrentInputSource::InputiaCandidate
            ),
            ShortcutRouting::Forwarded
        );
        let start = broker.poll("host-one", 0).unwrap();
        assert_eq!(without_policy(), ShortcutRouting::Forwarded);
        let stop = broker.poll("host-one", 0).unwrap();
        assert_eq!(stop.session_id, start.session_id);
        assert_eq!(stop.target, start.target);
        assert!(!stop.starts_session);
    }

    #[test]
    fn valid_lease_receives_bound_trigger() {
        let broker = HostShortcutBroker::default();
        register_default_lease(&broker);
        assert_eq!(
            broker.route_shortcut_event_with_source(
                "transcribe",
                "Option+Space",
                true,
                ShortcutActivation::HoldOrToggle,
                Duration::from_millis(400),
                Some(4),
                CurrentInputSource::Other,
            ),
            ShortcutRouting::Forwarded
        );
        let trigger = broker.poll("host-one", 0).unwrap();
        assert_eq!(trigger.lease_id, "lease-one");
        assert!(trigger.starts_session);
        assert_eq!(trigger.lease_epoch, 1);
        assert_eq!(trigger.target.target_id, "target-one");
        assert_eq!(trigger.activation, VoiceShortcutActivation::HoldOrToggle);
        assert!(trigger.is_pressed);
        assert_eq!(trigger.policy_epoch, 4);
    }

    #[test]
    fn expired_lease_is_not_forwarded_or_polled() {
        let broker = HostShortcutBroker::default();
        broker.note_connected("host-one");
        let mut expired = lease("lease-one", 1);
        expired.expires_at_unix_ms = unix_ms().saturating_sub(1);
        assert!(matches!(
            broker.process_request(HostShortcutRequest {
                request_id: "register".into(),
                client_instance: "host-one".into(),
                server_instance: "server-one".into(),
                policy_epoch: 4,
                shortcut: HostShortcutCommand::Register { lease: expired },
            }),
            HostShortcutReply::Rejected { .. }
        ));
        assert_eq!(
            route_with_source(
                &broker,
                true,
                ShortcutActivation::Toggle,
                CurrentInputSource::Other,
            ),
            ShortcutRouting::Legacy
        );
    }

    #[test]
    fn stale_or_conflicting_registration_cannot_replace_current_lease() {
        let broker = HostShortcutBroker::default();
        broker.note_connected("host-one");
        let register = |lease: HostShortcutLease| {
            broker.process_request(HostShortcutRequest {
                request_id: "register".into(),
                client_instance: "host-one".into(),
                server_instance: "server-one".into(),
                policy_epoch: 4,
                shortcut: HostShortcutCommand::Register { lease },
            })
        };
        assert!(matches!(
            register(lease("lease-new", 2)),
            HostShortcutReply::Registered { .. }
        ));
        assert!(matches!(
            register(lease("lease-old", 1)),
            HostShortcutReply::Rejected { .. }
        ));
        let mut conflict = lease("lease-other", 2);
        conflict.target.target_id = "other-target".into();
        assert!(matches!(
            register(conflict),
            HostShortcutReply::Rejected { .. }
        ));
        assert!(matches!(
            register(lease("lease-new", 2)),
            HostShortcutReply::Registered { .. }
        ));
    }

    #[test]
    fn active_session_keeps_first_target_after_lease_refresh() {
        let broker = HostShortcutBroker::default();
        register_default_lease(&broker);
        assert_eq!(
            route_with_source(
                &broker,
                true,
                ShortcutActivation::PushToTalk,
                CurrentInputSource::Other,
            ),
            ShortcutRouting::Forwarded
        );
        let first = broker.poll("host-one", 0).unwrap();
        let mut refreshed = lease("lease-two", 2);
        refreshed.target.target_id = "target-two".into();
        assert!(matches!(
            broker.process_request(HostShortcutRequest {
                request_id: "register-2".into(),
                client_instance: "host-one".into(),
                server_instance: "server-one".into(),
                policy_epoch: 4,
                shortcut: HostShortcutCommand::Register { lease: refreshed },
            }),
            HostShortcutReply::Registered { .. }
        ));
        assert_eq!(
            route_with_source(
                &broker,
                false,
                ShortcutActivation::PushToTalk,
                CurrentInputSource::Other,
            ),
            ShortcutRouting::Forwarded
        );
        let release = broker.poll("host-one", 0).unwrap();
        assert!(!release.starts_session);
        assert_eq!(release.session_id, first.session_id);
        assert_eq!(release.target.target_id, "target-one");
        assert_eq!(release.lease_id, "lease-one");
    }

    #[test]
    fn disconnected_active_session_never_falls_back_to_legacy() {
        let broker = HostShortcutBroker::default();
        register_default_lease(&broker);
        assert_eq!(
            route_with_source(
                &broker,
                true,
                ShortcutActivation::PushToTalk,
                CurrentInputSource::Other,
            ),
            ShortcutRouting::Forwarded
        );
        let _start = broker.poll("host-one", 0).unwrap();
        broker.note_disconnected("host-one");
        assert_eq!(
            route_with_source(
                &broker,
                false,
                ShortcutActivation::PushToTalk,
                CurrentInputSource::Other,
            ),
            ShortcutRouting::HostPending
        );
        assert_eq!(
            route_with_source(
                &broker,
                true,
                ShortcutActivation::PushToTalk,
                CurrentInputSource::Other,
            ),
            ShortcutRouting::Legacy
        );
    }

    #[test]
    fn rejected_start_trigger_clears_unconsumed_active_session() {
        let broker = HostShortcutBroker::default();
        register_default_lease(&broker);
        assert_eq!(
            route_with_source(
                &broker,
                true,
                ShortcutActivation::Toggle,
                CurrentInputSource::Other,
            ),
            ShortcutRouting::Forwarded
        );
        let trigger = broker.poll("host-one", 0).unwrap();
        assert!(matches!(
            broker.process_request(HostShortcutRequest {
                request_id: "reject".into(),
                client_instance: "host-one".into(),
                server_instance: "server-one".into(),
                policy_epoch: 4,
                shortcut: HostShortcutCommand::Reject {
                    trigger_id: trigger.trigger_id
                },
            }),
            HostShortcutReply::Retired { .. }
        ));
        assert_eq!(
            route_with_source(
                &broker,
                true,
                ShortcutActivation::Toggle,
                CurrentInputSource::Other,
            ),
            ShortcutRouting::Forwarded
        );
        let next = broker.poll("host-one", 0).unwrap();
        assert!(next.starts_session);
        assert_ne!(next.session_id, trigger.session_id);
    }

    #[test]
    fn issued_trigger_can_be_consumed_once_only() {
        let broker = HostShortcutBroker::default();
        register_default_lease(&broker);
        route_with_source(
            &broker,
            true,
            ShortcutActivation::Toggle,
            CurrentInputSource::Other,
        );
        let trigger = broker.poll("host-one", 0).unwrap();
        let request = VoiceRequest {
            request_id: "voice".into(),
            session_id: trigger.session_id.clone(),
            server_instance: trigger.server_instance.clone(),
            client_instance: trigger.client_instance.clone(),
            policy_epoch: trigger.policy_epoch,
            command: VoiceCommand::HostShortcut {
                target: trigger.target.clone(),
                post_process: false,
                terms: inputia_handy_runtime::voice_protocol::VoiceTermsVersion {
                    policy_epoch: trigger.policy_epoch,
                    learning_generation: 1,
                },
                edge: inputia_handy_runtime::voice_protocol::HostShortcutEdge {
                    trigger_id: trigger.trigger_id.clone(),
                    starts_session: trigger.starts_session,
                    lease_id: trigger.lease_id.clone(),
                    lease_epoch: trigger.lease_epoch,
                    binding_id: trigger.binding_id.clone(),
                    hotkey_string: trigger.hotkey_string.clone(),
                    is_pressed: trigger.is_pressed,
                    activation: trigger.activation,
                    pressed_at_unix_ms: trigger.pressed_at_unix_ms,
                    hold_threshold_ms: trigger.hold_threshold_ms,
                },
            },
        };
        assert!(broker.consume_voice_command(&request).is_ok());
        assert!(broker.consume_voice_command(&request).is_err());
    }

    #[test]
    fn consumed_start_trigger_cannot_be_rejected_or_reopened() {
        let broker = HostShortcutBroker::default();
        register_default_lease(&broker);
        route_with_source(
            &broker,
            true,
            ShortcutActivation::Toggle,
            CurrentInputSource::Other,
        );
        let trigger = broker.poll("host-one", 0).unwrap();
        let request = VoiceRequest {
            request_id: "voice".into(),
            session_id: trigger.session_id.clone(),
            server_instance: trigger.server_instance.clone(),
            client_instance: trigger.client_instance.clone(),
            policy_epoch: trigger.policy_epoch,
            command: VoiceCommand::HostShortcut {
                target: trigger.target.clone(),
                post_process: false,
                terms: inputia_handy_runtime::voice_protocol::VoiceTermsVersion {
                    policy_epoch: trigger.policy_epoch,
                    learning_generation: 1,
                },
                edge: inputia_handy_runtime::voice_protocol::HostShortcutEdge {
                    trigger_id: trigger.trigger_id.clone(),
                    starts_session: trigger.starts_session,
                    lease_id: trigger.lease_id.clone(),
                    lease_epoch: trigger.lease_epoch,
                    binding_id: trigger.binding_id.clone(),
                    hotkey_string: trigger.hotkey_string.clone(),
                    is_pressed: trigger.is_pressed,
                    activation: trigger.activation,
                    pressed_at_unix_ms: trigger.pressed_at_unix_ms,
                    hold_threshold_ms: trigger.hold_threshold_ms,
                },
            },
        };
        assert!(broker.consume_voice_command(&request).is_ok());
        assert!(matches!(
            broker.process_request(HostShortcutRequest {
                request_id: "reject".into(),
                client_instance: "host-one".into(),
                server_instance: "server-one".into(),
                policy_epoch: 4,
                shortcut: HostShortcutCommand::Reject {
                    trigger_id: trigger.trigger_id
                },
            }),
            HostShortcutReply::Rejected { .. }
        ));
        assert_eq!(
            route_with_source(
                &broker,
                true,
                ShortcutActivation::Toggle,
                CurrentInputSource::Other,
            ),
            ShortcutRouting::Forwarded
        );
        let stop = broker.poll("host-one", 0).unwrap();
        assert!(!stop.starts_session);
        assert_eq!(stop.session_id, trigger.session_id);
    }
}
