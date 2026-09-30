use crate::actions::ACTION_MAP;
use crate::managers::audio::AudioRecordingManager;
use crate::settings::ShortcutActivation;
use inputia_handy_runtime::voice_protocol::{
    VoiceCommand, VoicePhase, VoiceRequest, VoiceSessionView, VoiceShortcutActivation,
};
use log::{debug, error, warn};
use std::collections::HashMap;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

const DEBOUNCE: Duration = Duration::from_millis(30);
const RELEASE_GRACE: Duration = Duration::from_millis(50);
const VOICE_START_QUEUE_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PttAction {
    Passthrough,
    DeferRelease,
    CancelRelease,
}

/// A key-up deferred by `RELEASE_GRACE` so a synthesized X11 auto-repeat
/// press can cancel it (#1539). When the grace elapses the hold is resolved
/// by [`CoordinatorState::finish_hold`] (recording) or
/// [`CoordinatorState::finish_pending_hold`] (press remembered while busy).
struct PendingRelease {
    binding_id: String,
    hotkey_string: String,
    deadline: Instant,
    /// When the key actually went up. The hold duration is measured to this
    /// instant, not to the grace expiry.
    released_at: Instant,
    /// Holds at least this long stop recording; shorter ones lock it on.
    /// Push-to-talk passes zero so every release stops.
    hold_threshold: Duration,
}

/// A press that arrived while the pipeline was still busy processing the
/// previous transcription. Toggle-style triggers (SIGUSR2, CLI flags, some
/// pedal setups) flip state on every edge, so dropping a busy press desyncs
/// the parity: the next edge starts a recording nobody will ever stop.
struct PendingPress {
    binding_id: String,
    hotkey_string: String,
    /// The real key-down time, so a hold that straddles the drain is still
    /// measured from when the user pressed, not from when recording began.
    pressed_at: Instant,
    /// The recording will start locked on when the pipeline drains: set from
    /// the start for toggle, and for hold-or-toggle once the key came back up
    /// within the threshold (a tap). An unlocked pending press is a key we
    /// believe is still held.
    locked: bool,
}

impl PendingPress {
    fn remembered(&self) -> Remembered {
        if self.locked {
            Remembered::Locked
        } else {
            Remembered::Held
        }
    }
}

/// What kind of press is already waiting for the pipeline to drain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Remembered {
    /// The key is still down as far as we know.
    Held,
    /// A toggle press or a classified tap: it will start a locked session.
    Locked,
}

/// Bookkeeping for the key press that started the current recording.
struct Hold {
    pressed_at: Instant,
    /// Recording outlives the key: the next press stops it, releases are
    /// ignored. Always set for toggle; set for hold-or-toggle once a release
    /// has been classified as a tap.
    locked: bool,
}

/// What to do with an input that arrives while the pipeline is busy
/// (`Stage::Processing`). `remembered` is the press for the same binding
/// already waiting for the pipeline to drain, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BusyAction {
    /// Ignore the input entirely.
    Ignore,
    /// Remember the press; start recording when the pipeline finishes.
    Remember,
    /// This press cancels a previously remembered press: two presses during
    /// one busy window net to no-op, exactly as a press stops a locked
    /// session once recording.
    Forget,
}

fn classify_busy_input(
    is_pressed: bool,
    mode: ShortcutActivation,
    remembered: Option<Remembered>,
) -> BusyAction {
    use ShortcutActivation::*;
    match (mode, is_pressed, remembered) {
        // Toggle: presses alternate remember/forget to preserve parity.
        (Toggle, true, Some(_)) => BusyAction::Forget,
        (Toggle, true, None) => BusyAction::Remember,
        // Toggle mode ignores releases.
        (Toggle, false, _) => BusyAction::Ignore,
        // Hold modes: a press while busy means the user is holding the key —
        // start as soon as the pipeline drains. A press on a queued tap stops
        // it (parity); a press while the key is already down is a repeat.
        (PushToTalk | HoldOrToggle, true, None) => BusyAction::Remember,
        (PushToTalk | HoldOrToggle, true, Some(Remembered::Locked)) => BusyAction::Forget,
        (PushToTalk | HoldOrToggle, true, Some(Remembered::Held)) => BusyAction::Ignore,
        // Releases of a held pending press are deferred by the grace window
        // before reaching here and resolved by `finish_pending_hold`; any
        // other release (no press remembered, or already locked) is noise.
        (PushToTalk | HoldOrToggle, false, _) => BusyAction::Ignore,
    }
}

/// Pipeline lifecycle.
#[derive(Debug, PartialEq, Eq)]
enum Stage {
    Idle,
    Recording(String), // binding_id
    Processing,
}

/// A keyboard/signal edge for a transcribe binding.
struct InputEvent {
    binding_id: String,
    hotkey_string: String,
    is_pressed: bool,
    mode: ShortcutActivation,
    /// Hold-or-toggle: minimum press duration that counts as a hold.
    hold_threshold: Duration,
    /// External triggers (SIGUSR2, CLI flags) rather than physical keys.
    /// They fire on every edge by design and must never be debounced —
    /// dropping one desyncs toggle parity and wedges recording on.
    external: bool,
    /// 已认证 host 回传的边沿，允许驱动它自己冻结目标的 owned 会话。
    owned: bool,
}

impl InputEvent {
    /// The hold duration at or above which a release stops recording.
    fn effective_hold_threshold(&self) -> Duration {
        match self.mode {
            ShortcutActivation::HoldOrToggle => self.hold_threshold,
            // Every release stops; toggle never defers releases at all.
            ShortcutActivation::PushToTalk | ShortcutActivation::Toggle => Duration::ZERO,
        }
    }
}

/// A side effect decided by [`CoordinatorState`]; the coordinator thread is
/// the only executor. Keeping decisions pure lets tests drive the exact
/// production transitions without a Tauri `AppHandle` or real timers.
#[derive(Debug, PartialEq, Eq)]
enum Effect {
    Start {
        binding_id: String,
        hotkey_string: String,
    },
    Stop {
        binding_id: String,
        hotkey_string: String,
    },
    Cancel,
}

/// Commands processed sequentially by the coordinator thread.
enum Command {
    AcquireSettingsIdle {
        token: u64,
        deadline: Instant,
        reply: Sender<Result<(), String>>,
    },
    ReleaseSettingsIdle {
        token: u64,
    },
    Input(InputEvent),
    PermissionInput {
        input: InputEvent,
        epoch: Result<u64, String>,
    },
    LegacyContinuation(InputEvent),
    GlobalCancel,
    ProcessingFinished,
    Voice {
        permission_epoch: Option<Result<u64, String>>,
        request: Box<VoiceRequest>,
        reply: Sender<Result<VoiceSessionView, String>>,
        deadline: Instant,
    },
    VoiceSessionView {
        session_id: String,
        reply: Sender<Result<VoiceSessionView, String>>,
    },
    VoiceResultPrepared {
        permission_epoch: Result<u64, String>,
        start: Box<VoiceRequest>,
        item_id: String,
        operation_id: String,
        reply: Sender<Result<VoiceSessionView, String>>,
    },
    RecordingRequested {
        binding_id: String,
        generation: u64,
        session_id: Option<String>,
    },
    RecordingReady {
        binding_id: String,
        generation: u64,
    },
    PreparationFailed {
        binding_id: String,
        generation: u64,
    },
    VoiceClientDisconnected {
        client_instance: String,
    },
}

/// 仅镜像归属和回执事实；录音/处理生命周期仍只由 Stage 决定。
struct OwnedVoiceSession {
    start: VoiceRequest,
    /// 会话事实版本，与全局录音生命周期计数分离。
    view_generation: u64,
    binding_id: String,
    microphone_generation: Option<u64>,
    microphone_ready: bool,
    terminal: Option<VoicePhase>,
    result: Option<(String, String)>,
}

/// 给 actions 读取冻结输出身份，不承担生命周期转换。
#[derive(Default)]
struct VoiceProjection {
    context: Mutex<Option<VoiceRequest>>,
    #[cfg(target_os = "macos")]
    platform: Mutex<Option<crate::integration_output::PlatformVoiceContext>>,
}

const MAX_VOICE_SESSIONS: usize = 1024;
const MAX_VOICE_REQUESTS: usize = 16384;

/// Decide whether a key-up should be deferred (so auto-repeat can cancel it)
/// or a key-down cancels a deferred release. `hold_to_talk` is whether a
/// release currently ends the session: true for push-to-talk and for an
/// unlocked hold-or-toggle session, false for toggle and a locked session.
/// `held_binding` is the binding whose key we believe is down — the one
/// recording, or the one remembered while the pipeline is busy.
fn classify_ptt_event(
    pending_release_binding: Option<&str>,
    is_pressed: bool,
    hold_to_talk: bool,
    binding_id: &str,
    held_binding: Option<&str>,
) -> PttAction {
    if !hold_to_talk {
        return PttAction::Passthrough;
    }

    if is_pressed {
        if pending_release_binding == Some(binding_id) {
            PttAction::CancelRelease
        } else {
            PttAction::Passthrough
        }
    } else if held_binding == Some(binding_id) && pending_release_binding.is_none() {
        PttAction::DeferRelease
    } else {
        PttAction::Passthrough
    }
}

/// Pure lifecycle state machine: owns every transition decision (release
/// grace, hold-vs-tap classification, debounce, busy-pipeline
/// remember/forget, cancel, drain). Produces [`Effect`]s instead of touching
/// the app, so unit tests exercise the real production logic.
///
/// All three activation modes run through one machine. A recording starts on
/// key-down in every mode; what differs is how it ends:
///
/// * push-to-talk — every release stops (hold threshold of zero)
/// * toggle — releases are ignored, the next press stops (locked from the start)
/// * hold-or-toggle — a release after a long hold stops; a release after a
///   short tap locks the session, and the next press stops
struct CoordinatorState {
    settings_barrier: Option<u64>,
    stage: Stage,
    hold: Option<Hold>,
    last_press: Option<Instant>,
    pending_release: Option<PendingRelease>,
    pending_press: Option<PendingPress>,
    generation: u64,
    active_voice: Option<String>,
    voice_sessions: HashMap<String, OwnedVoiceSession>,
    voice_requests: HashMap<String, (VoiceRequest, Result<VoiceSessionView, String>)>,
}

impl CoordinatorState {
    fn new() -> Self {
        Self {
            settings_barrier: None,
            stage: Stage::Idle,
            hold: None,
            last_press: None,
            pending_release: None,
            pending_press: None,
            generation: 0,
            active_voice: None,
            voice_sessions: HashMap::new(),
            voice_requests: HashMap::new(),
        }
    }

    /// Deadline of the deferred release, if any — drives `recv_timeout`.
    fn grace_deadline(&self) -> Option<Instant> {
        self.pending_release.as_ref().map(|p| p.deadline)
    }

    /// Whether the current session (recording, or remembered for the drain)
    /// outlives the key, so releases are ignored and the next press ends it.
    fn is_locked(&self) -> bool {
        self.hold.as_ref().is_some_and(|h| h.locked)
            || self.pending_press.as_ref().is_some_and(|p| p.locked)
    }

    fn on_input(&mut self, input: InputEvent, now: Instant) -> Option<Effect> {
        // 本机同 binding 的明确停止按键仍可结束 IME 发起的录音，但不能换掉
        // 已冻结的目标/输出所有者；无关 release、PTT press 和其他 binding 不参与。
        if self.active_voice.is_some() && !input.owned {
            if let Stage::Recording(binding) = &self.stage {
                if !input.is_pressed
                    || input.binding_id != *binding
                    || input.mode == ShortcutActivation::PushToTalk
                {
                    return None;
                }
            }
        }
        let pending_release_binding = self
            .pending_release
            .as_ref()
            .map(|pending| pending.binding_id.as_str());
        let held_binding = match &self.stage {
            Stage::Recording(id) => Some(id.as_str()),
            Stage::Processing => self.pending_press.as_ref().map(|p| p.binding_id.as_str()),
            Stage::Idle => None,
        };
        let hold_to_talk = input.mode != ShortcutActivation::Toggle && !self.is_locked();

        match classify_ptt_event(
            pending_release_binding,
            input.is_pressed,
            hold_to_talk,
            &input.binding_id,
            held_binding,
        ) {
            PttAction::CancelRelease => {
                self.pending_release = None;
                return None;
            }
            PttAction::DeferRelease => {
                self.pending_release = Some(PendingRelease {
                    hold_threshold: input.effective_hold_threshold(),
                    binding_id: input.binding_id,
                    hotkey_string: input.hotkey_string,
                    deadline: now + RELEASE_GRACE,
                    released_at: now,
                });
                return None;
            }
            PttAction::Passthrough => {}
        }

        // Debounce rapid-fire press events (key repeat / double-tap).
        // Releases in the hold modes are deferred above to absorb X11 auto-repeat.
        // External triggers are exempt: each one is a deliberate edge from the
        // user's own integration, and dropping it desyncs toggle parity.
        if input.is_pressed && !input.external {
            if self
                .last_press
                .is_some_and(|t| now.duration_since(t) < DEBOUNCE)
            {
                debug!("Debounced press for '{}'", input.binding_id);
                return None;
            }
            self.last_press = Some(now);
        }

        // A busy pipeline can't accept lifecycle changes now: classify the
        // input against any already-remembered press instead of dropping it
        // silently.
        if let Stage::Processing = self.stage {
            // Only one press can be remembered. Once a binding has claimed it,
            // inputs for a different binding are ignored — the same rule as a
            // different binding pressed while recording — rather than silently
            // replacing the remembered press and breaking its parity.
            if let Some(pending) = &self.pending_press {
                if pending.binding_id != input.binding_id {
                    debug!(
                        "Ignoring input for '{}': '{}' is already pending",
                        input.binding_id, pending.binding_id
                    );
                    return None;
                }
            }
            let remembered = self.pending_press.as_ref().map(|p| p.remembered());
            match classify_busy_input(input.is_pressed, input.mode, remembered) {
                BusyAction::Remember => {
                    debug!(
                        "Remembering press for '{}': pipeline busy",
                        input.binding_id
                    );
                    self.pending_press = Some(PendingPress {
                        // Toggle never ends on a release: locked from the start.
                        locked: input.mode == ShortcutActivation::Toggle,
                        binding_id: input.binding_id,
                        hotkey_string: input.hotkey_string,
                        pressed_at: now,
                    });
                }
                BusyAction::Forget => {
                    debug!("Forgetting remembered press for '{}'", input.binding_id);
                    self.pending_press = None;
                }
                BusyAction::Ignore => {
                    debug!("Ignoring input for '{}': pipeline busy", input.binding_id);
                }
            }
            return None;
        }

        if input.is_pressed {
            match &self.stage {
                Stage::Idle => {
                    // Toggle never ends on a release: locked from the start.
                    let locked = input.mode == ShortcutActivation::Toggle;
                    return Some(self.begin_recording(
                        input.binding_id,
                        input.hotkey_string,
                        now,
                        locked,
                    ));
                }
                Stage::Recording(id) if id == &input.binding_id => {
                    // A locked session ends on the next press. In toggle mode
                    // every press ends it, even if the recording began under a
                    // hold mode (the setting changed mid-recording) — otherwise
                    // nothing but Escape could stop it.
                    if self.is_locked() || input.mode == ShortcutActivation::Toggle {
                        return Some(self.begin_processing(input.binding_id, input.hotkey_string));
                    }
                    // The key is still held (its release will end this
                    // recording), so a repeated press means nothing.
                    debug!("Ignoring press for '{}': key is held", input.binding_id);
                }
                _ => debug!(
                    "Ignoring press for '{}': another binding is recording",
                    input.binding_id
                ),
            }
        } else if hold_to_talk
            && matches!(&self.stage, Stage::Recording(id) if id == &input.binding_id)
        {
            // A release that was not deferred (one is already pending for this
            // binding): resolve it immediately rather than dropping it.
            let threshold = input.effective_hold_threshold();
            return self.finish_hold(input.binding_id, input.hotkey_string, now, threshold);
        }
        None
    }

