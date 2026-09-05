//! Receipt-sequenced clipboard paste ("reliable paste", debug-gated).
//!
//! The legacy clipboard paste (`clipboard::paste_via_clipboard`) restores the
//! previous clipboard after a fixed delay. The paste keystroke is only
//! *enqueued* at that point — the target application reads the clipboard
//! whenever its event loop gets to it, so any fixed delay can lose the race
//! and the user gets their old clipboard pasted back (#502).
//!
//! This module instead publishes the transcript as a *lazy promise* and waits
//! for the operating system to tell us that a consumer actually read the
//! clipboard — a "receipt" — before restoring:
//!
//! - Windows: delayed rendering (`SetClipboardData(CF_UNICODETEXT, NULL)`),
//!   the owner window receives `WM_RENDERFORMAT` on read.
//! - macOS: `declareTypes:owner:` with an owner object, the pasteboard calls
//!   `pasteboard:provideDataForType:` on read.
//!
//! Two rules make the receipt trustworthy:
//!
//! 1. Only receipts observed *after* the paste chord was injected count. A
//!    read before that is an eager third party (clipboard manager, antivirus)
//!    reacting to the clipboard change itself.
//! 2. Restoration only happens while we still own the clipboard
//!    (sequence number / changeCount unchanged, no ownership-lost event). If
//!    the user copied something else in the meantime, their action wins.
//!
//! The restore is additionally gated on a short quiet period after the *last*
//! receipt, because some applications read the clipboard several times per
//! paste (Chromium probes, then reads). A bounded timeout caps how long the
//! transcript may occupy the clipboard; the failure mode is always "the
//! transcript stays on the clipboard a bit longer", never "stale content gets
//! pasted".

// The shared transaction state is compiled on all platforms (for the unit
// tests), but only the macOS/Windows platform modules consume all of it.
#![cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]

use std::time::{Duration, Instant};

/// 描述是否可能已经向目标发键；剪贴板读取不构成输入回执。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryPasteOutcome {
    NotDispatched(String),
    PossiblyDispatched(String),
    Dispatched,
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(crate) struct PasteCompletion {
    pub auto_submit: bool,
    pub auto_submit_key: crate::settings::AutoSubmitKey,
    pub clipboard_handling: crate::settings::ClipboardHandling,
}

/// 在最后副作用边界检查目标/组合状态/策略。发键错误可能发生在部分发键后。
pub(crate) fn guarded_dispatch(
    validate: &mut dyn FnMut() -> Result<(), String>,
    dispatch: impl FnOnce() -> Result<(), String>,
) -> HistoryPasteOutcome {
    if let Err(error) = validate() {
        return HistoryPasteOutcome::NotDispatched(error);
    }
    match dispatch() {
        Ok(()) => HistoryPasteOutcome::Dispatched,
        Err(error) => HistoryPasteOutcome::PossiblyDispatched(error),
    }
}

/// 所有 item 的所有原始表示；不进行文本/图像优先级选择或格式转换。
#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ClipboardSnapshot {
    pub change_count: isize,
    pub items: Vec<Vec<(String, Vec<u8>)>>,
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn capture_snapshot(
    count: impl Fn() -> isize,
    read: impl FnOnce() -> Result<Vec<Vec<(String, Vec<u8>)>>, String>,
) -> Result<ClipboardSnapshot, String> {
    let change_count = count();
    let items = read()?;
    if count() != change_count {
        return Err("clipboard changed while materializing snapshot".into());
    }
    Ok(ClipboardSnapshot {
        change_count,
        items,
    })
}

#[cfg(any(target_os = "macos", test))]
pub(crate) fn restore_if_owned(
    expected: isize,
    count: impl FnOnce() -> isize,
    restore: impl FnOnce() -> Result<(), String>,
) -> Result<bool, String> {
    if count() != expected {
        return Ok(false);
    }
    restore()?;
    Ok(true)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(crate) fn paste_history(
    text: &str,
    app: &tauri::AppHandle,
    method: &crate::settings::PasteMethod,
    enigo: &mut enigo::Enigo,
    validate: &mut dyn FnMut() -> Result<(), String>,
) -> HistoryPasteOutcome {
    platform::run_history(text, app, method, enigo, validate)
}

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "macos")]
pub(crate) use macos::private_pasteboard_self_check;
#[cfg(target_os = "macos")]
pub(crate) use macos::publish_snapshot;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(target_os = "windows")]
use windows as platform;

