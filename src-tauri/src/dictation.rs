//! 普通口述采用系统粘贴语义；历史投影和精确字段租约不承担输入准入。

use crate::dictation_target::DictationTarget;
use crate::integration_output::AutoSubmitTarget;
use crate::paste_tx::{DictationPastePhase, HistoryPasteOutcome};
use crate::settings::{ClipboardHandling, DictationTargetMode, PasteMethod};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;
use tauri::AppHandle;

#[derive(Clone)]
pub(crate) struct DictationContext {
    operation_id: String,
    claimed: Arc<AtomicBool>,
    target: Result<DictationTarget, String>,
    permission_epoch: Result<u64, String>,
    auto_submit: Option<AutoSubmitTarget>,
}

impl DictationContext {
    pub(crate) fn capture(app: &AppHandle, auto_submit: bool) -> Self {
        let permission_epoch = crate::input_permission::capture_epoch();
        let mode = crate::settings::get_settings(app).dictation_target_mode;
        let target = match mode {
            // 默认模式不捕获开始时的前台，也不因主线程捕获排队而失效。
            DictationTargetMode::CurrentCursor => DictationTarget::capture(mode),
            DictationTargetMode::RecordingApplication => {
                crate::integration_output::main_thread_call(app, move || {
                    DictationTarget::capture(mode)
                })
                .and_then(|target| target)
            }
        };
        // 可选 AX 证明单独失败/超时不能丢弃已捕获的普通应用上下文。
        let auto_submit = if auto_submit {
            let submit_app = app.clone();
            crate::integration_output::main_thread_call(app, move || {
                AutoSubmitTarget::capture(submit_app)
            })
            .ok()
            .flatten()
        } else {
            None
        };
        Self {
            operation_id: format!("dictation-{}", uuid::Uuid::new_v4()),
            claimed: Arc::new(AtomicBool::new(false)),
            target,
            permission_epoch,
            auto_submit,
        }
    }
}

#[derive(Clone, serde::Serialize)]
pub(crate) struct DictationReceipt {
    pub operation_id: String,
    pub state: &'static str,
    pub history_id: Option<i64>,
    pub history_saved: bool,
}

fn receipt_state(outcome: &HistoryPasteOutcome, copy_only: bool) -> &'static str {
    match outcome {
        HistoryPasteOutcome::Dispatched if copy_only => "confirmed",
        HistoryPasteOutcome::Dispatched => "dispatched",
        HistoryPasteOutcome::NotDispatched(_) => "pending_target",
        HistoryPasteOutcome::PossiblyDispatched(_) => "uncertain",
    }
}

#[derive(Clone, Copy)]
enum DiagnosticStage {
    Context,
    Claim,
    Policy,
    PermissionCapture,
    PermissionCheck,
    TargetCapture,
    PreparationQueue,
    Preparation,
    Delay,
    DispatchQueue,
    Dispatch,
}

impl DiagnosticStage {
    fn label(self) -> &'static str {
        match self {
            Self::Context => "context",
            Self::Claim => "claim",
            Self::Policy => "policy",
            Self::PermissionCapture => "permission_capture",
            Self::PermissionCheck => "permission_check",
            Self::TargetCapture => "target_capture",
            Self::PreparationQueue => "preparation_queue",
            Self::Preparation => "preparation",
            Self::Delay => "delay",
            Self::DispatchQueue => "dispatch_queue",
            Self::Dispatch => "dispatch",
        }
    }
}