    /// The `RELEASE_GRACE` window elapsed with no cancelling press arriving:
    /// resolve the deferred release against whatever that binding's key was
    /// holding — the live recording, or a press remembered while busy.
    fn on_grace_expired(&mut self) -> Option<Effect> {
        let pending = self.pending_release.take()?;
        match &self.stage {
            Stage::Recording(id) if *id == pending.binding_id => self.finish_hold(
                pending.binding_id,
                pending.hotkey_string,
                pending.released_at,
                pending.hold_threshold,
            ),
            Stage::Processing => {
                self.finish_pending_hold(&pending);
                None
            }
            _ => None,
        }
    }

    /// A press remembered while the pipeline was busy has been released for
    /// real, still before the drain. A completed hold has nothing left to
    /// start; a tap queues a locked session so the drain starts it — the
    /// same hold-vs-tap rule as [`CoordinatorState::finish_hold`].
    fn finish_pending_hold(&mut self, release: &PendingRelease) {
        let Some(pending) = self
            .pending_press
            .as_mut()
            .filter(|p| p.binding_id == release.binding_id)
        else {
            return;
        };
        let held = release
            .released_at
            .saturating_duration_since(pending.pressed_at);
        if held < release.hold_threshold {
            debug!(
                "Tap ({held:?}) for '{}' while busy: will start locked on when the pipeline drains",
                release.binding_id
            );
            pending.locked = true;
        } else {
            debug!(
                "Forgetting remembered press for '{}': released after a {held:?} hold while busy",
                release.binding_id
            );
            self.pending_press = None;
        }
    }

    /// The key that started the current recording has been released for real.
    /// A hold at least `threshold` long stops recording; anything shorter was a
    /// tap, which locks the session on until the next press.
    fn finish_hold(
        &mut self,
        binding_id: String,
        hotkey_string: String,
        released_at: Instant,
        threshold: Duration,
    ) -> Option<Effect> {
        let held = self
            .hold
            .as_ref()
            .map(|h| released_at.saturating_duration_since(h.pressed_at))
            // No hold bookkeeping means we cannot tell a tap from a hold;
            // stopping is the safe reading (it is what push-to-talk always did).
            .unwrap_or(Duration::MAX);
        if held >= threshold {
            return Some(self.begin_processing(binding_id, hotkey_string));
        }
        if let Some(hold) = &mut self.hold {
            debug!("Tap ({held:?}) for '{binding_id}': recording locked on until the next press");
            hold.locked = true;
        }
        None
    }

    fn on_cancel(&mut self, recording_was_active: bool) {
        if let Some(session) = self
            .active_voice
            .as_ref()
            .and_then(|id| self.voice_sessions.get_mut(id))
        {
            if session.terminal.is_none() {
                session.terminal = Some(VoicePhase::Cancelled);
                session.view_generation += 1;
            }
        }
        self.pending_release = None;
        // An explicit cancel abandons any remembered start too — the user
        // asked for silence, not a deferred recording.
        self.pending_press = None;
        // Don't reset during processing — wait for the pipeline to finish.
        if !matches!(self.stage, Stage::Processing)
            && (recording_was_active || matches!(self.stage, Stage::Recording(_)))
        {
            self.stage = Stage::Idle;
            self.hold = None;
            self.active_voice = None;
        }
    }

    fn on_processing_finished(&mut self) -> Option<Effect> {
        if let Some(session) = self
            .active_voice
            .take()
            .and_then(|id| self.voice_sessions.get_mut(&id))
        {
            // FinishGuard 也在失败/展开栈时触发，不能据此声称已有结果或已上屏。
            if session.terminal.is_none() {
                session.terminal = Some(VoicePhase::Interrupted);
                session.view_generation += 1;
            }
        }
        self.stage = Stage::Idle;
        self.hold = None;
        let pending = self.pending_press.take()?;
        debug!(
            "Pipeline drained; starting remembered press for '{}'",
            pending.binding_id
        );
        Some(self.begin_recording(
            pending.binding_id,
            pending.hotkey_string,
            pending.pressed_at,
            pending.locked,
        ))
    }

    /// Reconcile the optimistic `Stage::Recording` after the executor reports
    /// whether recording actually began (microphone access can be denied).
    fn on_start_result(&mut self, binding_id: &str, started: bool) {
        if !started && matches!(&self.stage, Stage::Recording(id) if id == binding_id) {
            if let Some(session) = self
                .active_voice
                .take()
                .and_then(|id| self.voice_sessions.get_mut(&id))
            {
                session.terminal = Some(VoicePhase::Failed);
                session.view_generation += 1;
            }
            self.stage = Stage::Idle;
            self.hold = None;
        }
    }

    /// Optimistic transition to `Recording`; rolled back via
    /// [`CoordinatorState::on_start_result`] if the effect fails to start
    /// recording for real.
    fn begin_recording(
        &mut self,
        binding_id: String,
        hotkey_string: String,
        pressed_at: Instant,
        locked: bool,
    ) -> Effect {
        self.generation = self.generation.saturating_add(1);
        self.stage = Stage::Recording(binding_id.clone());
        self.hold = Some(Hold { pressed_at, locked });
        Effect::Start {
            binding_id,
            hotkey_string,
        }
    }

    fn begin_processing(&mut self, binding_id: String, hotkey_string: String) -> Effect {
        if matches!(self.stage, Stage::Recording(_)) {
            if let Some(session) = self
                .active_voice
                .as_ref()
                .and_then(|id| self.voice_sessions.get_mut(id))
            {
                if session.terminal.is_none() {
                    session.view_generation += 1;
                }
            }
        }
        self.stage = Stage::Processing;
        self.hold = None;
        Effect::Stop {
            binding_id,
            hotkey_string,
        }
    }

    fn voice_context(&self) -> Option<VoiceRequest> {
        self.active_voice
            .as_ref()
            .and_then(|id| self.voice_sessions.get(id))
            .filter(|session| session.terminal.is_none())
            .map(|session| session.start.clone())
    }

    fn voice_view(&self, session_id: &str) -> Result<VoiceSessionView, String> {
        let session = self.voice_sessions.get(session_id).ok_or("未知语音会话")?;
        let phase = session.terminal.unwrap_or_else(|| match &self.stage {
            Stage::Recording(id) if id == &session.binding_id => {
                if session.microphone_ready {
                    VoicePhase::Recording
                } else {
                    VoicePhase::Preparing
                }
            }
            Stage::Processing if self.active_voice.as_deref() == Some(session_id) => {
                VoicePhase::Processing
            }
            _ => VoicePhase::Interrupted,
        });
        let target_id = match &session.start.command {
            VoiceCommand::Start { target, .. } | VoiceCommand::HostShortcut { target, .. } => {
                Some(target.target_id.clone())
            }
            _ => None,
        };
        Ok(VoiceSessionView {
            session_id: session_id.into(),
            generation: session.view_generation,
            phase,
            target_id,
            item_id: session.result.as_ref().map(|result| result.0.clone()),
            output_operation_id: session.result.as_ref().map(|result| result.1.clone()),
        })
    }

    fn on_voice_result_prepared(
        &mut self,
        start: &VoiceRequest,
        item_id: String,
        operation_id: String,
    ) -> Result<VoiceSessionView, String> {
        let session = self
            .voice_sessions
            .get_mut(&start.session_id)
            .ok_or("未知语音会话")?;
        if &session.start != start
            || item_id.is_empty()
            || item_id.len() > 1024
            || operation_id.is_empty()
            || operation_id.len() > 256
            || item_id.chars().any(char::is_control)
            || operation_id.chars().any(char::is_control)
        {
            return Err("语音结果身份无效".into());
        }
        let result = (item_id, operation_id);
        if let Some(previous) = &session.result {
            if previous != &result {
                return Err("语音结果不能替换已有输出".into());
            }
            return self.voice_view(&start.session_id);
        }
        if session.terminal.is_some()
            || self.active_voice.as_deref() != Some(&start.session_id)
            || !matches!(self.stage, Stage::Processing)
        {
            return Err("语音会话已关闭或尚未处理".into());
        }
        session.result = Some(result);
        session.terminal = Some(VoicePhase::PendingTarget);
        session.view_generation += 1;
        self.voice_view(&start.session_id)
    }

    #[cfg(test)]
    fn on_voice(
        &mut self,
        request: VoiceRequest,
        now: Instant,
    ) -> (Result<VoiceSessionView, String>, Option<Effect>) {
        self.on_voice_before_deadline(request, now + VOICE_START_QUEUE_TIMEOUT, now)
    }

    fn on_voice_before_deadline(
        &mut self,
        request: VoiceRequest,
        deadline: Instant,
        now: Instant,
    ) -> (Result<VoiceSessionView, String>, Option<Effect>) {
        if let Some((original, result)) = self.voice_requests.get(&request.request_id) {
            if original != &request {
                return (Err("语音请求 ID 与原请求冲突".into()), None);
            }
            return (
                match result {
                    Ok(_) => self.voice_view(&request.session_id),
                    Err(error) => Err(error.clone()),
                },
                None,
            );
        }
        // 达到容量不驱逐旧身份；Status 不产生副作用，Stop/Cancel 仍须能关闭采集。
        let at_capacity = self.voice_requests.len() >= MAX_VOICE_REQUESTS;
        if at_capacity && request.strict_start_identity().is_some() {
            return (Err("语音请求账本已满；拒绝新会话".into()), None);
        }
        let (result, effect) = if request.strict_start_identity().is_some() && now >= deadline {
            (Err("语音 Start 排队已超时，未开始录音".into()), None)
        } else {
            self.apply_voice(&request, now)
        };
        // 状态轮询只读，不占用去重容量。满容量时仍允许已授权 Stop/Cancel
        // 关闭采集，但依赖会话的幂等事实而不再分配新回执；既有 ID 从不驱逐。
        if !at_capacity && !matches!(request.command, VoiceCommand::Status) {
            self.voice_requests
                .insert(request.request_id.clone(), (request, result.clone()));
        }
        (result, effect)
    }

    fn acknowledge_start_before_effect(
        &mut self,
        binding_id: &str,
        result: Result<VoiceSessionView, String>,
        reply: Sender<Result<VoiceSessionView, String>>,
    ) -> bool {
        // Preparing 是“请求被协调器接纳”，不是 microphone-ready。接收方已经
        // 放弃时不产生迟到的录音；此边界之后的断线由持久 ledger 保守处理。
        if reply.send(result).is_err() {
            self.on_start_result(binding_id, false);
            false
        } else {
            true
        }
    }