/// How long after the *last* observed read the transcript stays on the
/// clipboard before restoring. Covers applications that read the clipboard
/// several times per paste (e.g. Chromium probe-then-read).
pub(crate) const QUIET_PERIOD: Duration = Duration::from_millis(200);

/// Upper bound on how long the transcript may occupy the clipboard before we
/// restore regardless of receipts. Long enough that a realistically loaded
/// target always gets to read first; short enough that a lost keystroke does
/// not strand the transcript on the clipboard for long.
pub(crate) const RESTORE_TIMEOUT: Duration = Duration::from_secs(8);

/// When the chord could not be injected at all, no legitimate receipt can
/// arrive, so restore quickly instead of waiting out the full timeout.
pub(crate) const FAILED_INJECTION_TIMEOUT: Duration = Duration::from_millis(500);

/// Shared, cross-thread record of one paste transaction.
#[derive(Debug)]
pub(crate) struct TxState {
    /// When the transcript was published to the clipboard.
    pub published_at: Instant,
    /// When the paste chord was injected. Only receipts *after* this count as
    /// evidence the target read the transcript — earlier reads are eager third
    /// parties reacting to the clipboard change itself.
    pub injected_at: Option<Instant>,
    /// The chord could not be sent; short-circuit the wait.
    pub injection_failed: bool,
    /// Times at which a consumer requested the clipboard data.
    pub receipts: Vec<Instant>,
    /// Someone else took clipboard ownership (user copied elsewhere, ...).
    pub ownership_lost: bool,
    /// A newer paste transaction settled this one early (see flush logic in
    /// the platform modules).
    pub cancelled: bool,
    /// The post-paste Enter (auto-submit) has been sent for this transaction.
    /// (Read on Windows; the macOS path settles via `MacPending::settled`.)
    #[allow(dead_code)]
    pub auto_submit_sent: bool,
    /// First post-injection receipt has been logged.
    pub logged_receipt: bool,
}

impl TxState {
    pub fn new() -> Self {
        Self {
            published_at: Instant::now(),
            injected_at: None,
            injection_failed: false,
            receipts: Vec::new(),
            ownership_lost: false,
            cancelled: false,
            auto_submit_sent: false,
            logged_receipt: false,
        }
    }

    /// Records a read receipt, logging the first one that counts as evidence.
    pub fn record_receipt(&mut self, at: Instant) {
        self.receipts.push(at);
        if !self.logged_receipt {
            if let Some(injected) = self.injected_at {
                if at >= injected {
                    self.logged_receipt = true;
                    log::info!(
                        "[reliable-paste] clipboard read {}ms after chord",
                        at.duration_since(injected).as_millis()
                    );
                }
            }
        }
    }

    pub fn last_receipt_after_injection(&self) -> Option<Instant> {
        let injected = self.injected_at?;
        self.receipts.iter().copied().rev().find(|t| *t >= injected)
    }

    pub fn any_receipt_after_injection(&self) -> bool {
        self.last_receipt_after_injection().is_some()
    }
}

pub(crate) enum WaitDecision {
    KeepWaiting,
    /// Stop waiting; settle the transaction (auto-submit + guarded restore).
    Finish,
}

/// Pure decision: given the current transaction state, keep waiting for the
/// target to read, or finish now. Both platform event loops call this.
pub(crate) fn evaluate(state: &TxState, now: Instant) -> WaitDecision {
    if state.ownership_lost || state.cancelled {
        return WaitDecision::Finish;
    }
    if let Some(last) = state.last_receipt_after_injection() {
        if now.duration_since(last) >= QUIET_PERIOD {
            return WaitDecision::Finish;
        }
    }
    let deadline = if state.injection_failed {
        FAILED_INJECTION_TIMEOUT
    } else {
        RESTORE_TIMEOUT
    };
    if now.duration_since(state.published_at) >= deadline {
        return WaitDecision::Finish;
    }
    WaitDecision::KeepWaiting
}