/// 诊断只允许固定代码；底层错误可能含脚本 stderr、转写正文或剪贴板格式名。
pub(crate) fn diagnostic_reason(reason: &str) -> &'static str {
    match reason {
        "dictation_context_unavailable" => "context_unavailable",
        "dictation_already_claimed" => "already_claimed",
        "dictation_cancelled" => "cancelled",
        "automatic insertion disabled" => "insertion_disabled",
        "input_permission_epoch_invalid" => "permission_epoch_invalid",
        "input_permission_not_verified" => "permission_not_verified",
        "dictation_target_requires_main_thread" => "target_requires_main_thread",
        "dictation_foreground_unavailable" => "foreground_unavailable",
        "dictation_process_unavailable" => "process_unavailable",
        "dictation_secure_input" => "secure_input",
        "dictation_foreground_is_inputia" => "foreground_is_inputia",
        "dictation_recording_application_not_foreground" => "recording_application_not_foreground",
        "output task cancelled before starting" => "capture_queue_cancelled",
        "output task response is unknown" => "capture_queue_unknown",
        "dictation_queue_cancelled" => "queue_cancelled",
        "dictation_queue_unavailable" => "queue_unavailable",
        "dictation_dispatch_unknown" => "dispatch_unknown",
        "dictation_completion_unavailable" => "completion_unavailable",
        "dictation_preparation_unavailable" => "preparation_unavailable",
        "dictation_injection_already_attempted" => "injection_already_attempted",
        "input device unavailable" | "Enigo state not initialized" => "input_device_unavailable",
        "input device busy" => "input_device_busy",
        "input device retired" | "input_device_retired" => "input_device_retired",
        "external script is not configured" => "external_script_not_configured",
        "clipboard transaction busy" => "clipboard_transaction_busy",
        "clipboard transaction pending" | "standard clipboard transaction pending" => {
            "clipboard_transaction_pending"
        }
        "dictation_clipboard_items_unavailable"
        | "pasteboard items unavailable for non-empty clipboard" => "clipboard_items_unavailable",
        "dictation_clipboard_item_not_recoverable" => "clipboard_item_not_recoverable",
        "clipboard changed while materializing snapshot" => "clipboard_changed_during_snapshot",
        "clipboard changed during copy preparation" => "clipboard_changed_during_preparation",
        "clipboard changed during permission check" => "clipboard_changed_during_permission_check",
        "clipboard changed before publish" => "clipboard_changed_before_publish",
        "dictation_clipboard_changed_before_paste" | "clipboard changed before paste chord" => {
            "clipboard_changed_before_paste"
        }
        "clipboard publication outcome unknown" => "clipboard_publication_unknown",
        "declareTypes:owner: failed" => "clipboard_publish_failed",
        "dispatched" => "dispatched",
        "copied" => "copied",
        _ if reason.starts_with("clipboard representation unavailable: ") => {
            "clipboard_representation_unavailable"
        }
        _ if reason.starts_with("cannot prepare clipboard representation: ") => {
            "clipboard_preparation_failed"
        }
        _ if reason.starts_with("clipboard restore failed: ") => "clipboard_restore_failed",
        _ => "unclassified_error",
    }
}

fn finish_receipt(
    receipt: DictationReceipt,
    stage: DiagnosticStage,
    reason: &str,
) -> DictationReceipt {
    log::info!(
        "dictation_output_result state={} history_saved={} stage={} reason={}",
        receipt.state,
        receipt.history_saved,
        stage.label(),
        diagnostic_reason(reason),
    );
    receipt
}

fn claim_once(claimed: &AtomicBool) -> bool {
    !claimed.swap(true, Ordering::AcqRel)
}

pub(crate) fn wait_for_delay(
    mut remaining: Duration,
    mut check: impl FnMut() -> Result<(), String>,
    mut sleep: impl FnMut(Duration),
) -> Result<(), String> {
    loop {
        check()?;
        if remaining.is_zero() {
            return Ok(());
        }
        let interval = remaining.min(Duration::from_millis(25));
        sleep(interval);
        remaining = remaining.saturating_sub(interval);
    }
}

pub(crate) fn await_dispatch<T>(
    receiver: mpsc::Receiver<T>,
    gate: &crate::dispatch_gate::DispatchGate,
    timeout: Duration,
) -> Result<T, HistoryPasteOutcome> {
    match receiver.recv_timeout(timeout) {
        Ok(result) => Ok(result),
        Err(_) if gate.cancel_pending() => Err(HistoryPasteOutcome::NotDispatched(
            "dictation_queue_cancelled".into(),
        )),
        Err(mpsc::RecvTimeoutError::Timeout) => receiver.recv().map_err(|_| {
            HistoryPasteOutcome::PossiblyDispatched("dictation_dispatch_unknown".into())
        }),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(HistoryPasteOutcome::PossiblyDispatched(
            "dictation_dispatch_unknown".into(),
        )),
    }
}