    fn apply_voice(
        &mut self,
        request: &VoiceRequest,
        now: Instant,
    ) -> (Result<VoiceSessionView, String>, Option<Effect>) {
        if let Some(session) = self.voice_sessions.get(&request.session_id) {
            if session.start.client_instance != request.client_instance
                || session.start.server_instance != request.server_instance
            {
                return (Err("语音会话不属于此连接实例".into()), None);
            }
            if matches!(
                request.command,
                VoiceCommand::Start { .. } | VoiceCommand::HostShortcut { .. }
            ) {
                if session.start.command != request.command
                    || session.start.policy_epoch != request.policy_epoch
                {
                    if let (
                        VoiceCommand::HostShortcut {
                            target,
                            post_process,
                            terms,
                            ..
                        },
                        VoiceCommand::HostShortcut {
                            target: original_target,
                            post_process: original_post_process,
                            terms: original_terms,
                            ..
                        },
                    ) = (&request.command, &session.start.command)
                    {
                        if target == original_target
                            && post_process == original_post_process
                            && terms == original_terms
                            && request.policy_epoch == session.start.policy_epoch
                        {
                            return self.apply_host_shortcut(request, now);
                        }
                    }
                    return (
                        Err("重复 Start 的目标、选项或策略与原会话冲突".into()),
                        None,
                    );
                }
                return (self.voice_view(&request.session_id), None);
            }
        } else if !matches!(
            request.command,
            VoiceCommand::Start { .. } | VoiceCommand::HostShortcut { .. }
        ) {
            return (Err("未知语音会话".into()), None);
        }
        let effect = match &request.command {
            VoiceCommand::Start { post_process, .. } => {
                if self.stage != Stage::Idle || self.pending_press.is_some() {
                    return (Err("录音或处理流水线忙，不能开始新语音会话".into()), None);
                }
                if self.voice_sessions.len() >= MAX_VOICE_SESSIONS || self.generation == u64::MAX {
                    return (Err("语音会话账本已满；拒绝新会话".into()), None);
                }
                let binding_id = if *post_process {
                    "transcribe_with_post_process"
                } else {
                    "transcribe"
                }
                .to_owned();
                let effect =
                    self.begin_recording(binding_id.clone(), "inputia-session".into(), now, true);
                self.voice_sessions.insert(
                    request.session_id.clone(),
                    OwnedVoiceSession {
                        start: request.clone(),
                        view_generation: 1,
                        binding_id,
                        microphone_generation: None,
                        microphone_ready: false,
                        terminal: None,
                        result: None,
                    },
                );
                self.active_voice = Some(request.session_id.clone());
                Some(effect)
            }
            VoiceCommand::HostShortcut { .. } => return self.apply_host_shortcut(request, now),
            VoiceCommand::Stop => {
                if self.active_voice.as_deref() == Some(&request.session_id) {
                    if let Stage::Recording(binding) = &self.stage {
                        Some(self.begin_processing(binding.clone(), "inputia-session".into()))
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            VoiceCommand::Cancel => {
                if let Some(session) = self.voice_sessions.get_mut(&request.session_id) {
                    if session.terminal == Some(VoicePhase::PendingTarget) {
                        // dispatcher已原子拒绝尚未claim输出；不清理下一录音。
                        session.terminal = Some(VoicePhase::Cancelled);
                        session.view_generation += 1;
                        return (self.voice_view(&request.session_id), None);
                    }
                }
                let active = self.active_voice.as_deref() == Some(&request.session_id)
                    && self
                        .voice_sessions
                        .get(&request.session_id)
                        .is_some_and(|session| session.terminal.is_none());
                if active {
                    self.on_cancel(matches!(self.stage, Stage::Recording(_)));
                    Some(Effect::Cancel)
                } else {
                    None
                }
            }
            VoiceCommand::Status => None,
        };
        (self.voice_view(&request.session_id), effect)
    }

    fn apply_host_shortcut(
        &mut self,
        request: &VoiceRequest,
        now: Instant,
    ) -> (Result<VoiceSessionView, String>, Option<Effect>) {
        let VoiceCommand::HostShortcut {
            post_process, edge, ..
        } = &request.command
        else {
            return (Err("无效 host 快捷键请求".into()), None);
        };
        let existed = self.voice_sessions.contains_key(&request.session_id);
        if !existed && !edge.is_pressed {
            return (Err("未知语音会话".into()), None);
        }
        if !is_transcribe_binding(&edge.binding_id) {
            return (Err("无效语音快捷键绑定".into()), None);
        }
        if !existed
            && (self.stage != Stage::Idle
                || self.pending_press.is_some()
                || self.voice_sessions.len() >= MAX_VOICE_SESSIONS
                || self.generation == u64::MAX)
        {
            return (Err("录音或处理流水线忙，不能开始新语音会话".into()), None);
        }
        if existed
            && self.active_voice.as_deref() != Some(&request.session_id)
            && !matches!(self.stage, Stage::Processing)
        {
            return (Err("语音会话已关闭或不再是当前会话".into()), None);
        }
        let binding_id = if *post_process {
            "transcribe_with_post_process"
        } else {
            "transcribe"
        };
        if edge.binding_id != binding_id {
            return (Err("host 快捷键绑定与转写模式不一致".into()), None);
        }
        let effect = self.on_input(
            InputEvent {
                binding_id: edge.binding_id.clone(),
                hotkey_string: edge.hotkey_string.clone(),
                is_pressed: edge.is_pressed,
                mode: shortcut_activation(edge.activation),
                hold_threshold: Duration::from_millis(edge.hold_threshold_ms),
                external: false,
                owned: true,
            },
            now,
        );
        if !existed && matches!(effect, Some(Effect::Start { .. })) {
            self.voice_sessions.insert(
                request.session_id.clone(),
                OwnedVoiceSession {
                    start: request.clone(),
                    view_generation: 1,
                    binding_id: binding_id.to_owned(),
                    microphone_generation: None,
                    microphone_ready: false,
                    terminal: None,
                    result: None,
                },
            );
            self.active_voice = Some(request.session_id.clone());
        }
        (self.voice_view(&request.session_id), effect)
    }

    fn on_recording_requested(
        &mut self,
        binding_id: &str,
        generation: u64,
        session_id: Option<&str>,
    ) {
        if self.active_voice.as_deref() != session_id
            || !matches!(&self.stage, Stage::Recording(id) if id == binding_id)
        {
            return;
        }
        if let Some(session) = session_id.and_then(|id| self.voice_sessions.get_mut(id)) {
            if session.terminal.is_none() && session.microphone_generation.is_none() {
                session.microphone_generation = Some(generation);
            }
        }
    }

    fn on_recording_ready(&mut self, binding_id: &str, generation: u64) {
        if !matches!(&self.stage, Stage::Recording(id) if id == binding_id) {
            return;
        }
        if let Some(session) = self
            .active_voice
            .as_ref()
            .and_then(|id| self.voice_sessions.get_mut(id))
        {
            if session.terminal.is_none()
                && !session.microphone_ready
                && session.microphone_generation == Some(generation)
            {
                session.microphone_ready = true;
                session.view_generation += 1;
            }
        }
    }

    fn on_preparation_failed(&mut self, binding_id: &str, generation: u64) -> Option<Effect> {
        if !matches!(&self.stage, Stage::Recording(id) if id == binding_id) {
            return None;
        }
        let current_preparing = self
            .active_voice
            .as_ref()
            .and_then(|id| self.voice_sessions.get(id))
            .is_some_and(|session| {
                session.binding_id == binding_id
                    && session.terminal.is_none()
                    && !session.microphone_ready
                    && session.microphone_generation == Some(generation)
            });
        if !current_preparing {
            return None;
        }
        // 已知没有进入 ready；保留 Failed，而不是把清理副作用误记为用户取消。
        // 清理前清除 active_voice；raw cleanup 不会再发迟到的取消通知。
        self.on_start_result(binding_id, false);
        Some(Effect::Cancel)
    }

    fn on_voice_client_disconnected(&mut self, client_instance: &str) -> Option<Effect> {
        let owned_by_client = self
            .active_voice
            .as_ref()
            .and_then(|id| self.voice_sessions.get(id))
            .is_some_and(|session| {
                session.terminal.is_none() && session.start.client_instance == client_instance
            });
        if !owned_by_client {
            return None;
        }
        let recording_was_active = matches!(self.stage, Stage::Recording(_));
        self.on_cancel(recording_was_active);
        Some(Effect::Cancel)
    }
}

/// Serialises all transcription lifecycle events through a single thread
/// to eliminate race conditions between keyboard shortcuts, signals, and
/// the async transcribe-paste pipeline. The thread is a thin shell: it
/// transports commands to the pure [`CoordinatorState`] and executes the
/// returned [`Effect`]s.
pub struct TranscriptionCoordinator {
    tx: Sender<Command>,
    voice_projection: Arc<VoiceProjection>,
}

pub fn is_transcribe_binding(id: &str) -> bool {
    id == "transcribe" || id == "transcribe_with_post_process"
}

fn shortcut_activation(value: VoiceShortcutActivation) -> ShortcutActivation {
    match value {
        VoiceShortcutActivation::Toggle => ShortcutActivation::Toggle,
        VoiceShortcutActivation::PushToTalk => ShortcutActivation::PushToTalk,
        VoiceShortcutActivation::HoldOrToggle => ShortcutActivation::HoldOrToggle,
    }
}

pub(crate) struct SettingsIdleGuard {
    tx: Sender<Command>,
    token: u64,
}
impl Drop for SettingsIdleGuard {
    fn drop(&mut self) {
        let _ = self
            .tx
            .send(Command::ReleaseSettingsIdle { token: self.token });
    }
}

impl TranscriptionCoordinator {
    pub(crate) fn acquire_settings_idle(&self) -> Result<SettingsIdleGuard, String> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let token = NEXT
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_add(1))
            .map_err(|_| "shortcut_barrier_exhausted")?;
        let guard = SettingsIdleGuard {
            tx: self.tx.clone(),
            token,
        };
        let (reply, received) = mpsc::channel();
        self.tx
            .send(Command::AcquireSettingsIdle {
                token,
                deadline: Instant::now() + Duration::from_millis(750),
                reply,
            })
            .map_err(|_| "shortcut_coordinator_unavailable")?;
        received
            .recv_timeout(Duration::from_millis(800))
            .map_err(|_| "shortcut_idle_unconfirmed")??;
        Ok(guard)
    }

    pub fn new(app: AppHandle) -> Self {
        let (tx, rx) = mpsc::channel();
        let voice_projection = Arc::new(VoiceProjection::default());
        let projection = voice_projection.clone();

        thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut state = CoordinatorState::new();

                loop {
                    let cmd = if let Some(deadline) = state.grace_deadline() {
                        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                            Ok(cmd) => cmd,
                            Err(mpsc::RecvTimeoutError::Timeout) => {
                                if let Some(effect) = state.on_grace_expired() {
                                    run_effect(&app, &mut state, effect, &projection);
                                }
                                continue;
                            }
                            Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        }
                    } else {
                        match rx.recv() {
                            Ok(cmd) => cmd,
                            Err(_) => break,
                        }
                    };

                    dispatch_command(&mut state, cmd, Instant::now(), &mut |state, effect| {
                        run_effect(&app, state, effect, &projection);
                    });
                    publish_projection(&state, &projection);
                }
                debug!("Transcription coordinator exited");
            }));
            if let Err(e) = result {
                error!("Transcription coordinator panicked: {e:?}");
            }
        });

        Self {
            tx,
            voice_projection,
        }
    }

    /// 仅入队，Start 排队上限 15 秒；调用方在后台等待回执。
    /// 首次 Start 的 Preparing 只确认接纳准备，不代表麦克风开始录音；后续
    /// 失败/硬件 ready 必须通过会话视图查询，不能把初始回执当作录音成功。
    /// 认证/profile/策略校验由服务入口完成。
    pub fn control_voice(
        &self,
        request: VoiceRequest,
    ) -> mpsc::Receiver<Result<VoiceSessionView, String>> {
        let (reply, receiver) = mpsc::channel();
        let starts = request.strict_start_identity().is_some();
        let _settings_lease = starts.then(|| {
            crate::shortcut::settings_barrier::capture(true)
                .and_then(|generation| crate::shortcut::settings_barrier::admit(generation, true))
        });
        if matches!(_settings_lease, Some(None)) {
            let _ = reply.send(Err("shortcut_settings_busy".into()));
            return receiver;
        }
        if let Err(error) = self.tx.send(Command::Voice {
            permission_epoch: request
                .strict_start_identity()
                .map(|_| crate::input_permission::capture_epoch()),
            request: Box::new(request),
            reply,
            deadline: Instant::now() + VOICE_START_QUEUE_TIMEOUT,
        }) {
            if let Command::Voice { reply, .. } = error.0 {
                let _ = reply.send(Err("语音协调器已经退出".into()));
            }
        }
        receiver
    }

    /// 内部只读查询，不重放 Start，也不创建请求回执。外部调用须先验证会话归属。
    pub fn voice_session_view(
        &self,
        session_id: &str,
    ) -> mpsc::Receiver<Result<VoiceSessionView, String>> {
        let (reply, receiver) = mpsc::channel();
        if let Err(error) = self.tx.send(Command::VoiceSessionView {
            session_id: session_id.into(),
            reply,
        }) {
            if let Command::VoiceSessionView { reply, .. } = error.0 {
                let _ = reply.send(Err("语音协调器已经退出".into()));
            }
        }
        receiver
    }

    /// 仅本地转写保存/持久输出准备完成后调用；不代表已派发或已插入。
    pub fn notify_voice_result_prepared(
        &self,
        start: VoiceRequest,
        item_id: String,
        operation_id: String,
    ) -> mpsc::Receiver<Result<VoiceSessionView, String>> {
        let (reply, receiver) = mpsc::channel();
        if let Err(error) = self.tx.send(Command::VoiceResultPrepared {
            permission_epoch: crate::input_permission::capture_epoch(),
            start: Box::new(start),
            item_id,
            operation_id,
            reply,
        }) {
            if let Command::VoiceResultPrepared { reply, .. } = error.0 {
                let _ = reply.send(Err("语音协调器已经退出".into()));
            }
        }
        receiver
    }

    /// actions 在启动录音请求返回 generation 后调用；冻结请求所属会话。
    pub fn notify_recording_requested(&self, binding_id: &str, generation: u64) {
        let session_id = self
            .voice_output_context()
            .map(|request| request.session_id);
        let _ = self.tx.send(Command::RecordingRequested {
            binding_id: binding_id.into(),
            generation,
            session_id,
        });
    }

    /// 仅真实麦克风 ready 且 generation 仍有效时调用，不以启动进程成功代替。
    pub fn notify_recording_ready(&self, binding_id: &str, generation: u64) {
        let _ = self.tx.send(Command::RecordingReady {
            binding_id: binding_id.into(),
            generation,
        });
    }

    /// owned 会话等待首帧失败/超时；仅当前尚未 ready 的麦克风 generation 可清理。
    pub fn notify_preparation_failed(&self, binding_id: &str, microphone_generation: u64) {
        let _ = self.tx.send(Command::PreparationFailed {
            binding_id: binding_id.into(),
            generation: microphone_generation,
        });
    }

    /// 只读冻结的输出归属元数据，不等待录音/转写完成。
    pub fn voice_output_context(&self) -> Option<VoiceRequest> {
        self.voice_projection
            .context
            .lock()
            .ok()
            .and_then(|value| value.clone())
    }

    /// 只移动一次 Start 时冻结的原字段；异步输出拥有租约，不会影响下一次录音。
    #[cfg(target_os = "macos")]
    pub(crate) fn take_platform_output_context(
        &self,
    ) -> Option<crate::integration_output::PlatformVoiceContext> {
        self.voice_projection
            .platform
            .lock()
            .ok()
            .and_then(|mut value| value.take())
    }

    /// Send a keyboard input event for a transcribe binding. `hold_threshold`
    /// only matters for [`ShortcutActivation::HoldOrToggle`].
    pub fn send_input(
        &self,
        binding_id: &str,
        hotkey_string: &str,
        is_pressed: bool,
        mode: ShortcutActivation,
        hold_threshold: Duration,
    ) {
        self.send(
            binding_id,
            hotkey_string,
            is_pressed,
            mode,
            hold_threshold,
            false,
        );
    }

    /// Send an external trigger (SIGUSR2, CLI flag). Always a toggle press,
    /// always exempt from debounce — see [`InputEvent::external`].
    pub fn send_external_input(&self, binding_id: &str, source: &str) {
        self.send(
            binding_id,
            source,
            true,
            ShortcutActivation::Toggle,
            Duration::ZERO,
            true,
        );
    }

    fn send(
        &self,
        binding_id: &str,
        hotkey_string: &str,
        is_pressed: bool,
        mode: ShortcutActivation,
        hold_threshold: Duration,
        external: bool,
    ) {
        let Some(_settings_lease) = crate::shortcut::settings_barrier::capture(is_pressed)
            .and_then(|generation| {
                crate::shortcut::settings_barrier::admit(generation, is_pressed)
            })
        else {
            return;
        };
        if self
            .tx
            .send(Command::PermissionInput {
                epoch: crate::input_permission::capture_epoch(),
                input: InputEvent {
                    binding_id: binding_id.to_string(),
                    hotkey_string: hotkey_string.to_string(),
                    is_pressed,
                    mode,
                    hold_threshold,
                    external,
                    owned: false,
                },
            })
            .is_err()
        {
            warn!("Transcription coordinator channel closed");
        }
    }

    /// 只允许继续已存在的普通录音，不能在策略不可用时创建新录音。
    pub fn send_legacy_continuation(
        &self,
        binding_id: &str,
        hotkey_string: &str,
        is_pressed: bool,
        mode: ShortcutActivation,
        hold_threshold: Duration,
    ) {
        let _ = self.tx.send(Command::LegacyContinuation(InputEvent {
            binding_id: binding_id.into(),
            hotkey_string: hotkey_string.into(),
            is_pressed,
            mode,
            hold_threshold,
            external: false,
            owned: false,
        }));
    }

    /// 全局用户取消按队列顺序处理当前生命周期和 pending press；不先清理再通知。
    /// false 表示通道确实已关闭，调用方无需等待，可执行无 worker 紧急清理。
    pub fn request_cancel(&self) -> bool {
        self.tx.send(Command::GlobalCancel).is_ok()
    }

    pub fn notify_processing_finished(&self) {
        if self.tx.send(Command::ProcessingFinished).is_err() {
            warn!("Transcription coordinator channel closed");
        }
    }

    pub fn notify_voice_client_disconnected(&self, client_instance: &str) {
        if self
            .tx
            .send(Command::VoiceClientDisconnected {
                client_instance: client_instance.to_owned(),
            })
            .is_err()
        {
            warn!("Transcription coordinator channel closed");
        }
    }
}

fn publish_projection(state: &CoordinatorState, projection: &VoiceProjection) {
    if let Ok(mut context) = projection.context.lock() {
        *context = state.voice_context();
    }
}

/// 生产与时序回归共用同一分派入口；executor 返回前不处理下一条 Start/Finish。
fn dispatch_command(
    state: &mut CoordinatorState,
    command: Command,
    now: Instant,
    execute: &mut impl FnMut(&mut CoordinatorState, Effect),
) {
    match command {
        Command::AcquireSettingsIdle {
            token,
            deadline,
            reply,
        } => {
            if now >= deadline
                || state.settings_barrier.is_some()
                || !matches!(state.stage, Stage::Idle)
                || state.hold.is_some()
                || state.pending_release.is_some()
                || state.pending_press.is_some()
                || state.active_voice.is_some()
            {
                let _ = reply.send(Err("shortcut_gesture_busy".into()));
            } else {
                state.settings_barrier = Some(token);
                if reply.send(Ok(())).is_err() {
                    state.settings_barrier = None;
                }
            }
        }
        Command::ReleaseSettingsIdle { token } => {
            if state.settings_barrier == Some(token) {
                state.settings_barrier = None;
            }
        }
        Command::PermissionInput { input, epoch } => {
            if state.settings_barrier.is_some() {
                return;
            }
            if epoch.and_then(crate::input_permission::check_epoch).is_ok() {
                if let Some(effect) = state.on_input(input, now) {
                    execute(state, effect);
                }
            }
        }
        Command::Input(input) => {
            if state.settings_barrier.is_some() {
                return;
            }
            if let Some(effect) = state.on_input(input, now) {
                execute(state, effect);
            }
        }
        Command::LegacyContinuation(input) => {
            if state.active_voice.is_none()
                && matches!(&state.stage, Stage::Recording(id) if id == &input.binding_id)
            {
                if let Some(effect) = state.on_input(input, now) {
                    execute(state, effect);
                }
            }
        }
        Command::GlobalCancel => {
            state.on_cancel(matches!(state.stage, Stage::Recording(_)));
            execute(state, Effect::Cancel);
        }
        Command::ProcessingFinished => {
            if let Some(effect) = state.on_processing_finished() {
                execute(state, effect);
            }
        }
        Command::Voice {
            permission_epoch,
            request,
            reply,
            deadline,
        } => {
            if state.settings_barrier.is_some() && request.strict_start_identity().is_some() {
                let _ = reply.send(Err("shortcut_settings_busy".into()));
                return;
            }
            if let Some(epoch) = permission_epoch {
                if let Err(error) = epoch.and_then(crate::input_permission::check_epoch) {
                    let _ = reply.send(Err(error));
                    return;
                }
            }
            let session_id = request.session_id.clone();
            let (result, effect) = state.on_voice_before_deadline(*request, deadline, now);
            if let Some(Effect::Start { ref binding_id, .. }) = effect {
                if state.acknowledge_start_before_effect(binding_id, result, reply) {
                    if let Some(effect) = effect {
                        execute(state, effect);
                    }
                }
            } else {
                if let Some(effect) = effect {
                    execute(state, effect);
                }
                let _ = reply.send(result.and_then(|_| state.voice_view(&session_id)));
            }
        }
        Command::VoiceSessionView { session_id, reply } => {
            let _ = reply.send(state.voice_view(&session_id));
        }
        Command::VoiceResultPrepared {
            permission_epoch,
            start,
            item_id,
            operation_id,
            reply,
        } => {
            if let Err(error) = permission_epoch.and_then(crate::input_permission::check_epoch) {
                let _ = reply.send(Err(error));
                return;
            }
            let _ = reply.send(state.on_voice_result_prepared(&start, item_id, operation_id));
        }
        Command::RecordingRequested {
            binding_id,
            generation,
            session_id,
        } => state.on_recording_requested(&binding_id, generation, session_id.as_deref()),
        Command::RecordingReady {
            binding_id,
            generation,
        } => state.on_recording_ready(&binding_id, generation),
        Command::PreparationFailed {
            binding_id,
            generation,
        } => {
            if let Some(effect) = state.on_preparation_failed(&binding_id, generation) {
                execute(state, effect);
            }
        }
        Command::VoiceClientDisconnected { client_instance } => {
            if let Some(effect) = state.on_voice_client_disconnected(&client_instance) {
                execute(state, effect);
            }
        }
    }
}