/// Modifier hold for the receipt-sequenced chord, kept at parity with the
/// legacy path (100ms) for the beta: that hold was added in #165 because real
/// users' systems dropped chords released too quickly, and the beta should
/// validate the receipt mechanism without changing a second variable.
///
/// Once receipts are proven in the field this becomes a safe tuning knob — a
/// chord the target never recognizes produces no receipt and is logged ("no
/// read within timeout") rather than failing silently, so a shorter hold
/// (measured working at 10ms on a fast machine, cutting visible latency from
/// ~110ms to ~20ms) can be tried as its own experiment later.
const CHORD_HOLD_MS: u64 = 100;

/// Sends the platform paste chord for the configured method.
pub(crate) fn send_chord(
    enigo: &mut enigo::Enigo,
    paste_method: &crate::settings::PasteMethod,
) -> Result<(), String> {
    use crate::settings::PasteMethod;
    match paste_method {
        PasteMethod::CtrlV => crate::input::send_paste_ctrl_v(enigo, CHORD_HOLD_MS),
        PasteMethod::CtrlShiftV => crate::input::send_paste_ctrl_shift_v(enigo, CHORD_HOLD_MS),
        PasteMethod::ShiftInsert => crate::input::send_paste_shift_insert(enigo, CHORD_HOLD_MS),
        other => Err(format!(
            "Invalid paste method for clipboard paste: {:?}",
            other
        )),
    }
}