/// 仅未开始的排队任务有截止时间；开始后的原生操作不因人为短租约失效或重试。
fn dispatch_on_main_thread<T: Send + 'static>(
    app: &AppHandle,
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, HistoryPasteOutcome> {
    let gate = crate::dispatch_gate::DispatchGate::new(Duration::from_secs(3));
    let queued = gate.clone();
    let (sender, receiver) = mpsc::sync_channel(1);
    if app
        .run_on_main_thread(move || {
            if queued.start() {
                let _ = sender.send(work());
            }
        })
        .is_err()
    {
        return Err(HistoryPasteOutcome::NotDispatched(
            "dictation_queue_unavailable".into(),
        ));
    }
    await_dispatch(receiver, &gate, Duration::from_secs(3))
}

/// 本次转写只派发一次；历史保存失败单独呈现，不替换本次用户的输入意图。
pub(crate) fn dispatch_result(
    app: AppHandle,
    context: Option<DictationContext>,
    history_id: Option<i64>,
    text: String,
    is_cancelled: Arc<dyn Fn() -> bool + Send + Sync>,
) -> DictationReceipt {
    let settings = crate::settings::get_settings(&app);
    let copy_only = settings.paste_method == PasteMethod::None
        && settings.clipboard_handling == ClipboardHandling::CopyToClipboard;
    let mut receipt = DictationReceipt {
        operation_id: context
            .as_ref()
            .map(|context| context.operation_id.clone())
            .unwrap_or_else(|| format!("dictation-{}", uuid::Uuid::new_v4())),
        state: "pending_target",
        history_id,
        history_saved: history_id.is_some(),
    };
    let Some(context) = context else {
        return finish_receipt(
            receipt,
            DiagnosticStage::Context,
            "dictation_context_unavailable",
        );
    };
    if !claim_once(&context.claimed) {
        receipt.state = "uncertain";
        return finish_receipt(receipt, DiagnosticStage::Claim, "dictation_already_claimed");
    }
    if is_cancelled() {
        receipt.state = "rejected";
        return finish_receipt(receipt, DiagnosticStage::Policy, "dictation_cancelled");
    }
    if settings.paste_method == PasteMethod::None && !copy_only {
        receipt.state = "rejected";
        return finish_receipt(
            receipt,
            DiagnosticStage::Policy,
            "automatic insertion disabled",
        );
    }
    let permission_epoch = match context.permission_epoch {
        Ok(epoch) => epoch,
        Err(reason) => {
            return finish_receipt(receipt, DiagnosticStage::PermissionCapture, &reason);
        }
    };
    if let Err(reason) = crate::input_permission::check_epoch(permission_epoch) {
        return finish_receipt(receipt, DiagnosticStage::PermissionCheck, &reason);
    }
    if !copy_only {
        if let Err(reason) = &context.target {
            return finish_receipt(receipt, DiagnosticStage::TargetCapture, reason);
        }
    }
    let dispatch_app = app.clone();
    let completion = Arc::new(Mutex::new(None));
    let complete_later = completion.clone();
    let delayed_cancel = is_cancelled.clone();
    let phase = dispatch_on_main_thread(&app, move || {
        let target = context.target;
        let boundary = move || {
            crate::input_permission::check_epoch(permission_epoch)?;
            if is_cancelled() {
                return Err("dictation_cancelled".into());
            }
            if !copy_only {
                target.as_ref().map_err(Clone::clone)?.validate()?;
            }
            if is_cancelled() {
                return Err("dictation_cancelled".into());
            }
            crate::input_permission::check_epoch(permission_epoch)
        };
        let final_text = if settings.append_trailing_space {
            format!("{text} ")
        } else {
            text
        };
        if copy_only {
            return DictationPastePhase::Finished(crate::clipboard::copy_voice_text(
                &final_text,
                &dispatch_app,
                &mut || boundary(),
            ));
        }
        let submit_boundary = boundary.clone();
        let clipboard_boundary = boundary.clone();
        let submit = context.auto_submit;
        let Ok(mut completion) = complete_later.lock() else {
            return DictationPastePhase::Finished(HistoryPasteOutcome::NotDispatched(
                "dictation_completion_unavailable".into(),
            ));
        };
        crate::clipboard::paste_dictation_text(
            &final_text,
            &dispatch_app,
            &settings,
            &mut || boundary(),
            crate::paste_tx::VoiceCompletionGuards {
                auto_submit: Box::new(move || {
                    submit_boundary()?;
                    submit
                        .as_ref()
                        .ok_or("auto_submit_target_unavailable")?
                        .validate()?;
                    submit_boundary()
                }),
                clipboard: Box::new(clipboard_boundary),
            },
            &mut completion,
        )
    });
    let (stage, result) = match phase {
        Err(outcome) => (DiagnosticStage::PreparationQueue, outcome),
        Ok(DictationPastePhase::Finished(outcome)) => (DiagnosticStage::Preparation, outcome),
        Ok(DictationPastePhase::Prepared) => {
            let delay = completion
                .lock()
                .ok()
                .and_then(|pending| pending.as_ref().map(|pending| pending.before_delay()));
            match delay {
                None => (
                    DiagnosticStage::Preparation,
                    HistoryPasteOutcome::NotDispatched("dictation_preparation_unavailable".into()),
                ),
                Some(delay) => {
                    let ready = wait_for_delay(
                        delay,
                        || {
                            if delayed_cancel() {
                                return Err("dictation_cancelled".into());
                            }
                            crate::input_permission::check_epoch(permission_epoch)
                        },
                        std::thread::sleep,
                    );
                    if let Err(reason) = ready {
                        (
                            DiagnosticStage::Delay,
                            HistoryPasteOutcome::NotDispatched(reason),
                        )
                    } else {
                        let inject_app = app.clone();
                        let inject_pending = completion.clone();
                        let injection = dispatch_on_main_thread(&app, move || {
                            let Ok(mut pending) = inject_pending.lock() else {
                                return HistoryPasteOutcome::NotDispatched(
                                    "dictation_completion_unavailable".into(),
                                );
                            };
                            let Some(pending) = pending.as_mut() else {
                                return HistoryPasteOutcome::NotDispatched(
                                    "dictation_preparation_unavailable".into(),
                                );
                            };
                            pending.inject(&inject_app)
                        });
                        match injection {
                            Ok(outcome) => (DiagnosticStage::Dispatch, outcome),
                            Err(outcome) => (DiagnosticStage::DispatchQueue, outcome),
                        }
                    }
                }
            }
        }
    };
    if let Ok(mut pending) = completion.lock() {
        if let Some(pending) = pending.as_mut() {
            pending.record_outcome(&result);
        }
    }
    let pending = completion
        .lock()
        .ok()
        .and_then(|mut pending| pending.take());
    if let Some(pending) = pending {
        // 已发出的 Cmd+V 可能仍在排队读取；取消只阻止 Enter，不能提前恢复旧内容。
        let _ = wait_for_delay(pending.delay(), || Ok(()), std::thread::sleep);
        let (sender, receiver) = mpsc::sync_channel(1);
        let finish_app = app.clone();
        let queued = app.run_on_main_thread(move || {
            let _ = sender.send(pending.finish(&finish_app));
        });
        // 清理不是新一次文本派发，不按三秒业务截止时间丢弃旧剪贴板快照。
        if queued.is_err() || !matches!(receiver.recv(), Ok(Ok(()))) {
            log::warn!("dictation_clipboard_completion_unavailable");
        }
    }
    receipt.state = receipt_state(&result, copy_only);
    let reason = match &result {
        HistoryPasteOutcome::Dispatched if copy_only => "copied",
        HistoryPasteOutcome::Dispatched => "dispatched",
        HistoryPasteOutcome::NotDispatched(reason)
        | HistoryPasteOutcome::PossiblyDispatched(reason) => reason,
    };
    finish_receipt(receipt, stage, reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_long_delay_has_no_short_lease_and_cancel_interrupts_it() {
        use std::cell::Cell;
        let elapsed = Cell::new(Duration::ZERO);
        wait_for_delay(
            Duration::from_secs(121),
            || Ok(()),
            |d| elapsed.set(elapsed.get() + d),
        )
        .unwrap();
        assert_eq!(elapsed.get(), Duration::from_secs(121));
        elapsed.set(Duration::ZERO);
        let stopped = wait_for_delay(
            Duration::from_secs(121),
            || {
                if elapsed.get() >= Duration::from_millis(50) {
                    Err("cancelled".into())
                } else {
                    Ok(())
                }
            },
            |d| elapsed.set(elapsed.get() + d),
        );
        assert!(stopped.is_err());
        assert_eq!(elapsed.get(), Duration::from_millis(50));
    }

    #[test]
    fn disconnected_or_timed_out_queue_does_not_replay_or_misreport_unstarted_work() {
        for started in [false, true] {
            let gate = crate::dispatch_gate::DispatchGate::new(Duration::from_secs(3));
            if started {
                assert!(gate.start());
            }
            let (sender, receiver) = mpsc::channel::<()>();
            drop(sender);
            let result = await_dispatch(receiver, &gate, Duration::ZERO).unwrap_err();
            assert_eq!(
                matches!(result, HistoryPasteOutcome::PossiblyDispatched(_)),
                started
            );
            assert!(!gate.start());
        }
        let gate = crate::dispatch_gate::DispatchGate::new(Duration::from_secs(3));
        let (_sender, receiver) = mpsc::channel::<()>();
        assert!(matches!(
            await_dispatch(receiver, &gate, Duration::ZERO),
            Err(HistoryPasteOutcome::NotDispatched(_))
        ));
        assert!(!gate.start());
    }

    #[test]
    fn a_dictation_result_can_be_claimed_once_even_without_saved_history() {
        let claimed = AtomicBool::new(false);
        assert!(claim_once(&claimed));
        assert!(!claim_once(&claimed));
        assert!(!claim_once(&claimed));
        assert_eq!(
            receipt_state(&HistoryPasteOutcome::Dispatched, false),
            "dispatched"
        );
    }

    #[test]
    fn unknown_dispatch_never_becomes_a_confirmed_insertion() {
        assert_eq!(
            receipt_state(
                &HistoryPasteOutcome::PossiblyDispatched("fixture".into()),
                false
            ),
            "uncertain"
        );
        assert_eq!(
            receipt_state(&HistoryPasteOutcome::NotDispatched("fixture".into()), false),
            "pending_target"
        );
        assert_eq!(
            receipt_state(&HistoryPasteOutcome::Dispatched, true),
            "confirmed"
        );
    }

    #[test]
    fn diagnostics_distinguish_target_clipboard_permission_and_queue_failures() {
        let cases = [
            ("dictation_context_unavailable", "context_unavailable"),
            ("dictation_already_claimed", "already_claimed"),
            ("dictation_cancelled", "cancelled"),
            ("automatic insertion disabled", "insertion_disabled"),
            ("input_permission_epoch_invalid", "permission_epoch_invalid"),
            ("dictation_foreground_unavailable", "foreground_unavailable"),
            ("dictation_secure_input", "secure_input"),
            ("dictation_foreground_is_inputia", "foreground_is_inputia"),
            (
                "dictation_recording_application_not_foreground",
                "recording_application_not_foreground",
            ),
            (
                "dictation_clipboard_changed_before_paste",
                "clipboard_changed_before_paste",
            ),
            ("dictation_queue_cancelled", "queue_cancelled"),
            ("dictation_dispatch_unknown", "dispatch_unknown"),
        ];
        for (reason, diagnostic) in cases {
            assert_eq!(diagnostic_reason(reason), diagnostic);
        }
    }

    #[test]
    fn diagnostics_never_echo_transcripts_clipboard_formats_or_script_errors() {
        for reason in [
            "脚本 stderr: privacy-fixture-transcript",
            "dictation_secure_input\nprivacy-fixture-transcript",
            "unknown native error: privacy-fixture-transcript",
        ] {
            assert_eq!(diagnostic_reason(reason), "unclassified_error");
        }
        for (prefix, expected) in [
            (
                "clipboard representation unavailable: ",
                "clipboard_representation_unavailable",
            ),
            (
                "cannot prepare clipboard representation: ",
                "clipboard_preparation_failed",
            ),
            ("clipboard restore failed: ", "clipboard_restore_failed"),
        ] {
            let reason = format!("{prefix}private.format-name\nprivacy-fixture-transcript");
            assert_eq!(diagnostic_reason(&reason), expected);
        }
    }

    #[test]
    fn diagnostic_logging_does_not_change_the_frontend_receipt_shape() {
        let receipt = finish_receipt(
            DictationReceipt {
                operation_id: "dictation-fixture".into(),
                state: "pending_target",
                history_id: None,
                history_saved: false,
            },
            DiagnosticStage::Dispatch,
            "dictation_secure_input",
        );
        assert_eq!(
            serde_json::to_value(receipt).unwrap(),
            serde_json::json!({
                "operation_id": "dictation-fixture",
                "state": "pending_target",
                "history_id": null,
                "history_saved": false,
            })
        );
    }
}