fn run_effect(
    app: &AppHandle,
    state: &mut CoordinatorState,
    effect: Effect,
    projection: &VoiceProjection,
) {
    // action.start/stop 会同步读取此镜像，必须先于副作用发布当前归属。
    publish_projection(state, projection);
    match effect {
        Effect::Start {
            binding_id,
            hotkey_string,
        } => {
            #[cfg(target_os = "macos")]
            if let Ok(mut platform) = projection.platform.lock() {
                *platform = if state.active_voice.is_none() {
                    Some(crate::integration_output::PlatformVoiceContext::capture(
                        app,
                    ))
                } else {
                    None
                };
            }
            let started = start(app, &binding_id, &hotkey_string);
            #[cfg(target_os = "macos")]
            if !started {
                if let Ok(mut platform) = projection.platform.lock() {
                    platform.take();
                }
            }
            state.on_start_result(&binding_id, started);
        }
        Effect::Stop {
            binding_id,
            hotkey_string,
        } => stop(app, &binding_id, &hotkey_string),
        Effect::Cancel => {
            #[cfg(target_os = "macos")]
            if let Ok(mut platform) = projection.platform.lock() {
                platform.take();
            }
            crate::utils::cancel_current_operation_raw(app);
        }
    }
    publish_projection(state, projection);
}

/// Execute a start effect; returns whether recording actually began, so the
/// state machine can roll back its optimistic transition on failure.
fn start(app: &AppHandle, binding_id: &str, hotkey_string: &str) -> bool {
    let Some(action) = ACTION_MAP.get(binding_id) else {
        warn!("No action in ACTION_MAP for '{binding_id}'");
        return false;
    };
    action.start(app, binding_id, hotkey_string);
    let recording = app
        .try_state::<Arc<AudioRecordingManager>>()
        .is_some_and(|a| a.is_recording());
    if !recording {
        debug!("Start for '{binding_id}' did not begin recording; staying idle");
    }
    recording
}