/// Attempts the receipt-sequenced paste. Returns `Err` before anything has
/// been published when the platform transaction cannot start, in which case
/// the caller should fall back to the legacy paste path. On `Ok`, publishing
/// and chord injection have completed and the guarded restore (plus
/// auto-submit) finishes asynchronously.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub(crate) fn try_reliable_paste(
    text: &str,
    app_handle: &tauri::AppHandle,
    paste_method: &crate::settings::PasteMethod,
    enigo: &mut enigo::Enigo,
    auto_submit: bool,
    auto_submit_key: crate::settings::AutoSubmitKey,
    clipboard_handling: crate::settings::ClipboardHandling,
) -> Result<(), String> {
    platform::run(
        text,
        app_handle,
        paste_method,
        enigo,
        auto_submit,
        auto_submit_key,
        clipboard_handling,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_snapshot_keeps_all_items_and_formats_including_empty_bytes() {
        let items = vec![
            vec![
                ("public.utf8-plain-text".into(), b"report".to_vec()),
                ("public.html".into(), b"<b>report</b>".to_vec()),
                ("public.rtf".into(), b"{\\rtf1 report}".to_vec()),
                ("custom.zero-length".into(), Vec::new()),
            ],
            vec![
                ("public.file-url".into(), b"file:///tmp/report.pdf".to_vec()),
                ("public.png".into(), vec![0, 255, 3, 4]),
            ],
        ];
        let snapshot = capture_snapshot(|| 7, || Ok(items.clone())).unwrap();
        assert_eq!(snapshot.items, items);
        assert_eq!(snapshot.change_count, 7);
    }

    #[test]
    fn history_snapshot_does_not_treat_unavailable_data_as_empty() {
        assert!(capture_snapshot(|| 7, || Err("unavailable promise".into())).is_err());
    }

    #[test]
    fn history_snapshot_rejects_user_copy_during_materialization() {
        let count = std::cell::Cell::new(7);
        let snapshot = capture_snapshot(
            || count.get(),
            || {
                count.set(8);
                Ok(Vec::new())
            },
        );
        assert!(snapshot.is_err());
    }

    #[test]
    fn history_restore_does_not_write_after_new_user_copy() {
        let restored = std::cell::Cell::new(false);
        assert!(!restore_if_owned(
            7,
            || 8,
            || {
                restored.set(true);
                Ok(())
            }
        )
        .unwrap());
        assert!(!restored.get());
    }

    #[test]
    fn history_restore_preserves_a_genuinely_empty_clipboard() {
        let saved = capture_snapshot(|| 7, || Ok(Vec::new())).unwrap();
        let board =
            std::cell::RefCell::new(vec![vec![("public.text".into(), b"temporary".to_vec())]]);
        assert!(restore_if_owned(
            8,
            || 8,
            || {
                *board.borrow_mut() = saved.items;
                Ok(())
            }
        )
        .unwrap());
        assert!(board.borrow().is_empty());
    }

    #[test]
    fn history_restore_failure_is_not_swallowed() {
        assert_eq!(
            restore_if_owned(8, || 8, || Err("write failed".into())),
            Err("write failed".into())
        );
    }

    #[test]
    fn history_last_boundary_rejects_target_change_after_slow_snapshot() {
        let target_valid = std::cell::Cell::new(true);
        let sent = std::cell::Cell::new(false);
        let _ = capture_snapshot(
            || 7,
            || {
                target_valid.set(false);
                Ok(Vec::new())
            },
        )
        .unwrap();
        let result = guarded_dispatch(
            &mut || {
                if target_valid.get() {
                    Ok(())
                } else {
                    Err("target changed".into())
                }
            },
            || {
                sent.set(true);
                Ok(())
            },
        );
        assert_eq!(
            result,
            HistoryPasteOutcome::NotDispatched("target changed".into())
        );
        assert!(!sent.get());
    }

    #[test]
    fn history_partial_injection_error_is_uncertain_and_never_retried() {
        let attempts = std::cell::Cell::new(0);
        let result = guarded_dispatch(&mut || Ok(()), || {
            attempts.set(attempts.get() + 1);
            Err("key release failed after key press".into())
        });
        assert_eq!(
            result,
            HistoryPasteOutcome::PossiblyDispatched("key release failed after key press".into())
        );
        assert_eq!(attempts.get(), 1);
    }

    #[test]
    fn history_chord_success_is_dispatch_only_not_a_target_receipt() {
        assert_eq!(
            guarded_dispatch(&mut || Ok(()), || Ok(())),
            HistoryPasteOutcome::Dispatched
        );
    }

    fn state_after_publish(published_ago: Duration) -> TxState {
        let mut s = TxState::new();
        s.published_at = Instant::now() - published_ago;
        s
    }

    #[test]
    fn keeps_waiting_without_receipt_within_timeout() {
        let s = state_after_publish(Duration::from_millis(100));
        assert!(matches!(
            evaluate(&s, Instant::now()),
            WaitDecision::KeepWaiting
        ));
    }

    #[test]
    fn finishes_after_quiet_period_once_read() {
        let mut s = state_after_publish(Duration::from_millis(300));
        s.injected_at = Some(Instant::now() - Duration::from_millis(250));
        s.receipts.push(Instant::now() - QUIET_PERIOD);
        assert!(matches!(evaluate(&s, Instant::now()), WaitDecision::Finish));
    }

    #[test]
    fn waits_through_quiet_period_after_recent_read() {
        let mut s = state_after_publish(Duration::from_millis(300));
        s.injected_at = Some(Instant::now() - Duration::from_millis(100));
        s.receipts.push(Instant::now() - Duration::from_millis(50));
        assert!(matches!(
            evaluate(&s, Instant::now()),
            WaitDecision::KeepWaiting
        ));
    }

    #[test]
    fn pre_injection_receipt_does_not_count() {
        let mut s = state_after_publish(Duration::from_millis(300));
        s.receipts.push(Instant::now() - Duration::from_millis(200));
        s.injected_at = Some(Instant::now() - Duration::from_millis(100));
        assert!(!s.any_receipt_after_injection());
        assert!(matches!(
            evaluate(&s, Instant::now()),
            WaitDecision::KeepWaiting
        ));
    }

    #[test]
    fn finishes_on_timeout_without_receipt() {
        let s = state_after_publish(RESTORE_TIMEOUT);
        assert!(matches!(evaluate(&s, Instant::now()), WaitDecision::Finish));
    }

    #[test]
    fn failed_injection_uses_short_timeout() {
        let mut s = state_after_publish(FAILED_INJECTION_TIMEOUT);
        s.injection_failed = true;
        assert!(matches!(evaluate(&s, Instant::now()), WaitDecision::Finish));
    }

    #[test]
    fn ownership_loss_finishes_immediately() {
        let mut s = state_after_publish(Duration::from_millis(10));
        s.ownership_lost = true;
        assert!(matches!(evaluate(&s, Instant::now()), WaitDecision::Finish));
    }
}