fn stop(app: &AppHandle, binding_id: &str, hotkey_string: &str) {
    let Some(action) = ACTION_MAP.get(binding_id) else {
        warn!("No action in ACTION_MAP for '{binding_id}'");
        return;
    };
    action.stop(app, binding_id, hotkey_string);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_barrier_refuses_active_and_queued_gestures_and_blocks_new_starts() {
        let now = Instant::now();
        let acquire = |state: &mut CoordinatorState, token| {
            let (reply, received) = mpsc::channel();
            dispatch_command(
                state,
                Command::AcquireSettingsIdle {
                    token,
                    deadline: now + Duration::from_secs(1),
                    reply,
                },
                now,
                &mut |_, _| panic!("barrier must not execute input"),
            );
            received.recv().unwrap()
        };
        let mut state = CoordinatorState::new();
        state.stage = Stage::Processing;
        assert!(acquire(&mut state, 1).is_err());
        state.stage = Stage::Idle;
        state.pending_press = Some(PendingPress {
            binding_id: "transcribe".into(),
            hotkey_string: "F8".into(),
            pressed_at: now,
            locked: false,
        });
        assert!(acquire(&mut state, 2).is_err());
        state.pending_press = None;
        acquire(&mut state, 3).unwrap();
        dispatch_command(
            &mut state,
            Command::Input(InputEvent {
                binding_id: "transcribe".into(),
                hotkey_string: "F8".into(),
                is_pressed: true,
                mode: ShortcutActivation::PushToTalk,
                hold_threshold: Duration::ZERO,
                external: false,
                owned: false,
            }),
            now,
            &mut |_, _| panic!("new Start crossed barrier"),
        );
        assert!(matches!(state.stage, Stage::Idle));
        dispatch_command(
            &mut state,
            Command::ReleaseSettingsIdle { token: 2 },
            now,
            &mut |_, _| {},
        );
        assert_eq!(state.settings_barrier, Some(3));
        dispatch_command(
            &mut state,
            Command::ReleaseSettingsIdle { token: 3 },
            now,
            &mut |_, _| {},
        );
        assert_eq!(state.settings_barrier, None);
    }

    #[test]
    fn rejected_settings_barrier_preserves_active_ptt_release() {
        let now = Instant::now();
        let mut state = CoordinatorState::new();
        let mut effects = vec![];
        dispatch_command(
            &mut state,
            Command::Input(input(ShortcutActivation::PushToTalk, true)),
            now,
            &mut |_, effect| effects.push(effect),
        );
        assert!(matches!(effects.pop(), Some(Effect::Start { .. })));
        let (reply, received) = mpsc::channel();
        dispatch_command(
            &mut state,
            Command::AcquireSettingsIdle {
                token: 1,
                deadline: now + Duration::from_secs(1),
                reply,
            },
            now,
            &mut |_, _| panic!("must not change active input"),
        );
        assert!(received.recv().unwrap().is_err());
        dispatch_command(
            &mut state,
            Command::Input(input(ShortcutActivation::PushToTalk, false)),
            now + Duration::from_millis(300),
            &mut |_, effect| effects.push(effect),
        );
        effects.extend(state.on_grace_expired());
        assert!(matches!(effects.pop(), Some(Effect::Stop { .. })));
    }

    #[test]
    fn policy_failure_continuation_cannot_start_but_stops_existing_legacy_recording() {
        for mode in [ShortcutActivation::Toggle, ShortcutActivation::PushToTalk] {
            let mut state = CoordinatorState::new();
            let now = Instant::now();
            let mut effects = Vec::new();
            dispatch_command(
                &mut state,
                Command::LegacyContinuation(input(mode, true)),
                now,
                &mut |_, effect| effects.push(effect),
            );
            assert!(effects.is_empty());
            assert_eq!(state.stage, Stage::Idle);
            dispatch_command(
                &mut state,
                Command::Input(input(mode, true)),
                now,
                &mut |_, effect| effects.push(effect),
            );
            assert!(matches!(effects.pop(), Some(Effect::Start { .. })));
            dispatch_command(
                &mut state,
                Command::LegacyContinuation(input(mode, mode == ShortcutActivation::Toggle)),
                now + Duration::from_secs(1),
                &mut |_, effect| effects.push(effect),
            );
            if mode == ShortcutActivation::PushToTalk {
                assert!(effects.is_empty());
                assert!(state.grace_deadline().is_some());
                effects.extend(state.on_grace_expired());
            }
            assert!(matches!(effects.pop(), Some(Effect::Stop { .. })));
            dispatch_command(
                &mut state,
                Command::LegacyContinuation(input(mode, true)),
                now + Duration::from_secs(2),
                &mut |_, effect| effects.push(effect),
            );
            assert!(effects.is_empty());
        }
    }

    fn voice_start(session_id: &str, request_id: &str) -> VoiceRequest {
        use inputia_handy_runtime::voice_protocol::{HostTargetToken, VoiceTermsVersion};
        VoiceRequest {
            request_id: request_id.into(),
            session_id: session_id.into(),
            server_instance: "server-one".into(),
            client_instance: "host-one".into(),
            policy_epoch: 4,
            command: VoiceCommand::Start {
                target: HostTargetToken {
                    target_id: "field-token".into(),
                    host_instance: "host-one".into(),
                    controller_id: "controller-one".into(),
                    activation_generation: 1,
                    field_id: Some("field-one".into()),
                    selection_generation: 1,
                    composition_generation: 1,
                    source_app: Some("synthetic.editor".into()),
                },
                post_process: false,
                terms: VoiceTermsVersion {
                    policy_epoch: 4,
                    learning_generation: 2,
                },
            },
        }
    }

    fn voice_command(
        start: &VoiceRequest,
        request_id: &str,
        command: VoiceCommand,
    ) -> VoiceRequest {
        VoiceRequest {
            request_id: request_id.into(),
            command,
            ..start.clone()
        }
    }

    fn host_shortcut(
        start: &VoiceRequest,
        request_id: &str,
        is_pressed: bool,
        activation: VoiceShortcutActivation,
        starts_session: bool,
    ) -> VoiceRequest {
        use inputia_handy_runtime::voice_protocol::HostShortcutEdge;
        let VoiceCommand::Start {
            target,
            post_process,
            terms,
        } = start.command.clone()
        else {
            unreachable!();
        };
        VoiceRequest {
            request_id: request_id.into(),
            command: VoiceCommand::HostShortcut {
                target,
                post_process,
                terms,
                edge: HostShortcutEdge {
                    trigger_id: format!("trigger-{request_id}"),
                    starts_session,
                    lease_id: "lease-one".into(),
                    lease_epoch: 1,
                    binding_id: "transcribe".into(),
                    hotkey_string: "Option+Space".into(),
                    is_pressed,
                    activation,
                    pressed_at_unix_ms: 1,
                    hold_threshold_ms: 400,
                },
            },
            ..start.clone()
        }
    }

    #[test]
    fn prepared_result_survives_finish_and_cannot_be_replaced_or_move_next_session() {
        let mut state = CoordinatorState::new();
        let now = Instant::now();
        let start = voice_start("old", "start");
        state.on_voice(start.clone(), now).0.unwrap();
        assert!(state
            .on_voice_result_prepared(&start, "item".into(), "operation".into())
            .is_err());
        state
            .on_voice(voice_command(&start, "stop", VoiceCommand::Stop), now)
            .0
            .unwrap();
        let ready = state
            .on_voice_result_prepared(&start, "item".into(), "operation".into())
            .unwrap();
        assert_eq!(ready.phase, VoicePhase::PendingTarget);
        assert_eq!(ready.item_id.as_deref(), Some("item"));
        assert_eq!(ready.output_operation_id.as_deref(), Some("operation"));
        assert_eq!(
            state
                .on_voice_result_prepared(&start, "item".into(), "operation".into())
                .unwrap(),
            ready
        );
        assert!(state
            .on_voice_result_prepared(&start, "other".into(), "operation".into())
            .is_err());
        state.on_processing_finished();
        assert_eq!(state.voice_view("old").unwrap(), ready);
        let next = voice_start("next", "next-start");
        state.on_voice(next.clone(), now).0.unwrap();
        assert_eq!(
            state
                .on_voice_result_prepared(&start, "item".into(), "operation".into())
                .unwrap(),
            ready
        );
        assert_eq!(state.voice_context(), Some(next));
    }

    #[test]
    fn host_shortcut_push_to_talk_release_stops_owned_session_with_same_target() {
        let mut state = CoordinatorState::new();
        let now = Instant::now();
        let template = voice_start("host-session", "template");
        let start = host_shortcut(
            &template,
            "host-press",
            true,
            VoiceShortcutActivation::PushToTalk,
            true,
        );
        let (view, effect) = state.on_voice(start.clone(), now);
        assert!(matches!(effect, Some(Effect::Start { .. })));
        assert_eq!(view.unwrap().target_id.as_deref(), Some("field-token"));
        let release = host_shortcut(
            &template,
            "host-release",
            false,
            VoiceShortcutActivation::PushToTalk,
            false,
        );
        let (_, effect) = state.on_voice(release, now + Duration::from_millis(100));
        assert!(effect.is_none());
        let effect = state.on_grace_expired();
        assert!(matches!(effect, Some(Effect::Stop { .. })));
        assert_eq!(
            state.voice_view("host-session").unwrap().phase,
            VoicePhase::Processing
        );
    }

    #[test]
    fn host_shortcut_hold_or_toggle_tap_locks_until_next_press() {
        let mut state = CoordinatorState::new();
        let now = Instant::now();
        let template = voice_start("host-session", "template");
        let start = host_shortcut(
            &template,
            "host-press",
            true,
            VoiceShortcutActivation::HoldOrToggle,
            true,
        );
        assert!(matches!(
            state.on_voice(start, now).1,
            Some(Effect::Start { .. })
        ));
        let release = host_shortcut(
            &template,
            "host-release",
            false,
            VoiceShortcutActivation::HoldOrToggle,
            false,
        );
        assert!(state
            .on_voice(release, now + Duration::from_millis(50))
            .1
            .is_none());
        assert!(state.on_grace_expired().is_none());
        let stop_press = host_shortcut(
            &template,
            "host-stop",
            true,
            VoiceShortcutActivation::HoldOrToggle,
            false,
        );
        assert!(matches!(
            state
                .on_voice(stop_press, now + Duration::from_millis(500))
                .1,
            Some(Effect::Stop { .. })
        ));
    }

    #[test]
    fn host_shortcut_later_edge_cannot_change_frozen_target() {
        let mut state = CoordinatorState::new();
        let now = Instant::now();
        let template = voice_start("host-session", "template");
        let start = host_shortcut(
            &template,
            "host-press",
            true,
            VoiceShortcutActivation::Toggle,
            true,
        );
        state.on_voice(start, now).0.unwrap();
        let mut changed = host_shortcut(
            &template,
            "host-stop",
            true,
            VoiceShortcutActivation::Toggle,
            false,
        );
        if let VoiceCommand::HostShortcut { target, .. } = &mut changed.command {
            target.target_id = "other-target".into();
        }
        let (result, effect) = state.on_voice(changed, now + Duration::from_millis(500));
        assert!(result.is_err());
        assert!(effect.is_none());
        assert!(matches!(state.stage, Stage::Recording(_)));
    }

    #[test]
    fn menu_owned_start_can_be_stopped_by_unified_shortcut_press_without_new_session() {
        let mut state = CoordinatorState::new();
        let now = Instant::now();
        let start = voice_start("menu-session", "menu-start");
        assert!(state.on_voice(start.clone(), now).0.is_ok());
        let effect = state.on_input(
            InputEvent {
                binding_id: "transcribe".into(),
                hotkey_string: "Option+Space".into(),
                is_pressed: true,
                mode: ShortcutActivation::Toggle,
                hold_threshold: Duration::ZERO,
                external: false,
                owned: false,
            },
            now + Duration::from_millis(500),
        );
        assert!(matches!(effect, Some(Effect::Stop { .. })));
        assert_eq!(
            state.voice_view("menu-session").unwrap().phase,
            VoicePhase::Processing
        );
        assert_eq!(state.voice_sessions.len(), 1);
    }

    #[test]
    fn pending_result_cancel_preserves_history_identity_and_never_cancels_next_recording() {
        for phase in 0..3 {
            let mut state = CoordinatorState::new();
            let now = Instant::now();
            let old = voice_start("old", "start");
            state.on_voice(old.clone(), now).0.unwrap();
            state
                .on_voice(voice_command(&old, "stop", VoiceCommand::Stop), now)
                .0
                .unwrap();
            state
                .on_voice_result_prepared(&old, "item".into(), "operation".into())
                .unwrap();
            if phase >= 1 {
                state.on_processing_finished();
            }
            let next = voice_start("next", "new-start");
            if phase == 2 {
                state.on_voice(next.clone(), now).0.unwrap();
            }
            let (result, effect) =
                state.on_voice(voice_command(&old, "cancel", VoiceCommand::Cancel), now);
            let result = result.unwrap();
            assert_eq!(result.phase, VoicePhase::Cancelled);
            assert_eq!(result.item_id.as_deref(), Some("item"));
            assert!(effect.is_none());
            if phase == 2 {
                assert_eq!(state.voice_context(), Some(next));
            }
            let duplicate = state.on_voice(
                voice_command(&old, "cancel-again", VoiceCommand::Cancel),
                now,
            );
            assert!(duplicate.1.is_none());
            assert_eq!(duplicate.0.unwrap(), result);
        }
    }

    #[test]
    fn cancelled_or_foreign_result_cannot_turn_into_pending_output() {
        let mut state = CoordinatorState::new();
        let now = Instant::now();
        let start = voice_start("voice", "start");
        state.on_voice(start.clone(), now).0.unwrap();
        state
            .on_voice(voice_command(&start, "stop", VoiceCommand::Stop), now)
            .0
            .unwrap();
        let mut foreign = start.clone();
        foreign.client_instance = "another-host".into();
        assert!(state
            .on_voice_result_prepared(&foreign, "item".into(), "operation".into())
            .is_err());
        state
            .on_voice(voice_command(&start, "cancel", VoiceCommand::Cancel), now)
            .0
            .unwrap();
        assert!(state
            .on_voice_result_prepared(&start, "item".into(), "operation".into())
            .is_err());
        assert_eq!(
            state.voice_view("voice").unwrap().phase,
            VoicePhase::Cancelled
        );
        assert!(state.voice_view("voice").unwrap().item_id.is_none());
    }

    #[test]
    fn owned_start_is_not_toggle_and_only_current_microphone_ready_is_recording() {
        let mut state = CoordinatorState::new();
        let now = Instant::now();
        let start = voice_start("session-one", "start-one");
        let (view, effect) = state.on_voice(start.clone(), now);
        assert!(matches!(effect, Some(Effect::Start { .. })));
        assert_eq!(view.unwrap().phase, VoicePhase::Preparing);
        assert!(state.on_voice(start.clone(), now).1.is_none());
        assert!(state
            .on_voice(
                VoiceRequest {
                    request_id: "start-two".into(),
                    ..start.clone()
                },
                now
            )
            .1
            .is_none());
        state.on_start_result("transcribe", true);
        assert_eq!(
            state.voice_view("session-one").unwrap().phase,
            VoicePhase::Preparing
        );
        state.on_recording_ready("transcribe", 10);
        assert_eq!(
            state.voice_view("session-one").unwrap().phase,
            VoicePhase::Preparing
        );
        state.on_recording_requested("transcribe", 10, Some("session-one"));
        state.on_recording_ready("transcribe", 9);
        assert_eq!(
            state.voice_view("session-one").unwrap().phase,
            VoicePhase::Preparing
        );
        state.on_recording_ready("transcribe", 10);
        assert_eq!(
            state.on_voice(start, now).0.unwrap().phase,
            VoicePhase::Recording
        );
    }

    #[test]
    fn owned_session_rejects_foreign_client_server_and_request_or_start_conflicts() {
        let mut state = CoordinatorState::new();
        let now = Instant::now();
        let start = voice_start("session-one", "start-one");
        assert!(state.on_voice(start.clone(), now).0.is_ok());
        let reused = voice_command(&start, "start-one", VoiceCommand::Stop);
        assert!(state.on_voice(reused, now).0.is_err());
        for command in [
            VoiceCommand::Stop,
            VoiceCommand::Cancel,
            VoiceCommand::Status,
            start.command.clone(),
        ] {
            let mut request = voice_command(&start, &format!("foreign-{command:?}"), command);
            request.client_instance = "host-other".into();
            let (result, effect) = state.on_voice(request, now);
            assert!(result.is_err());
            assert!(effect.is_none());
        }
        let mut other_server = voice_command(&start, "other-server", VoiceCommand::Stop);
        other_server.server_instance = "server-other".into();
        assert!(state.on_voice(other_server, now).0.is_err());
        let mut changed = start.clone();
        changed.request_id = "changed-target".into();
        if let VoiceCommand::Start {
            target,
            post_process,
            ..
        } = &mut changed.command
        {
            target.target_id = "field-other".into();
            *post_process = true;
        }
        assert!(state.on_voice(changed, now).0.is_err());
        assert_eq!(state.stage, Stage::Recording("transcribe".into()));
    }

    #[test]
    fn owned_stop_cancel_are_idempotent_and_drain_does_not_claim_delivery() {
        let mut state = CoordinatorState::new();
        let now = Instant::now();
        let start = voice_start("session-one", "start-one");
        assert!(state.on_voice(start.clone(), now).0.is_ok());
        let stop = voice_command(&start, "stop-one", VoiceCommand::Stop);
        let (view, effect) = state.on_voice(stop.clone(), now);
        assert!(matches!(effect, Some(Effect::Stop { .. })));
        assert_eq!(view.unwrap().phase, VoicePhase::Processing);
        assert!(state.on_voice(stop, now).1.is_none());
        assert!(state
            .on_voice(voice_command(&start, "stop-two", VoiceCommand::Stop), now)
            .1
            .is_none());
        let cancel = voice_command(&start, "cancel-one", VoiceCommand::Cancel);
        assert_eq!(state.on_voice(cancel.clone(), now).1, Some(Effect::Cancel));
        assert!(state.on_voice(cancel, now).1.is_none());
        assert_eq!(state.stage, Stage::Processing);
        assert!(state.voice_context().is_none());
        state.on_processing_finished();
        assert_eq!(
            state.voice_view("session-one").unwrap().phase,
            VoicePhase::Cancelled
        );
        assert!(state.on_voice(start, now).1.is_none());
        assert_eq!(state.stage, Stage::Idle);
    }

    #[test]
    fn cancelled_session_and_queued_old_readiness_cannot_affect_replacement() {
        let mut state = CoordinatorState::new();
        let now = Instant::now();
        let old = voice_start("old-session", "old-start");
        assert!(state.on_voice(old.clone(), now).0.is_ok());
        assert!(state
            .on_voice(voice_command(&old, "old-cancel", VoiceCommand::Cancel), now)
            .0
            .is_ok());
        let next = voice_start("next-session", "next-start");
        assert!(state.on_voice(next.clone(), now).0.is_ok());
        state.on_recording_requested("transcribe", 10, Some("old-session"));
        state.on_recording_ready("transcribe", 10);
        assert_eq!(
            state.voice_view("next-session").unwrap().phase,
            VoicePhase::Preparing
        );
        state.on_recording_requested("transcribe", 11, Some("next-session"));
        state.on_recording_ready("transcribe", 10);
        assert_eq!(
            state.voice_view("next-session").unwrap().phase,
            VoicePhase::Preparing
        );
        state.on_recording_ready("transcribe", 11);
        assert_eq!(
            state.voice_view("next-session").unwrap().phase,
            VoicePhase::Recording
        );
        assert!(state
            .on_voice(
                voice_command(&old, "late-old-cancel", VoiceCommand::Cancel),
                now
            )
            .1
            .is_none());
        assert_eq!(state.voice_context(), Some(next));
    }

    #[test]
    fn busy_owned_start_rejection_is_not_later_replayed_as_a_start() {
        let mut state = CoordinatorState::new();
        let now = Instant::now();
        state.on_input(toggle_input(true), now);
        let request = voice_start("session-one", "busy-start");
        assert!(state.on_voice(request.clone(), now).0.is_err());
        state.on_cancel(true);
        assert!(state.on_voice(request.clone(), now).0.is_err());
        assert_eq!(state.stage, Stage::Idle);
        let fresh = VoiceRequest {
            request_id: "fresh-start".into(),
            ..request
        };
        assert!(matches!(
            state.on_voice(fresh, now).1,
            Some(Effect::Start { .. })
        ));
    }

    #[test]
    fn shortcut_stops_owned_recording_without_replacing_owner_and_preserves_processing_drain() {
        let mut state = CoordinatorState::new();
        let now = Instant::now();
        let start = voice_start("session-one", "start-one");
        assert!(state.on_voice(start.clone(), now).0.is_ok());
        assert!(state.on_input(ptt_input(false), now).is_none());
        assert!(state.pending_release.is_none());
        assert!(matches!(
            state.on_input(toggle_input(true), now),
            Some(Effect::Stop { .. })
        ));
        assert_eq!(state.voice_context(), Some(start.clone()));
        assert!(state
            .on_voice(voice_command(&start, "stop-one", VoiceCommand::Stop), now)
            .0
            .is_ok());
        state.on_input(toggle_input(true), now);
        assert!(state.pending_press.is_some());
        assert!(matches!(
            state.on_processing_finished(),
            Some(Effect::Start { .. })
        ));
        assert_eq!(
            state.voice_view("session-one").unwrap().phase,
            VoicePhase::Interrupted
        );
        assert!(state.voice_context().is_none());
        assert_eq!(state.stage, Stage::Recording("transcribe".into()));
        assert!(matches!(
            state.on_input(toggle_input(true), now),
            Some(Effect::Stop { .. })
        ));
    }

    #[test]
    fn failed_owned_start_is_terminal_and_late_ready_does_not_revive_it() {
        let mut state = CoordinatorState::new();
        let now = Instant::now();
        let start = voice_start("session-one", "start-one");
        assert!(state.on_voice(start.clone(), now).0.is_ok());
        state.on_start_result("transcribe", false);
        state.on_recording_requested("transcribe", 1, Some("session-one"));
        state.on_recording_ready("transcribe", 1);
        assert_eq!(
            state.on_voice(start, now).0.unwrap().phase,
            VoicePhase::Failed
        );
        assert_eq!(state.stage, Stage::Idle);
        assert!(state.voice_context().is_none());
    }

    #[test]
    fn owned_session_capacity_never_evicts_old_start_identity() {
        let mut state = CoordinatorState::new();
        let now = Instant::now();
        let first = voice_start("first", "first-start");
        for index in 0..MAX_VOICE_SESSIONS {
            let start = if index == 0 {
                first.clone()
            } else {
                voice_start(&format!("session-{index}"), &format!("start-{index}"))
            };
            assert!(state.on_voice(start.clone(), now).0.is_ok());
            assert!(state
                .on_voice(
                    voice_command(&start, &format!("cancel-{index}"), VoiceCommand::Cancel),
                    now,
                )
                .0
                .is_ok());
        }
        assert!(state
            .on_voice(voice_start("overflow", "overflow-start"), now)
            .0
            .is_err());
        assert_eq!(
            state.on_voice(first, now).0.unwrap().phase,
            VoicePhase::Cancelled
        );
        assert_eq!(state.stage, Stage::Idle);
    }

    #[test]
    fn voice_control_only_enqueues_and_reports_closed_channel() {
        let (tx, rx) = mpsc::channel();
        let coordinator = TranscriptionCoordinator {
            tx,
            voice_projection: Arc::new(VoiceProjection::default()),
        };
        let request = voice_start("session-one", "start-one");
        let result = coordinator.control_voice(request.clone());
        assert!(matches!(result.try_recv(), Err(mpsc::TryRecvError::Empty)));
        let Command::Voice {
            request: queued,
            reply,
            ..
        } = rx.try_recv().unwrap()
        else {
            panic!("expected voice command")
        };
        assert_eq!(*queued, request);
        reply.send(Err("synthetic refusal".into())).unwrap();
        assert_eq!(result.recv().unwrap(), Err("synthetic refusal".into()));
        drop(rx);
        assert!(coordinator.control_voice(request).recv().unwrap().is_err());
    }

    #[test]
    fn microphone_requested_hook_captures_identity_before_delayed_delivery() {
        let (tx, rx) = mpsc::channel();
        let projection = Arc::new(VoiceProjection::default());
        let coordinator = TranscriptionCoordinator {
            tx,
            voice_projection: projection.clone(),
        };
        let mut state = CoordinatorState::new();
        let now = Instant::now();
        let old = voice_start("old-session", "old-start");
        assert!(state.on_voice(old.clone(), now).0.is_ok());
        publish_projection(&state, &projection);
        coordinator.notify_recording_requested("transcribe", 10);
        assert!(state
            .on_voice(voice_command(&old, "cancel", VoiceCommand::Cancel), now)
            .0
            .is_ok());
        assert!(state
            .on_voice(voice_start("new-session", "new-start"), now)
            .0
            .is_ok());
        publish_projection(&state, &projection);
        let Command::RecordingRequested {
            binding_id,
            generation,
            session_id,
        } = rx.try_recv().unwrap()
        else {
            panic!("expected microphone request")
        };
        assert_eq!(session_id.as_deref(), Some("old-session"));
        state.on_recording_requested(&binding_id, generation, session_id.as_deref());
        state.on_recording_ready(&binding_id, generation);
        assert_eq!(
            state.voice_view("new-session").unwrap().phase,
            VoicePhase::Preparing
        );
    }

    #[test]
    fn owned_view_revision_changes_only_when_observable_phase_changes() {
        let mut state = CoordinatorState::new();
        let now = Instant::now();
        let start = voice_start("session-one", "start-one");
        let preparing = state.on_voice(start.clone(), now).0.unwrap();
        let lifecycle_generation = state.generation;
        assert_eq!(preparing.generation, 1);
        state.on_recording_requested("transcribe", 20, Some("session-one"));
        state.on_recording_requested("transcribe", 20, Some("session-one"));
        assert_eq!(state.voice_view("session-one").unwrap(), preparing);
        state.on_recording_ready("transcribe", 20);
        let recording = state.voice_view("session-one").unwrap();
        assert_eq!(recording.phase, VoicePhase::Recording);
        assert_eq!(recording.generation, preparing.generation + 1);
        state.on_recording_ready("transcribe", 20);
        state.on_recording_ready("transcribe", 19);
        assert_eq!(state.on_voice(start.clone(), now).0.unwrap(), recording);
        let stop = voice_command(&start, "stop", VoiceCommand::Stop);
        let processing = state.on_voice(stop.clone(), now).0.unwrap();
        assert_eq!(processing.phase, VoicePhase::Processing);
        assert_eq!(processing.generation, recording.generation + 1);
        assert_eq!(state.on_voice(stop, now).0.unwrap(), processing);
        let cancel = voice_command(&start, "cancel", VoiceCommand::Cancel);
        let cancelled = state.on_voice(cancel.clone(), now).0.unwrap();
        assert_eq!(cancelled.phase, VoicePhase::Cancelled);
        assert_eq!(cancelled.generation, processing.generation + 1);
        assert_eq!(state.on_voice(cancel, now).0.unwrap(), cancelled);
        state.on_cancel(true);
        state.on_processing_finished();
        assert_eq!(state.voice_view("session-one").unwrap(), cancelled);
        assert_eq!(state.generation, lifecycle_generation);
        let next = state
            .on_voice(voice_start("session-two", "start-two"), now)
            .0
            .unwrap();
        assert_eq!(state.generation, lifecycle_generation + 1);
        assert_eq!(next.generation, 1);
    }

    #[test]
    fn owned_interrupted_and_failed_views_have_new_revisions() {
        let mut state = CoordinatorState::new();
        let now = Instant::now();
        let start = voice_start("session-one", "start-one");
        assert!(state.on_voice(start.clone(), now).0.is_ok());
        let processing = state
            .on_voice(voice_command(&start, "stop", VoiceCommand::Stop), now)
            .0
            .unwrap();
        state.on_processing_finished();
        let interrupted = state.voice_view("session-one").unwrap();
        assert_eq!(interrupted.phase, VoicePhase::Interrupted);
        assert_eq!(interrupted.generation, processing.generation + 1);
        state.on_processing_finished();
        assert_eq!(state.voice_view("session-one").unwrap(), interrupted);
        let next = state
            .on_voice(voice_start("session-two", "start-two"), now)
            .0
            .unwrap();
        state.on_start_result("transcribe", false);
        let failed = state.voice_view("session-two").unwrap();
        assert_eq!(failed.phase, VoicePhase::Failed);
        assert_eq!(failed.generation, next.generation + 1);
    }

    #[test]
    fn local_owned_stop_preserves_debounce_binding_and_frozen_context() {
        for mode in [ShortcutActivation::Toggle, ShortcutActivation::HoldOrToggle] {
            let mut state = CoordinatorState::new();
            let now = Instant::now();
            let start = voice_start("session-one", "start-one");
            assert!(state.on_voice(start.clone(), now).0.is_ok());
            assert!(state.on_input(input(mode, false), now).is_none());
            assert!(state.on_input(ptt_input(true), now).is_none());
            let mut other = input(mode, true);
            other.binding_id = "transcribe_with_post_process".into();
            assert!(state.on_input(other, now).is_none());
            state.last_press = Some(now);
            assert!(state
                .on_input(input(mode, true), now + Duration::from_millis(5))
                .is_none());
            assert_eq!(state.stage, Stage::Recording("transcribe".into()));
            let effect = state.on_input(input(mode, true), now + DEBOUNCE);
            assert!(
                matches!(effect, Some(Effect::Stop { binding_id, .. }) if binding_id == "transcribe")
            );
            assert_eq!(state.voice_context(), Some(start));
            assert_eq!(
                state.voice_view("session-one").unwrap().phase,
                VoicePhase::Processing
            );
            assert!(state
                .on_input(input(mode, true), now + DEBOUNCE + Duration::from_millis(1))
                .is_none());
            assert!(state.pending_press.is_none());
        }
    }

    #[test]
    fn status_polling_does_not_grow_request_receipts() {
        let mut state = CoordinatorState::new();
        let now = Instant::now();
        let start = voice_start("session-one", "start-one");
        assert!(state.on_voice(start.clone(), now).0.is_ok());
        for index in 0..MAX_VOICE_REQUESTS + 2 {
            assert!(state
                .on_voice(
                    voice_command(&start, &format!("status-{index}"), VoiceCommand::Status),
                    now
                )
                .0
                .is_ok());
        }
        assert_eq!(state.voice_requests.len(), 1);
    }

    #[test]
    fn full_request_ledger_keeps_stop_cancel_available_without_growing_or_reviving_start() {
        let mut state = CoordinatorState::new();
        let now = Instant::now();
        let start = voice_start("session-one", "start-one");
        assert!(state.on_voice(start.clone(), now).0.is_ok());
        for index in 1..MAX_VOICE_REQUESTS {
            let rejected = voice_start("busy-other-session", &format!("busy-start-{index}"));
            assert!(state.on_voice(rejected, now).0.is_err());
        }
        assert_eq!(state.voice_requests.len(), MAX_VOICE_REQUESTS);
        for index in 0..MAX_VOICE_REQUESTS + 2 {
            assert!(state
                .on_voice(
                    voice_command(&start, &format!("status-{index}"), VoiceCommand::Status),
                    now
                )
                .0
                .is_ok());
            let (view, effect) = state.on_voice(
                voice_command(&start, &format!("stop-{index}"), VoiceCommand::Stop),
                now,
            );
            assert_eq!(view.unwrap().phase, VoicePhase::Processing);
            assert_eq!(effect.is_some(), index == 0);
        }
        assert_eq!(state.voice_requests.len(), MAX_VOICE_REQUESTS);
        for index in 0..MAX_VOICE_REQUESTS + 2 {
            let (view, effect) = state.on_voice(
                voice_command(&start, &format!("cancel-{index}"), VoiceCommand::Cancel),
                now,
            );
            assert_eq!(view.unwrap().phase, VoicePhase::Cancelled);
            assert_eq!(effect.is_some(), index == 0);
        }
        assert_eq!(state.voice_requests.len(), MAX_VOICE_REQUESTS);
        assert!(state
            .on_voice(
                voice_command(&start, "start-one", VoiceCommand::Cancel),
                now
            )
            .0
            .is_err());
        state.on_processing_finished();
        assert!(state
            .on_voice(voice_start("new-session", "new-start"), now)
            .0
            .is_err());
        let (old, effect) = state.on_voice(start, now);
        assert_eq!(old.unwrap().phase, VoicePhase::Cancelled);
        assert!(effect.is_none());
        assert_eq!(state.stage, Stage::Idle);
    }

    #[test]
    fn internal_voice_view_query_is_async_read_only_and_handles_closed_channel() {
        let (tx, rx) = mpsc::channel();
        let coordinator = TranscriptionCoordinator {
            tx,
            voice_projection: Arc::new(VoiceProjection::default()),
        };
        let mut state = CoordinatorState::new();
        let start = voice_start("session-one", "start-one");
        assert!(state.on_voice(start, Instant::now()).0.is_ok());
        let count = state.voice_requests.len();
        let result = coordinator.voice_session_view("session-one");
        assert!(matches!(result.try_recv(), Err(mpsc::TryRecvError::Empty)));
        let Command::VoiceSessionView { session_id, reply } = rx.try_recv().unwrap() else {
            panic!("expected read-only query")
        };
        reply.send(state.voice_view(&session_id)).unwrap();
        assert_eq!(result.recv().unwrap().unwrap().phase, VoicePhase::Preparing);
        assert_eq!(state.voice_requests.len(), count);
        assert_eq!(state.stage, Stage::Recording("transcribe".into()));
        drop(rx);
        assert!(coordinator
            .voice_session_view("session-one")
            .recv()
            .unwrap()
            .is_err());
    }

    #[test]
    fn expired_queued_start_is_rejected_without_effect_and_cannot_be_replayed() {
        let mut state = CoordinatorState::new();
        let now = Instant::now();
        let request = voice_start("session-one", "start-one");
        let (result, effect) = state.on_voice_before_deadline(request.clone(), now, now);
        assert!(result.is_err());
        assert!(effect.is_none());
        assert_eq!(state.stage, Stage::Idle);
        assert!(state.voice_context().is_none());
        assert!(state
            .on_voice_before_deadline(request, now + VOICE_START_QUEUE_TIMEOUT, now)
            .0
            .is_err());
        assert_eq!(state.stage, Stage::Idle);
    }

    #[test]
    fn expired_stop_cancel_still_close_existing_session() {
        let mut state = CoordinatorState::new();
        let now = Instant::now();
        let start = voice_start("session-one", "start-one");
        assert!(state.on_voice(start.clone(), now).0.is_ok());
        let (_, stop) = state.on_voice_before_deadline(
            voice_command(&start, "stop", VoiceCommand::Stop),
            now,
            now,
        );
        assert!(matches!(stop, Some(Effect::Stop { .. })));
        let (view, cancel) = state.on_voice_before_deadline(
            voice_command(&start, "cancel", VoiceCommand::Cancel),
            now,
            now,
        );
        assert_eq!(cancel, Some(Effect::Cancel));
        assert_eq!(view.unwrap().phase, VoicePhase::Cancelled);
    }

    #[test]
    fn dropped_start_receiver_prevents_effect_dispatch_and_preserves_failed_identity() {
        let mut state = CoordinatorState::new();
        let now = Instant::now();
        let request = voice_start("session-one", "start-one");
        let (result, effect) = state.on_voice(request.clone(), now);
        let Some(Effect::Start { binding_id, .. }) = effect else {
            panic!("expected prepared start")
        };
        let (reply, receiver) = mpsc::channel();
        drop(receiver);
        assert!(!state.acknowledge_start_before_effect(&binding_id, result, reply));
        assert_eq!(state.stage, Stage::Idle);
        assert!(state.voice_context().is_none());
        let (view, replay) = state.on_voice(request, now);
        assert_eq!(view.unwrap().phase, VoicePhase::Failed);
        assert!(replay.is_none());
    }

    #[test]
    fn accepted_start_receipt_is_preparing_not_recording() {
        let mut state = CoordinatorState::new();
        let now = Instant::now();
        let (result, _) = state.on_voice(voice_start("session-one", "start-one"), now);
        let (reply, receiver) = mpsc::channel();
        assert!(state.acknowledge_start_before_effect("transcribe", result, reply));
        assert_eq!(
            receiver.recv().unwrap().unwrap().phase,
            VoicePhase::Preparing
        );
        assert_eq!(
            state.voice_view("session-one").unwrap().phase,
            VoicePhase::Preparing
        );
        state.on_recording_requested("transcribe", 10, Some("session-one"));
        assert_eq!(
            state.voice_view("session-one").unwrap().phase,
            VoicePhase::Preparing
        );
        state.on_start_result("transcribe", false);
        assert_eq!(
            state.voice_view("session-one").unwrap().phase,
            VoicePhase::Failed
        );
        state.on_recording_ready("transcribe", 10);
        assert_eq!(
            state.voice_view("session-one").unwrap().phase,
            VoicePhase::Failed
        );
    }

    #[test]
    fn preparation_timeout_fails_once_and_cleanup_cannot_relabel_or_revive_it() {
        let mut state = CoordinatorState::new();
        let now = Instant::now();
        let start = voice_start("session-one", "start-one");
        let preparing = state.on_voice(start.clone(), now).0.unwrap();
        state.on_recording_requested("transcribe", 10, Some("session-one"));
        assert_eq!(
            state.on_preparation_failed("transcribe", 10),
            Some(Effect::Cancel)
        );
        let failed = state.voice_view("session-one").unwrap();
        assert_eq!(failed.phase, VoicePhase::Failed);
        assert_eq!(failed.generation, preparing.generation + 1);
        assert_eq!(state.stage, Stage::Idle);
        assert!(state.voice_context().is_none());
        assert!(state.on_preparation_failed("transcribe", 10).is_none());
        state.on_cancel(true);
        state.on_recording_ready("transcribe", 10);
        assert_eq!(state.voice_view("session-one").unwrap(), failed);
        let (view, effect) = state.on_voice(start, now);
        assert_eq!(view.unwrap(), failed);
        assert!(effect.is_none());
    }

    #[test]
    fn preparation_failure_rejects_unknown_old_binding_generation_and_newer_session() {
        let mut state = CoordinatorState::new();
        let now = Instant::now();
        let first = voice_start("session-one", "start-one");
        assert!(state.on_voice(first, now).0.is_ok());
        assert!(state.on_preparation_failed("transcribe", 10).is_none());
        state.on_recording_requested("transcribe", 10, Some("session-one"));
        assert!(state.on_preparation_failed("transcribe", 9).is_none());
        assert!(state
            .on_preparation_failed("transcribe_with_post_process", 10)
            .is_none());
        assert_eq!(
            state.on_preparation_failed("transcribe", 10),
            Some(Effect::Cancel)
        );
        let next = voice_start("session-two", "start-two");
        assert!(state.on_voice(next.clone(), now).0.is_ok());
        state.on_recording_requested("transcribe", 11, Some("session-two"));
        assert!(state.on_preparation_failed("transcribe", 10).is_none());
        assert_eq!(state.voice_context(), Some(next));
        assert_eq!(
            state.voice_view("session-two").unwrap().phase,
            VoicePhase::Preparing
        );
    }

    #[test]
    fn preparation_failure_after_ready_or_stop_does_not_cancel_the_session() {
        let now = Instant::now();
        for stopped in [false, true] {
            let mut state = CoordinatorState::new();
            let start = voice_start("session-one", "start-one");
            assert!(state.on_voice(start.clone(), now).0.is_ok());
            state.on_recording_requested("transcribe", 10, Some("session-one"));
            if stopped {
                assert!(state
                    .on_voice(voice_command(&start, "stop", VoiceCommand::Stop), now)
                    .0
                    .is_ok());
            } else {
                state.on_recording_ready("transcribe", 10);
            }
            let view = state.voice_view("session-one").unwrap();
            assert!(state.on_preparation_failed("transcribe", 10).is_none());
            assert_eq!(state.voice_view("session-one").unwrap(), view);
            assert_eq!(state.voice_context(), Some(start));
        }
    }

    #[test]
    fn preparation_failure_does_not_apply_to_legacy_shortcut_recording() {
        let mut state = CoordinatorState::new();
        state.on_input(toggle_input(true), Instant::now());
        state.on_recording_requested("transcribe", 10, None);
        assert!(state.on_preparation_failed("transcribe", 10).is_none());
        assert_eq!(state.stage, Stage::Recording("transcribe".into()));
    }

    fn processing_with_pending_shortcut(now: Instant) -> CoordinatorState {
        let mut state = CoordinatorState::new();
        assert!(matches!(
            state.on_input(toggle_input(true), now),
            Some(Effect::Start { .. })
        ));
        assert!(matches!(
            state.on_input(toggle_input(true), now),
            Some(Effect::Stop { .. })
        ));
        assert!(state.on_input(toggle_input(true), now).is_none());
        assert!(state.pending_press.is_some());
        state
    }

    #[test]
    fn real_cancel_route_before_old_finish_clears_pending_before_cleanup_and_drain() {
        let now = Instant::now();
        let mut state = processing_with_pending_shortcut(now);
        let (tx, rx) = mpsc::channel();
        let coordinator = TranscriptionCoordinator {
            tx,
            voice_projection: Arc::new(VoiceProjection::default()),
        };
        crate::utils::route_cancellation(Some(&coordinator), || {
            panic!("live worker must not use emergency cleanup")
        });
        coordinator.notify_processing_finished();
        // 公共入口只排队；此时还没有清理，也没有提前修改 actor 状态。
        assert!(state.pending_press.is_some());
        let mut effects = Vec::new();
        for _ in 0..2 {
            dispatch_command(
                &mut state,
                rx.try_recv().unwrap(),
                now,
                &mut |state, effect| {
                    if effect == Effect::Cancel {
                        assert!(state.pending_press.is_none());
                    }
                    effects.push(effect);
                },
            );
        }
        assert_eq!(effects, vec![Effect::Cancel]);
        assert_eq!(state.stage, Stage::Idle);
        assert!(matches!(rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
    }

    #[test]
    fn real_cancel_route_after_old_finish_cancels_the_new_current_recording() {
        let now = Instant::now();
        let mut state = processing_with_pending_shortcut(now);
        let (tx, rx) = mpsc::channel();
        let coordinator = TranscriptionCoordinator {
            tx,
            voice_projection: Arc::new(VoiceProjection::default()),
        };
        coordinator.notify_processing_finished();
        crate::utils::route_cancellation(Some(&coordinator), || panic!("unexpected emergency"));
        let mut effects = Vec::new();
        for _ in 0..2 {
            dispatch_command(
                &mut state,
                rx.try_recv().unwrap(),
                now,
                &mut |state, effect| {
                    if effect == Effect::Cancel {
                        assert_eq!(state.stage, Stage::Idle);
                    }
                    effects.push(effect);
                },
            );
        }
        assert!(matches!(effects.first(), Some(Effect::Start { .. })));
        assert_eq!(effects.last(), Some(&Effect::Cancel));
        assert_eq!(state.stage, Stage::Idle);
        assert!(matches!(rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
    }

    #[test]
    fn cleanup_serialization_prevents_new_start_from_crossing_cleanup() {
        let now = Instant::now();
        let state = processing_with_pending_shortcut(now);
        let (tx, rx) = mpsc::channel();
        let coordinator = TranscriptionCoordinator {
            tx,
            voice_projection: Arc::new(VoiceProjection::default()),
        };
        let (events, received) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let worker = thread::spawn(move || {
            let mut state = state;
            for _ in 0..3 {
                let command = rx.recv_timeout(Duration::from_secs(2)).unwrap();
                dispatch_command(&mut state, command, Instant::now(), &mut |state, effect| {
                    match effect {
                        Effect::Cancel => {
                            assert!(state.pending_press.is_none());
                            events.send("cleanup-start").unwrap();
                            released.recv_timeout(Duration::from_secs(2)).unwrap();
                            events.send("cleanup-finished").unwrap();
                        }
                        Effect::Start { .. } => {
                            events.send("start").unwrap();
                        }
                        Effect::Stop { .. } => panic!("unexpected stop"),
                    }
                });
            }
            state.stage
        });
        crate::utils::route_cancellation(Some(&coordinator), || panic!("unexpected emergency"));
        assert_eq!(
            received.recv_timeout(Duration::from_secs(2)).unwrap(),
            "cleanup-start"
        );
        coordinator.notify_processing_finished();
        coordinator.send_external_input("transcribe", "synthetic-after-cancel");
        assert!(matches!(
            received.recv_timeout(Duration::from_millis(20)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        release.send(()).unwrap();
        assert_eq!(
            received.recv_timeout(Duration::from_secs(2)).unwrap(),
            "cleanup-finished"
        );
        assert_eq!(
            received.recv_timeout(Duration::from_secs(2)).unwrap(),
            "start"
        );
        assert_eq!(
            worker.join().unwrap(),
            Stage::Recording("transcribe".into())
        );
    }

    #[test]
    fn real_cancel_route_uses_emergency_only_when_worker_is_absent_or_channel_closed() {
        let (tx, rx) = mpsc::channel();
        let coordinator = TranscriptionCoordinator {
            tx,
            voice_projection: Arc::new(VoiceProjection::default()),
        };
        let count = std::cell::Cell::new(0);
        crate::utils::route_cancellation(Some(&coordinator), || count.set(count.get() + 1));
        assert_eq!(count.get(), 0);
        assert!(matches!(rx.try_recv().unwrap(), Command::GlobalCancel));
        drop(rx);
        crate::utils::route_cancellation(Some(&coordinator), || count.set(count.get() + 1));
        crate::utils::route_cancellation(None, || count.set(count.get() + 1));
        assert_eq!(count.get(), 2);
    }

    #[test]
    fn push_to_talk_release_while_recording_defers_release() {
        assert_eq!(
            classify_ptt_event(None, false, true, "transcribe", Some("transcribe")),
            PttAction::DeferRelease
        );
    }

    #[test]
    fn push_to_talk_press_matching_pending_release_cancels_release() {
        assert_eq!(
            classify_ptt_event(
                Some("transcribe"),
                true,
                true,
                "transcribe",
                Some("transcribe")
            ),
            PttAction::CancelRelease
        );
    }

    #[test]
    fn toggle_mode_press_and_release_pass_through() {
        assert_eq!(
            classify_ptt_event(
                Some("transcribe"),
                true,
                false,
                "transcribe",
                Some("transcribe")
            ),
            PttAction::Passthrough
        );
        assert_eq!(
            classify_ptt_event(None, false, false, "transcribe", Some("transcribe")),
            PttAction::Passthrough
        );
    }

    #[test]
    fn press_for_different_binding_than_pending_release_passes_through() {
        assert_eq!(
            classify_ptt_event(
                Some("transcribe"),
                true,
                true,
                "transcribe_with_post_process",
                Some("transcribe")
            ),
            PttAction::Passthrough
        );
    }

    #[test]
    fn press_matching_pending_release_cancels_without_recording_state() {
        assert_eq!(
            classify_ptt_event(Some("transcribe"), true, true, "transcribe", None),
            PttAction::CancelRelease
        );
    }

    // ---------------------------------------------------------------------
    // Busy-pipeline input classification.
    //
    // Toggle-style triggers (SIGUSR2, CLI flags, pedals that signal on both
    // edges) flip state on every edge. Dropping a press that arrives while
    // the previous pipeline is still processing desyncs the parity: the next
    // edge then starts a recording no one will stop, leaving the overlay
    // waiting for input with the button long released.
    // ---------------------------------------------------------------------

    #[test]
    fn toggle_press_during_processing_remembers_start() {
        assert_eq!(
            classify_busy_input(true, ShortcutActivation::Toggle, None),
            BusyAction::Remember
        );
    }

    #[test]
    fn second_toggle_press_during_processing_forgets_press() {
        assert_eq!(
            classify_busy_input(true, ShortcutActivation::Toggle, Some(Remembered::Locked)),
            BusyAction::Forget
        );
    }

    #[test]
    fn toggle_release_during_processing_is_ignored() {
        assert_eq!(
            classify_busy_input(false, ShortcutActivation::Toggle, None),
            BusyAction::Ignore
        );
        assert_eq!(
            classify_busy_input(false, ShortcutActivation::Toggle, Some(Remembered::Locked)),
            BusyAction::Ignore
        );
    }

    #[test]
    fn hold_modes_classify_busy_inputs_by_pending_state() {
        let cases = [
            (true, None, BusyAction::Remember),
            (true, Some(Remembered::Held), BusyAction::Ignore),
            (true, Some(Remembered::Locked), BusyAction::Forget),
            (false, None, BusyAction::Ignore),
            (false, Some(Remembered::Held), BusyAction::Ignore),
            (false, Some(Remembered::Locked), BusyAction::Ignore),
        ];

        for mode in [
            ShortcutActivation::PushToTalk,
            ShortcutActivation::HoldOrToggle,
        ] {
            for (is_pressed, remembered, expected) in cases {
                assert_eq!(classify_busy_input(is_pressed, mode, remembered), expected);
            }
        }
    }

    /// Toggle parity across a busy window: an odd number of presses remembers
    /// one start, each further press flips the remembered press off/on again.
    #[test]
    fn toggle_presses_alternate_remember_and_forget_while_busy() {
        let mut remembered = None;
        for expected in [
            BusyAction::Remember,
            BusyAction::Forget,
            BusyAction::Remember,
        ] {
            let action = classify_busy_input(true, ShortcutActivation::Toggle, remembered);
            assert_eq!(action, expected);
            remembered = (action == BusyAction::Remember).then_some(Remembered::Locked);
        }
        assert!(remembered.is_some());
    }

    // ---------------------------------------------------------------------
    // Sequence-level regression coverage for issue #1539.
    //
    // Under X11 key auto-repeat, holding a push-to-talk key does not emit one
    // long press. It emits the initial press followed by a stream of
    // synthesized release/press pairs, then a single genuine release on key-up.
    // Before the fix, every synthesized release passed straight through and
    // stopped recording, so holding the key "rapidly toggled" recording on and
    // off. The fix defers each release for a short grace window and cancels it
    // when the matching auto-repeat press arrives.
    //
    // The unit tests above assert the classifiers in isolation. The harness
    // below drives the real `CoordinatorState` through whole event sequences
    // — the same `on_input` / `on_grace_expired` handlers the coordinator
    // thread runs — so a burst can be exercised deterministically without a
    // Tauri AppHandle or real timers, and the tests can never drift from the
    // production transitions.
    // ---------------------------------------------------------------------

    const BINDING: &str = "transcribe";

    #[derive(Clone, Copy)]
    enum Ev {
        /// A key-down event (real initial press or a synthesized auto-repeat press).
        Press,
        /// A key-up event (synthesized auto-repeat release or the genuine key-up).
        Release,
        /// The `RELEASE_GRACE` window elapsed with no cancelling press arriving.
        Grace,
    }

    struct DriveResult {
        starts: u32,
        stops: u32,
        stage: Stage,
    }

    fn ptt_input(is_pressed: bool) -> InputEvent {
        InputEvent {
            binding_id: BINDING.to_string(),
            hotkey_string: BINDING.to_string(),
            is_pressed,
            mode: ShortcutActivation::PushToTalk,
            hold_threshold: Duration::ZERO,
            external: false,
            owned: false,
        }
    }

    /// Feeds an event sequence to a real [`CoordinatorState`] the way the
    /// coordinator thread would; effects are counted instead of executed.
    fn drive(events: &[Ev]) -> DriveResult {
        let mut state = CoordinatorState::new();
        let mut clock = Instant::now();
        let mut starts = 0u32;
        let mut stops = 0u32;

        for ev in events {
            // Auto-repeat events arrive a few ms apart, well inside DEBOUNCE.
            clock += Duration::from_millis(5);

            let effect = match ev {
                Ev::Grace => state.on_grace_expired(),
                Ev::Press | Ev::Release => {
                    state.on_input(ptt_input(matches!(ev, Ev::Press)), clock)
                }
            };
            match effect {
                Some(Effect::Start { .. }) => starts += 1,
                Some(Effect::Stop { .. }) => stops += 1,
                Some(Effect::Cancel) => panic!("普通按键不应产生远程会话取消 effect"),
                None => {}
            }
        }

        DriveResult {
            starts,
            stops,
            stage: state.stage,
        }
    }

    /// Initial press plus several synthesized release/press pairs, as X11 emits
    /// while a push-to-talk key is held down.
    fn autorepeat_burst() -> Vec<Ev> {
        let mut events = vec![Ev::Press];
        for _ in 0..6 {
            events.push(Ev::Release);
            events.push(Ev::Press);
        }
        events
    }

    /// Regression for #1539: a burst of X11 auto-repeat release/press pairs must
    /// not stop recording. Before the fix the first synthesized release stopped
    /// recording immediately (stops == 1, stage left Recording), which produced
    /// the rapid on/off toggling. With the fix the releases are coalesced and
    /// recording stays continuously active for the whole burst.
    #[test]
    fn x11_autorepeat_burst_does_not_toggle_recording() {
        let result = drive(&autorepeat_burst());
        assert_eq!(result.starts, 1, "recording should start exactly once");
        assert_eq!(
            result.stops, 0,
            "synthesized auto-repeat releases must not stop recording mid-burst"
        );
        assert_eq!(
            result.stage,
            Stage::Recording(BINDING.to_string()),
            "recording must remain active across the entire auto-repeat burst"
        );
    }

    /// Complements the burst test: once the key is genuinely released and the
    /// grace window elapses with no re-press, recording stops exactly once. This
    /// proves the debounce only coalesces synthesized releases and does not wedge
    /// the coordinator or swallow the real key-up.
    #[test]
    fn genuine_release_after_grace_stops_recording_once() {
        let mut events = autorepeat_burst();
        events.push(Ev::Release); // genuine key-up
        events.push(Ev::Grace); // grace window elapses, no cancelling press
        let result = drive(&events);
        assert_eq!(result.starts, 1, "recording should start exactly once");
        assert_eq!(
            result.stops, 1,
            "a genuine release should stop recording exactly once"
        );
        assert_eq!(result.stage, Stage::Processing);
    }

    // ---------------------------------------------------------------------
    // Sequence-level coverage of the busy-pipeline and cancel paths, driven
    // through the real machine.
    // ---------------------------------------------------------------------

    /// PTT press while the pipeline is busy is remembered and starts recording
    /// once the pipeline drains.
    #[test]
    fn press_during_processing_starts_after_drain() {
        let mut state = CoordinatorState::new();
        let now = Instant::now();

        let effect = state.on_input(ptt_input(true), now);
        assert!(matches!(effect, Some(Effect::Start { .. })));

        let effect = state.on_input(ptt_input(false), now + Duration::from_millis(100));
        assert!(effect.is_none(), "release should be deferred, not fired");

        let effect = state.on_grace_expired();
        assert!(matches!(effect, Some(Effect::Stop { .. })));

        let effect = state.on_input(ptt_input(true), now + Duration::from_millis(200));
        assert!(effect.is_none(), "busy pipeline must remember, not start");

        let effect = state.on_processing_finished();
        assert!(
            matches!(effect, Some(Effect::Start { .. })),
            "remembered press should start once the pipeline drains"
        );
    }

    /// Two toggle presses inside one busy window net to no-op: nothing starts
    /// when the pipeline drains (toggle parity).
    #[test]
    fn toggle_presses_during_processing_net_noop_after_drain() {
        let mut state = CoordinatorState::new();
        let now = Instant::now();

        let effect = state.on_input(ptt_input(true), now);
        assert!(matches!(effect, Some(Effect::Start { .. })));
        let effect = state.on_input(ptt_input(false), now + Duration::from_millis(100));
        assert!(effect.is_none());
        let effect = state.on_grace_expired();
        assert!(matches!(effect, Some(Effect::Stop { .. })));

        let toggle = |state: &mut CoordinatorState, at: Instant| {
            state.on_input(
                InputEvent {
                    binding_id: BINDING.to_string(),
                    hotkey_string: BINDING.to_string(),
                    is_pressed: true,
                    mode: ShortcutActivation::Toggle,
                    hold_threshold: Duration::ZERO,
                    external: true,
                    owned: false,
                },
                at,
            )
        };

        let effect = toggle(&mut state, now + Duration::from_millis(200));
        assert!(effect.is_none());
        let effect = toggle(&mut state, now + Duration::from_millis(300));
        assert!(effect.is_none());

        let effect = state.on_processing_finished();
        assert!(
            effect.is_none(),
            "even number of busy toggle presses must not start recording"
        );
        assert_eq!(state.stage, Stage::Idle);
    }

    /// Cancel while processing abandons a remembered press: the pipeline drains
    /// to idle and nothing starts.
    #[test]
    fn cancel_during_processing_drops_remembered_press() {
        let mut state = CoordinatorState::new();
        let now = Instant::now();

        let effect = state.on_input(ptt_input(true), now);
        assert!(matches!(effect, Some(Effect::Start { .. })));
        let effect = state.on_input(ptt_input(false), now + Duration::from_millis(100));
        assert!(effect.is_none());
        let effect = state.on_grace_expired();
        assert!(matches!(effect, Some(Effect::Stop { .. })));

        let effect = state.on_input(ptt_input(true), now + Duration::from_millis(200));
        assert!(effect.is_none());

        state.on_cancel(false);
        assert_eq!(
            state.stage,
            Stage::Processing,
            "cancel must not reset mid-processing — the pipeline still finishes"
        );

        let effect = state.on_processing_finished();
        assert!(
            effect.is_none(),
            "cancelled session must not spawn a deferred recording"
        );
        assert_eq!(state.stage, Stage::Idle);
    }

    fn toggle_input(external: bool) -> InputEvent {
        toggle_input_for(BINDING, external)
    }

    fn toggle_input_for(binding_id: &str, external: bool) -> InputEvent {
        InputEvent {
            binding_id: binding_id.to_string(),
            hotkey_string: binding_id.to_string(),
            is_pressed: true,
            mode: ShortcutActivation::Toggle,
            hold_threshold: Duration::ZERO,
            external,
            owned: false,
        }
    }

    /// Start and stop one toggle recording so the machine sits in `Processing`.
    fn drive_into_processing(state: &mut CoordinatorState, now: Instant) {
        let effect = state.on_input(toggle_input(true), now);
        assert!(matches!(effect, Some(Effect::Start { .. })));
        let effect = state.on_input(toggle_input(true), now + Duration::from_millis(100));
        assert!(matches!(effect, Some(Effect::Stop { .. })));
        assert_eq!(state.stage, Stage::Processing);
    }

    const OTHER_BINDING: &str = "transcribe_with_post_process";

    /// Only one press can be pending. Once a binding has claimed it, a toggle
    /// for a different binding is ignored (as it is while recording) instead of
    /// replacing the remembered press, so the pending binding's parity holds:
    /// two transcribe toggles still net to no-op.
    #[test]
    fn different_binding_does_not_replace_pending_press() {
        let mut state = CoordinatorState::new();
        let now = Instant::now();
        drive_into_processing(&mut state, now);

        let at = |ms| now + Duration::from_millis(ms);
        assert!(state.on_input(toggle_input(true), at(200)).is_none());
        assert!(state
            .on_input(toggle_input_for(OTHER_BINDING, true), at(300))
            .is_none());
        assert!(state.on_input(toggle_input(true), at(400)).is_none());

        let effect = state.on_processing_finished();
        assert!(
            effect.is_none(),
            "two transcribe toggles net to no-op; the ignored post-process toggle must not start"
        );
        assert_eq!(state.stage, Stage::Idle);
    }

    /// The binding that claimed the pending press is the one that starts on
    /// drain, regardless of other bindings toggled in between.
    #[test]
    fn drain_starts_the_pending_binding_not_a_later_one() {
        let mut state = CoordinatorState::new();
        let now = Instant::now();
        drive_into_processing(&mut state, now);

        let at = |ms| now + Duration::from_millis(ms);
        assert!(state.on_input(toggle_input(true), at(200)).is_none());
        assert!(state
            .on_input(toggle_input_for(OTHER_BINDING, true), at(300))
            .is_none());

        match state.on_processing_finished() {
            Some(Effect::Start { binding_id, .. }) => assert_eq!(binding_id, BINDING),
            other => panic!("expected Start for '{BINDING}', got {other:?}"),
        }
    }

    /// External triggers fire on every edge by design (e.g. SIGUSR2 sent on
    /// both key press and release). Two edges inside the debounce window must
    /// both be honoured, or the parity desyncs and recording wedges on.
    #[test]
    fn external_edges_inside_debounce_window_are_not_dropped() {
        let mut state = CoordinatorState::new();
        let now = Instant::now();

        let effect = state.on_input(toggle_input(true), now);
        assert!(matches!(effect, Some(Effect::Start { .. })));

        let effect = state.on_input(toggle_input(true), now + Duration::from_millis(5));
        assert!(
            matches!(effect, Some(Effect::Stop { .. })),
            "second external edge inside DEBOUNCE must stop the recording"
        );
        assert_eq!(state.stage, Stage::Processing);
    }

    /// Physical keyboard presses keep the debounce: a repeat inside the window
    /// is still dropped and recording stays active.
    #[test]
    fn keyboard_press_inside_debounce_window_is_still_dropped() {
        let mut state = CoordinatorState::new();
        let now = Instant::now();

        let effect = state.on_input(toggle_input(false), now);
        assert!(matches!(effect, Some(Effect::Start { .. })));

        let effect = state.on_input(toggle_input(false), now + Duration::from_millis(5));
        assert!(
            effect.is_none(),
            "keyboard repeat inside DEBOUNCE must be debounced"
        );
        assert_eq!(state.stage, Stage::Recording(BINDING.to_string()));
    }

    /// If the start effect fails to begin recording (e.g. microphone access
    /// denied), the optimistic transition rolls back to idle.
    #[test]
    fn failed_start_rolls_back_to_idle() {
        let mut state = CoordinatorState::new();

        let effect = state.on_input(ptt_input(true), Instant::now());
        assert!(matches!(effect, Some(Effect::Start { .. })));

        state.on_start_result(BINDING, false);
        assert_eq!(state.stage, Stage::Idle);
    }

    // ---------------------------------------------------------------------
    // Hold-or-toggle (the combined mode from #147) and the two legacy modes,
    // driven through the real machine on a synthetic clock. Recording starts
    // on key-down in every mode; the tests pin how each mode ends it.
    // ---------------------------------------------------------------------

    const HOLD_THRESHOLD: Duration = Duration::from_millis(300);

    fn input(mode: ShortcutActivation, is_pressed: bool) -> InputEvent {
        InputEvent {
            binding_id: BINDING.to_string(),
            hotkey_string: BINDING.to_string(),
            is_pressed,
            mode,
            hold_threshold: HOLD_THRESHOLD,
            external: false,
            owned: false,
        }
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// Hold-or-toggle: a key held past the threshold is push-to-talk — the
    /// (deferred) release stops recording.
    #[test]
    fn hold_or_toggle_long_hold_stops_on_release() {
        let mode = ShortcutActivation::HoldOrToggle;
        let mut state = CoordinatorState::new();
        let t0 = Instant::now();

        assert!(matches!(
            state.on_input(input(mode, true), t0),
            Some(Effect::Start { .. })
        ));
        assert!(state.on_input(input(mode, false), t0 + ms(800)).is_none());
        assert!(
            matches!(state.on_grace_expired(), Some(Effect::Stop { .. })),
            "an 800ms hold must stop when its release grace elapses"
        );
        assert_eq!(state.stage, Stage::Processing);
    }

    /// Hold-or-toggle: a tap keeps recording (locked on); the next press stops.
    #[test]
    fn hold_or_toggle_tap_locks_recording_until_next_press() {
        let mode = ShortcutActivation::HoldOrToggle;
        let mut state = CoordinatorState::new();
        let t0 = Instant::now();

        assert!(matches!(
            state.on_input(input(mode, true), t0),
            Some(Effect::Start { .. })
        ));
        assert!(state.on_input(input(mode, false), t0 + ms(120)).is_none());
        assert!(
            state.on_grace_expired().is_none(),
            "a 120ms tap must not stop recording"
        );
        assert_eq!(state.stage, Stage::Recording(BINDING.to_string()));
        assert!(state.is_locked());

        // Seconds later the user presses again to finish.
        assert!(matches!(
            state.on_input(input(mode, true), t0 + ms(5000)),
            Some(Effect::Stop { .. })
        ));
        assert_eq!(state.stage, Stage::Processing);
        // The release of that stopping press lands in the busy window and is
        // ignored, so nothing is remembered for the drain.
        assert!(state.on_input(input(mode, false), t0 + ms(5080)).is_none());
        assert!(state.on_processing_finished().is_none());
        assert_eq!(state.stage, Stage::Idle);
    }

    /// Hold-or-toggle: a locked session ignores stray releases — only a press
    /// ends it.
    #[test]
    fn hold_or_toggle_locked_session_ignores_release() {
        let mode = ShortcutActivation::HoldOrToggle;
        let mut state = CoordinatorState::new();
        let t0 = Instant::now();

        state.on_input(input(mode, true), t0);
        state.on_input(input(mode, false), t0 + ms(100));
        assert!(state.on_grace_expired().is_none());
        assert!(state.is_locked());

        assert!(state.on_input(input(mode, false), t0 + ms(900)).is_none());
        assert!(
            state.grace_deadline().is_none(),
            "no release may be deferred once locked"
        );
        assert_eq!(state.stage, Stage::Recording(BINDING.to_string()));
    }

    /// Hold-or-toggle: while the key is genuinely held, extra presses do not
    /// stop the recording (that is the release's job).
    #[test]
    fn hold_or_toggle_press_while_held_is_ignored() {
        let mode = ShortcutActivation::HoldOrToggle;
        let mut state = CoordinatorState::new();
        let t0 = Instant::now();

        state.on_input(input(mode, true), t0);
        assert!(state.on_input(input(mode, true), t0 + ms(400)).is_none());
        assert_eq!(state.stage, Stage::Recording(BINDING.to_string()));
        assert!(!state.is_locked());
    }

    /// Hold-or-toggle under X11 auto-repeat: the synthesized release/press
    /// pairs must not be misread as taps. The hold is measured from the
    /// original key-down to the genuine key-up.
    #[test]
    fn hold_or_toggle_autorepeat_burst_is_one_long_hold() {
        let mode = ShortcutActivation::HoldOrToggle;
        let mut state = CoordinatorState::new();
        let t0 = Instant::now();
        let mut clock = t0;

        assert!(matches!(
            state.on_input(input(mode, true), clock),
            Some(Effect::Start { .. })
        ));
        // ~600ms of auto-repeat pairs a few ms apart.
        for _ in 0..60 {
            clock += ms(5);
            assert!(state.on_input(input(mode, false), clock).is_none());
            clock += ms(5);
            assert!(state.on_input(input(mode, true), clock).is_none());
            assert!(
                state.grace_deadline().is_none(),
                "auto-repeat press must cancel the deferred release"
            );
        }
        assert!(!state.is_locked(), "no tap may be classified mid-burst");

        clock += ms(5);
        assert!(state.on_input(input(mode, false), clock).is_none());
        assert!(
            matches!(state.on_grace_expired(), Some(Effect::Stop { .. })),
            "the genuine release after a ~600ms hold must stop recording"
        );
    }

    /// Hold-or-toggle: a press remembered during the busy window is measured
    /// from the real key-down, so a hold that straddles the drain still counts
    /// as a hold when it is released shortly after recording actually starts.
    #[test]
    fn hold_or_toggle_remembered_press_measures_hold_from_real_key_down() {
        let mode = ShortcutActivation::HoldOrToggle;
        let mut state = CoordinatorState::new();
        let t0 = Instant::now();

        // Previous session: hold, release, stop → Processing.
        state.on_input(input(mode, true), t0);
        state.on_input(input(mode, false), t0 + ms(800));
        assert!(matches!(
            state.on_grace_expired(),
            Some(Effect::Stop { .. })
        ));

        // Pressed again while busy; still held when the pipeline drains 700ms later.
        assert!(state.on_input(input(mode, true), t0 + ms(1000)).is_none());
        assert!(matches!(
            state.on_processing_finished(),
            Some(Effect::Start { .. })
        ));
        // Released 100ms after recording began — but 800ms after key-down.
        assert!(state.on_input(input(mode, false), t0 + ms(1800)).is_none());
        assert!(
            matches!(state.on_grace_expired(), Some(Effect::Stop { .. })),
            "held 800ms overall: must stop, not lock"
        );
    }

    /// Toggle: releases never stop, the next press does. (Toggle is the
    /// combined machine with the session locked from the start.)
    #[test]
    fn toggle_mode_ignores_release_and_stops_on_next_press() {
        let mode = ShortcutActivation::Toggle;
        let mut state = CoordinatorState::new();
        let t0 = Instant::now();

        assert!(matches!(
            state.on_input(input(mode, true), t0),
            Some(Effect::Start { .. })
        ));
        assert!(state.is_locked());
        assert!(state.on_input(input(mode, false), t0 + ms(100)).is_none());
        assert!(
            state.grace_deadline().is_none(),
            "toggle never defers releases"
        );
        assert!(state.on_input(input(mode, false), t0 + ms(3000)).is_none());
        assert!(matches!(
            state.on_input(input(mode, true), t0 + ms(4000)),
            Some(Effect::Stop { .. })
        ));
    }

    /// Push-to-talk: even a very short press stops on release — there is no
    /// tap-to-lock in this mode (hold threshold of zero).
    #[test]
    fn push_to_talk_short_press_still_stops_on_release() {
        let mode = ShortcutActivation::PushToTalk;
        let mut state = CoordinatorState::new();
        let t0 = Instant::now();

        assert!(matches!(
            state.on_input(input(mode, true), t0),
            Some(Effect::Start { .. })
        ));
        assert!(state.on_input(input(mode, false), t0 + ms(40)).is_none());
        assert!(matches!(
            state.on_grace_expired(),
            Some(Effect::Stop { .. })
        ));
    }

    /// Cancel (Escape) during a locked hold-or-toggle session resets cleanly so
    /// the next press starts a fresh recording rather than stopping a dead one.
    #[test]
    fn hold_or_toggle_cancel_clears_locked_session() {
        let mode = ShortcutActivation::HoldOrToggle;
        let mut state = CoordinatorState::new();
        let t0 = Instant::now();

        state.on_input(input(mode, true), t0);
        state.on_input(input(mode, false), t0 + ms(100));
        assert!(state.on_grace_expired().is_none());
        assert!(state.is_locked());

        state.on_cancel(true);
        assert_eq!(state.stage, Stage::Idle);
        assert!(!state.is_locked());
        assert!(matches!(
            state.on_input(input(mode, true), t0 + ms(2000)),
            Some(Effect::Start { .. })
        ));
    }

    /// Switching to toggle while an unlocked hold recording is running must not
    /// strand it: in toggle mode a press always stops.
    #[test]
    fn toggle_press_stops_recording_started_as_hold() {
        let mut state = CoordinatorState::new();
        let t0 = Instant::now();

        state.on_input(input(ShortcutActivation::HoldOrToggle, true), t0);
        assert!(!state.is_locked());
        assert!(matches!(
            state.on_input(input(ShortcutActivation::Toggle, true), t0 + ms(2000)),
            Some(Effect::Stop { .. })
        ));
    }

    // Hold-vs-tap classification while the previous transcription is busy.
    fn hold_or_toggle_into_processing(state: &mut CoordinatorState, t0: Instant) {
        let mode = ShortcutActivation::HoldOrToggle;
        assert!(matches!(
            state.on_input(input(mode, true), t0),
            Some(Effect::Start { .. })
        ));
        assert!(state.on_input(input(mode, false), t0 + ms(800)).is_none());
        assert!(matches!(
            state.on_grace_expired(),
            Some(Effect::Stop { .. })
        ));
        assert_eq!(state.stage, Stage::Processing);
    }

    #[test]
    fn hold_or_toggle_tap_during_processing_queues_locked_start() {
        let mode = ShortcutActivation::HoldOrToggle;
        let mut state = CoordinatorState::new();
        let t0 = Instant::now();
        hold_or_toggle_into_processing(&mut state, t0);

        assert!(state.on_input(input(mode, true), t0 + ms(1000)).is_none());
        assert!(state.on_input(input(mode, false), t0 + ms(1100)).is_none());
        assert!(state.on_grace_expired().is_none());
        assert!(state.is_locked(), "a busy tap should queue a locked start");
        assert!(matches!(
            state.on_processing_finished(),
            Some(Effect::Start { .. })
        ));
        assert!(state.is_locked());
    }

    #[test]
    fn hold_or_toggle_completed_hold_during_processing_nets_noop() {
        let mode = ShortcutActivation::HoldOrToggle;
        let mut state = CoordinatorState::new();
        let t0 = Instant::now();
        hold_or_toggle_into_processing(&mut state, t0);

        assert!(state.on_input(input(mode, true), t0 + ms(1000)).is_none());
        assert!(state.on_input(input(mode, false), t0 + ms(1600)).is_none());
        assert!(state.on_grace_expired().is_none());
        assert!(!state.is_locked());

        assert!(
            state.on_processing_finished().is_none(),
            "a 600ms hold that ended before the drain has nothing left to start"
        );
        assert_eq!(state.stage, Stage::Idle);
    }

    #[test]
    fn hold_or_toggle_two_taps_during_processing_net_noop() {
        let mode = ShortcutActivation::HoldOrToggle;
        let mut state = CoordinatorState::new();
        let t0 = Instant::now();
        hold_or_toggle_into_processing(&mut state, t0);

        assert!(state.on_input(input(mode, true), t0 + ms(1000)).is_none());
        assert!(state.on_input(input(mode, false), t0 + ms(1100)).is_none());
        assert!(state.on_grace_expired().is_none());
        assert!(state.is_locked());

        assert!(state.on_input(input(mode, true), t0 + ms(1500)).is_none());
        assert!(
            !state.is_locked(),
            "the second tap's press forgets the queued tap"
        );
        assert!(state.on_input(input(mode, false), t0 + ms(1600)).is_none());
        assert!(state.grace_deadline().is_none());

        assert!(state.on_processing_finished().is_none());
        assert_eq!(state.stage, Stage::Idle);
    }

    #[test]
    fn ptt_tap_inside_busy_window_nets_noop() {
        let mut state = CoordinatorState::new();
        let t0 = Instant::now();
        hold_or_toggle_into_processing(&mut state, t0);

        assert!(state.on_input(ptt_input(true), t0 + ms(1000)).is_none());
        assert!(state.on_input(ptt_input(false), t0 + ms(1040)).is_none());
        assert!(state.on_grace_expired().is_none());
        assert!(state.on_processing_finished().is_none());
    }

    /// The pipeline drains inside the 50ms grace of a busy tap: recording
    /// starts first (unlocked, from the real key-down), and the grace then
    /// resolves against the live recording, locking it as the tap it was.
    #[test]
    fn hold_or_toggle_drain_inside_busy_release_grace_still_classifies_tap() {
        let mode = ShortcutActivation::HoldOrToggle;
        let mut state = CoordinatorState::new();
        let t0 = Instant::now();
        hold_or_toggle_into_processing(&mut state, t0);

        assert!(state.on_input(input(mode, true), t0 + ms(1000)).is_none());
        assert!(state.on_input(input(mode, false), t0 + ms(1100)).is_none());
        assert!(matches!(
            state.on_processing_finished(),
            Some(Effect::Start { .. })
        ));
        assert!(!state.is_locked());

        assert!(state.on_grace_expired().is_none());
        assert!(state.is_locked(), "the deferred 100ms release is a tap");
    }

    /// X11 auto-repeat while busy, key still held at the drain: recording
    /// starts measured from the first press, not from the last synthesized
    /// press before the drain. Released 400ms after the real key-down but
    /// only ~100ms after the drain — a hold, so it must stop rather than lock.
    #[test]
    fn hold_or_toggle_autorepeat_burst_straddling_drain_measures_from_first_press() {
        let mode = ShortcutActivation::HoldOrToggle;
        let mut state = CoordinatorState::new();
        let t0 = Instant::now();
        hold_or_toggle_into_processing(&mut state, t0);

        let mut clock = t0 + ms(1000);
        assert!(state.on_input(input(mode, true), clock).is_none());
        for _ in 0..30 {
            clock += ms(5);
            assert!(state.on_input(input(mode, false), clock).is_none());
            clock += ms(5);
            assert!(state.on_input(input(mode, true), clock).is_none());
            assert!(state.grace_deadline().is_none());
        }

        // Drain at ~t0 + 1300ms with the key still down.
        assert!(matches!(
            state.on_processing_finished(),
            Some(Effect::Start { .. })
        ));
        assert!(!state.is_locked());

        for _ in 0..10 {
            clock += ms(5);
            assert!(state.on_input(input(mode, false), clock).is_none());
            clock += ms(5);
            assert!(state.on_input(input(mode, true), clock).is_none());
        }
        assert_eq!(clock, t0 + ms(1400));
        assert!(state.on_input(input(mode, false), clock).is_none());
        assert!(
            matches!(state.on_grace_expired(), Some(Effect::Stop { .. })),
            "held 400ms since the real key-down: must stop, not lock"
        );
        assert_eq!(state.stage, Stage::Processing);
    }
}
