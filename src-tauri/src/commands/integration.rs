use crate::integration_output::{guarded_main_thread_call, main_thread_call, MainThreadError};
use crate::managers::clipboard::ClipboardManager;
use crate::managers::integration::IntegrationManager;
use inputia_core::integration::events::Identifier;
use inputia_handy_runtime::learning::{ApplyContribution, HistoryTermConfirmation};
use inputia_handy_runtime::output_ledger::{
    OutputAction, OutputIntent, OutputOutcome, OutputOwner, OutputState,
};
use inputia_handy_runtime::store::{ContentType, HistoryQuery, IndexedItem, SourceKind};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;
use tauri::{AppHandle, State};

#[derive(Clone, Debug, Serialize, Type)]
pub struct UnifiedOutputResult {
    pub operation_id: String,
    pub status: String,
}

fn validate_term_confirmation(
    label: &str,
    focused: bool,
    confirmed: bool,
    secure_input: bool,
) -> Result<(), String> {
    if label != "main" || !focused || !confirmed || secure_input {
        return Err("term confirmation is unavailable".into());
    }
    Ok(())
}

/// 在控制中心明确确认一个短词；来源身份及策略版本只能从服务端读取。
#[tauri::command]
#[specta::specta]
pub async fn confirm_unified_history_term(
    window: tauri::WebviewWindow,
    manager: State<'_, Arc<IntegrationManager>>,
    item_id: String,
    expected_revision: u64,
    operation_id: String,
    term: String,
    confirmed: bool,
) -> Result<String, String> {
    validate_term_confirmation(
        window.label(),
        window.is_focused().unwrap_or(false),
        confirmed,
        crate::secure_input::is_enabled_now(),
    )?;
    let service = manager.service.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let request = HistoryTermConfirmation {
            operation_id: Identifier::parse(operation_id)?,
            item_id,
            expected_revision,
            term,
        };
        service
            .confirm_history_term(request, move || {
                validate_term_confirmation(
                    window.label(),
                    window.is_focused().unwrap_or(false),
                    confirmed,
                    crate::secure_input::is_enabled_now(),
                )
                .is_ok()
            })
            .map(|result| {
                match result {
                    ApplyContribution::Applied => "applied",
                    ApplyContribution::Replay => "replay",
                    ApplyContribution::AlreadyContributed => "already_contributed",
                }
                .to_owned()
            })
            .map_err(|_| "term contribution was not accepted".to_owned())
    })
    .await
    .map_err(|_| "term confirmation worker failed".to_owned())?
}

/// 回执查询没有输出副作用；不得通过重放 Prepared 命令实现“查看状态”。
#[tauri::command]
#[specta::specta]
pub async fn get_unified_output_receipt(
    manager: State<'_, Arc<IntegrationManager>>,
    operation_id: String,
) -> Result<Option<UnifiedOutputResult>, String> {
    let service = manager.service.clone();
    tauri::async_runtime::spawn_blocking(move || {
        service
            .output_record(operation_id.clone())
            .map(|record| record.map(|record| output_result(operation_id, record.state)))
    })
    .await
    .map_err(|_| "receipt lookup worker failed".to_owned())?
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct UnifiedOutputNotice {
    pub operation_id: String,
    pub item_id: String,
    pub state: String,
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct UnifiedOutputNoticePage {
    pub items: Vec<UnifiedOutputNotice>,
    pub next_cursor: Option<String>,
}

/// 无正文的启动恢复查询；分页不会准备或派发任何输出。
#[tauri::command]
#[specta::specta]
pub async fn list_unresolved_unified_outputs(
    manager: State<'_, Arc<IntegrationManager>>,
    cursor: Option<String>,
    limit: Option<u32>,
) -> Result<UnifiedOutputNoticePage, String> {
    let service = manager.service.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let page = service.unresolved_output_notices(cursor, limit.unwrap_or(50))?;
        Ok(UnifiedOutputNoticePage {
            items: page
                .items
                .into_iter()
                .map(|notice| UnifiedOutputNotice {
                    state: output_result(notice.operation_id.clone(), notice.state).status,
                    operation_id: notice.operation_id,
                    item_id: notice.item_id,
                })
                .collect(),
            next_cursor: page.next_cursor,
        })
    })
    .await
    .map_err(|_| "output notice lookup worker failed".to_owned())?
}

/// 用户关闭提示只记录已读；未知输出仍不可自动重试。
#[tauri::command]
#[specta::specta]
pub async fn acknowledge_unified_output_notice(
    manager: State<'_, Arc<IntegrationManager>>,
    operation_id: String,
    expected_state: String,
) -> Result<(), String> {
    let state = match expected_state.as_str() {
        "pending_target" => OutputState::PendingTarget,
        "uncertain" => OutputState::Uncertain,
        "rejected" => OutputState::Rejected,
        _ => return Err("invalid output notice state".into()),
    };
    let service = manager.service.clone();
    tauri::async_runtime::spawn_blocking(move || {
        service.acknowledge_output_notice(operation_id, state)
    })
    .await
    .map_err(|_| "output notice acknowledgment worker failed".to_owned())?
}

#[tauri::command]
#[specta::specta]
pub async fn retranscribe_unified_history_item(
    app: AppHandle,
    manager: State<'_, Arc<IntegrationManager>>,
    history: State<'_, Arc<crate::managers::history::HistoryManager>>,
    transcription: State<'_, Arc<crate::managers::transcription::TranscriptionManager>>,
    item_id: String,
    expected_revision: u64,
) -> Result<(), String> {
    let service = manager.service.clone();
    let lookup = service.clone();
    let item =
        tauri::async_runtime::spawn_blocking(move || lookup.get_item(item_id, expected_revision))
            .await
            .map_err(|_| "history worker failed".to_owned())??;
    if item.snapshot.source_kind != SourceKind::Voice || item.snapshot.asset_ref.is_none() {
        return Err("recording is unavailable".into());
    }
    let id = item
        .record_id
        .parse::<i64>()
        .map_err(|_| "invalid recording identity".to_owned())?;
    super::history::retry_history_entry_checked(
        app,
        &history,
        &transcription,
        id,
        Some(expected_revision),
    )
    .await?;
    tauri::async_runtime::spawn_blocking(move || service.synchronize())
        .await
        .map_err(|_| "history worker failed".to_owned())??;
    Ok(())
}

fn output_result(operation_id: String, state: OutputState) -> UnifiedOutputResult {
    UnifiedOutputResult {
        operation_id,
        status: match state {
            OutputState::Confirmed => "confirmed",
            OutputState::DispatchedOnly => "dispatched",
            OutputState::PendingTarget => "pending_target",
            OutputState::Rejected => "rejected",
            OutputState::Prepared | OutputState::Dispatched | OutputState::Uncertain => "uncertain",
        }
        .into(),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn insert_unified_history_item(
    app: AppHandle,
    manager: State<'_, Arc<IntegrationManager>>,
    item_id: String,
    expected_revision: u64,
    operation_id: String,
) -> Result<UnifiedOutputResult, String> {
    let service = manager.service.clone();
    tauri::async_runtime::spawn_blocking(move || {
        // 重试时复用已持久化 intent，不能用新焦点重新构造同一 operation。
        let existing = service.output_record(operation_id.clone())?;
        let intent = if let Some(record) = existing {
            if record.intent.item_id != item_id
                || record.intent.revision != expected_revision
                || record.intent.action != OutputAction::InsertText
            {
                return Err("output operation identity conflict".into());
            }
            if record.state != OutputState::Prepared {
                return Ok(output_result(operation_id, record.state));
            }
            record.intent
        } else {
            OutputIntent {
                operation_id: operation_id.clone(),
                item_id: item_id.clone(),
                revision: expected_revision,
                source: Some("integration".into()),
                profile_id: None,
                deadline_at_ms: None,
                target_id: main_thread_call(&app, crate::integration_output::current_target)?,
                owner: OutputOwner::Platform,
                policy_epoch: service.policy_epoch()?,
                action: OutputAction::InsertText,
            }
        };
        service.prepare_output(intent.clone())?;
        let item = service.get_item(item_id, expected_revision)?;
        if item.snapshot.content_type != ContentType::Text {
            let record = service.finish_output(intent, OutputOutcome::Rejected)?;
            return Ok(output_result(operation_id, record.state));
        }
        let Some(target) = intent.target_id.clone() else {
            let record = service.finish_output(intent, OutputOutcome::PendingTarget)?;
            return Ok(output_result(operation_id, record.state));
        };
        let target_check = target.clone();
        let phase = main_thread_call(&app, move || {
            crate::integration_output::validate_target(&target_check)
        })?;
        if matches!(&phase,Err(reason) if reason=="suspended_by_owner") {
            let hide = app.clone();
            main_thread_call(&app, move || {
                crate::overlay::hide_clipboard_overlay(&hide);
                #[cfg(target_os = "macos")]
                if let Some(mtm) = objc2::MainThreadMarker::new() {
                    objc2_app_kit::NSApplication::sharedApplication(mtm).hide(None);
                }
            })?;
            std::thread::sleep(std::time::Duration::from_millis(80));
        } else if phase.is_err() {
            let record = service.finish_output(intent, OutputOutcome::PendingTarget)?;
            return Ok(output_result(operation_id, record.state));
        }
        let check = target.clone();
        let ready = main_thread_call(&app, move || {
            crate::integration_output::validate_target(&check).is_ok()
                && crate::integration_output::platform_composition_clear()
        })?;
        if !ready {
            let record = service.finish_output(intent, OutputOutcome::PendingTarget)?;
            return Ok(output_result(operation_id, record.state));
        }
        let Some(permit) = service.claim_output_with_permit(intent.clone())? else {
            let record = service
                .output_record(operation_id.clone())?
                .ok_or_else(|| "output receipt unavailable".to_owned())?;
            return Ok(output_result(operation_id, record.state));
        };
        let text = item.snapshot.text.unwrap_or_default();
        let output_app = app.clone();
        let dispatched = guarded_main_thread_call(&app, move |gate| {
            let mut validate = || {
                crate::dispatch_gate::validate_output_boundary(
                    || {
                        gate.check()?;
                        permit.check()
                    },
                    || {
                        crate::integration_output::validate_target(&target)?;
                        if !crate::integration_output::platform_composition_clear() {
                            return Err("composition requires IME session".into());
                        }
                        Ok(())
                    },
                )
            };
            let result = crate::clipboard::paste_history_text(&text, &output_app, &mut validate);
            if !matches!(
                result,
                crate::paste_tx::HistoryPasteOutcome::NotDispatched(_)
            ) {
                crate::integration_output::forget_target(&target);
            }
            result
        });
        let outcome = match dispatched {
            Ok(crate::paste_tx::HistoryPasteOutcome::Dispatched) => OutputOutcome::DispatchedOnly,
            Ok(crate::paste_tx::HistoryPasteOutcome::NotDispatched(_))
            | Err(MainThreadError::NotStarted) => OutputOutcome::NotDispatchedPendingTarget,
            Ok(crate::paste_tx::HistoryPasteOutcome::PossiblyDispatched(_))
            | Err(MainThreadError::Unknown) => OutputOutcome::Uncertain,
        };
        let record = service.finish_output(intent, outcome)?;
        Ok(output_result(operation_id, record.state))
    })
    .await
    .map_err(|_| "output worker failed; result may be unknown".to_owned())?
}

#[derive(Clone, Debug, Deserialize, Type)]
pub struct UnifiedHistoryPatch {
    pub starred: Option<bool>,
    pub pinned: Option<bool>,
    pub title: Option<String>,
    pub clear_title: bool,
    pub text: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub async fn update_unified_history_item(
    manager: State<'_, Arc<IntegrationManager>>,
    item_id: String,
    expected_revision: u64,
    operation_id: String,
    patch: UnifiedHistoryPatch,
) -> Result<u64, String> {
    let service = manager.service.clone();
    let patch = inputia_handy_runtime::source::HistoryPatch {
        starred: patch.starred,
        pinned: patch.pinned,
        title: patch.title,
        clear_title: patch.clear_title,
        text: patch.text,
    };
    tauri::async_runtime::spawn_blocking(move || {
        service.update_item(item_id, expected_revision, operation_id, patch)
    })
    .await
    .map_err(|_| "unified history worker failed".to_owned())?
}

#[tauri::command]
#[specta::specta]
pub async fn copy_unified_history_item(
    app: AppHandle,
    manager: State<'_, Arc<IntegrationManager>>,
    clipboard: State<'_, Arc<ClipboardManager>>,
    item_id: String,
    expected_revision: u64,
    operation_id: String,
) -> Result<UnifiedOutputResult, String> {
    copy_unified_history_item_impl(
        app,
        manager,
        clipboard,
        item_id,
        expected_revision,
        operation_id,
        false,
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn delete_unified_history_item(
    manager: State<'_, Arc<IntegrationManager>>,
    item_id: String,
    expected_revision: u64,
    operation_id: String,
) -> Result<bool, String> {
    let service = manager.service.clone();
    tauri::async_runtime::spawn_blocking(move || {
        service.delete_item(item_id, expected_revision, operation_id)
    })
    .await
    .map_err(|_| "history deletion worker failed".to_owned())?
}

#[tauri::command]
#[specta::specta]
pub async fn copy_unified_history_item_as_text(
    app: AppHandle,
    manager: State<'_, Arc<IntegrationManager>>,
    clipboard: State<'_, Arc<ClipboardManager>>,
    item_id: String,
    expected_revision: u64,
    operation_id: String,
) -> Result<UnifiedOutputResult, String> {
    copy_unified_history_item_impl(
        app,
        manager,
        clipboard,
        item_id,
        expected_revision,
        operation_id,
        true,
    )
    .await
}

async fn copy_unified_history_item_impl(
    app: AppHandle,
    manager: State<'_, Arc<IntegrationManager>>,
    clipboard: State<'_, Arc<ClipboardManager>>,
    item_id: String,
    expected_revision: u64,
    operation_id: String,
    as_text: bool,
) -> Result<UnifiedOutputResult, String> {
    let service = manager.service.clone();
    let clipboard = Arc::clone(&clipboard);
    tauri::async_runtime::spawn_blocking(move || {
        let action = if as_text {
            OutputAction::CopyPlainText
        } else {
            OutputAction::Copy
        };
        let intent = if let Some(record) = service.output_record(operation_id.clone())? {
            if record.intent.item_id != item_id
                || record.intent.revision != expected_revision
                || record.intent.action != action
            {
                return Err("output operation identity conflict".into());
            }
            if record.state != OutputState::Prepared {
                return Ok(output_result(operation_id, record.state));
            }
            record.intent
        } else {
            OutputIntent {
                operation_id: operation_id.clone(),
                item_id: item_id.clone(),
                revision: expected_revision,
                source: Some("integration".into()),
                profile_id: None,
                deadline_at_ms: None,
                target_id: None,
                owner: OutputOwner::Platform,
                policy_epoch: service.policy_epoch()?,
                action,
            }
        };
        service.prepare_output(intent.clone())?;
        let item = service.get_item(item_id, expected_revision)?;
        if !as_text
            && matches!(
                item.snapshot.content_type,
                ContentType::Html | ContentType::Rtf
            )
        {
            let record = service.finish_output(intent, OutputOutcome::Rejected)?;
            return Ok(output_result(operation_id, record.state));
        }
        let kind = match item.snapshot.content_type {
            ContentType::Text => "text",
            ContentType::Files => "files",
            ContentType::Image => "image",
            ContentType::Html | ContentType::Rtf if as_text => "text",
            _ => "unsupported",
        };
        let expected_change_count = main_thread_call(&app, || {
            #[cfg(target_os = "macos")]
            {
                Some(objc2_app_kit::NSPasteboard::generalPasteboard().changeCount())
            }
            #[cfg(not(target_os = "macos"))]
            {
                None
            }
        })?;
        let prepared_result = if as_text {
            clipboard.prepare_unified_plain_text(kind, item.snapshot.text)
        } else {
            clipboard.prepare_unified_copy(kind, item.snapshot.text, item.snapshot.asset_ref)
        };
        let prepared = match prepared_result {
            Ok(prepared) => prepared,
            Err(_) => {
                let record = service.finish_output(intent, OutputOutcome::Rejected)?;
                return Ok(output_result(operation_id, record.state));
            }
        };
        let Some(permit) = service.claim_output_with_permit(intent.clone())? else {
            return Ok(output_result(operation_id, OutputState::Uncertain));
        };
        let copied = guarded_main_thread_call(&app, move |gate| {
            clipboard.publish_unified_copy(prepared, expected_change_count, || {
                gate.check()?;
                permit.check()
            })
        });
        use crate::managers::clipboard::ClipboardWriteOutcome;
        let outcome = match copied {
            Ok(ClipboardWriteOutcome::Written) => OutputOutcome::Confirmed,
            Ok(ClipboardWriteOutcome::NotWritten) | Err(MainThreadError::NotStarted) => {
                OutputOutcome::NotDispatchedRejected
            }
            Ok(ClipboardWriteOutcome::Unknown) | Err(MainThreadError::Unknown) => {
                OutputOutcome::Uncertain
            }
        };
        let record = service.finish_output(intent, outcome)?;
        Ok(output_result(operation_id, record.state))
    })
    .await
    .map_err(|_| "unified history worker failed".to_owned())?
}

#[tauri::command]
#[specta::specta]
pub async fn get_unified_history_asset(
    app: AppHandle,
    manager: State<'_, Arc<IntegrationManager>>,
    item_id: String,
    expected_revision: u64,
) -> Result<Option<String>, String> {
    let service = manager.service.clone();
    let root =
        crate::portable::app_data_dir(&app).map_err(|_| "history data unavailable".to_owned())?;
    tauri::async_runtime::spawn_blocking(move || {
        let item = service.get_item(item_id, expected_revision)?;
        let Some(reference) = item.snapshot.asset_ref else {
            return Ok(None);
        };
        let folder = match (item.snapshot.source_kind, item.snapshot.content_type) {
            (SourceKind::Voice, _) => root.join("recordings"),
            (_, ContentType::Image) => root.join("clipboard_images"),
            _ => return Ok(None),
        };
        let folder = folder
            .canonicalize()
            .map_err(|_| "history attachment directory unavailable".to_owned())?;
        let path = folder
            .join(reference)
            .canonicalize()
            .map_err(|_| "history attachment unavailable".to_owned())?;
        if !path.starts_with(&folder) || !path.is_file() {
            return Err("history attachment is outside managed storage".into());
        }
        Ok(Some(path.to_string_lossy().into_owned()))
    })
    .await
    .map_err(|_| "unified history worker failed".to_owned())?
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct UnifiedTerm {
    pub term: String,
    pub contributions: u64,
    pub explicitly_confirmed: bool,
}

#[tauri::command]
#[specta::specta]
pub async fn get_unified_terms(
    manager: State<'_, Arc<IntegrationManager>>,
    limit: u32,
    offset: u64,
) -> Result<Vec<UnifiedTerm>, String> {
    let service = manager.service.clone();
    tauri::async_runtime::spawn_blocking(move || {
        service.list_terms(limit, offset).map(|rows| {
            rows.into_iter()
                .map(|row| UnifiedTerm {
                    term: row.term,
                    contributions: row.contributions,
                    explicitly_confirmed: row.explicitly_confirmed,
                })
                .collect()
        })
    })
    .await
    .map_err(|_| "unified history worker failed".to_owned())?
}

#[derive(Clone, Debug, Deserialize, Type)]
pub struct UnifiedHistoryQuery {
    pub search: Option<String>,
    pub source_kind: Option<String>,
    pub content_type: Option<String>,
    pub starred_only: bool,
    pub limit: u32,
    pub offset: u64,
}

impl TryFrom<UnifiedHistoryQuery> for HistoryQuery {
    type Error = String;
    fn try_from(query: UnifiedHistoryQuery) -> Result<Self, String> {
        let source_kind = match query.source_kind.as_deref() {
            None | Some("all") => None,
            Some("voice") => Some(SourceKind::Voice),
            Some("clipboard") => Some(SourceKind::Clipboard),
            Some("saved_snippet") => Some(SourceKind::SavedSnippet),
            _ => return Err("invalid history source filter".into()),
        };
        let content_type = match query.content_type.as_deref() {
            None | Some("all") => None,
            Some("text") => Some(ContentType::Text),
            Some("image") => Some(ContentType::Image),
            Some("files") => Some(ContentType::Files),
            Some("html") => Some(ContentType::Html),
            Some("rtf") => Some(ContentType::Rtf),
            _ => return Err("invalid history type filter".into()),
        };
        Ok(Self {
            search: query.search,
            source_kind,
            content_type,
            starred_only: query.starred_only,
            limit: query.limit,
            offset: query.offset,
        })
    }
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct UnifiedHistoryItem {
    pub item_id: String,
    pub store_id: String,
    pub record_id: String,
    pub revision: u64,
    pub source_kind: String,
    pub content_type: String,
    pub text: Option<String>,
    pub title: Option<String>,
    pub starred: bool,
    pub pinned: bool,
    pub created_at_ms: i64,
    pub asset_ref: Option<String>,
    pub source_app: Option<String>,
}

impl From<IndexedItem> for UnifiedHistoryItem {
    fn from(item: IndexedItem) -> Self {
        let snapshot = item.snapshot;
        Self {
            item_id: item.item_id,
            store_id: item.store_id,
            record_id: item.record_id,
            revision: item.revision,
            source_kind: match snapshot.source_kind {
                SourceKind::Voice => "voice",
                SourceKind::Clipboard => "clipboard",
                SourceKind::SavedSnippet => "saved_snippet",
            }
            .into(),
            content_type: match snapshot.content_type {
                ContentType::Text => "text",
                ContentType::Image => "image",
                ContentType::Files => "files",
                ContentType::Html => "html",
                ContentType::Rtf => "rtf",
            }
            .into(),
            text: snapshot.text,
            title: snapshot.title,
            starred: snapshot.starred,
            pinned: snapshot.pinned,
            created_at_ms: snapshot.created_at_ms,
            asset_ref: snapshot.asset_ref,
            source_app: snapshot.source_app,
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn get_unified_history(
    manager: State<'_, Arc<IntegrationManager>>,
    query: UnifiedHistoryQuery,
) -> Result<Vec<UnifiedHistoryItem>, String> {
    let query = HistoryQuery::try_from(query)?;
    let service = manager.service.clone();
    tauri::async_runtime::spawn_blocking(move || {
        service
            .query(query)
            .map(|items| items.into_iter().map(Into::into).collect())
    })
    .await
    .map_err(|_| "unified history worker failed".to_owned())?
}

#[derive(Clone, Debug, Serialize, Type)]
pub struct UnifiedHistoryRevision {
    pub revision: u64,
    pub text: Option<String>,
    pub asset_ref: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub async fn get_unified_history_revisions(
    manager: State<'_, Arc<IntegrationManager>>,
    item_id: String,
) -> Result<Vec<UnifiedHistoryRevision>, String> {
    let service = manager.service.clone();
    tauri::async_runtime::spawn_blocking(move || {
        service.revisions(item_id).map(|revisions| {
            revisions
                .into_iter()
                .map(|item| UnifiedHistoryRevision {
                    revision: item.revision,
                    text: item.snapshot.text,
                    asset_ref: item.snapshot.asset_ref,
                })
                .collect()
        })
    })
    .await
    .map_err(|_| "unified history worker failed".to_owned())?
}

#[tauri::command]
#[specta::specta]
pub async fn refresh_unified_history(
    manager: State<'_, Arc<IntegrationManager>>,
) -> Result<u64, String> {
    let service = manager.service.clone();
    tauri::async_runtime::spawn_blocking(move || service.synchronize())
        .await
        .map_err(|_| "unified history worker failed".to_owned())?
}

#[cfg(test)]
mod term_confirmation_tests {
    use super::validate_term_confirmation;

    #[test]
    fn confirmation_requires_focused_control_center_and_nonsecure_explicit_consent() {
        assert!(validate_term_confirmation("main", true, true, false).is_ok());
        for (label, focused, confirmed, secure) in [
            ("clipboard", true, true, false),
            ("main", false, true, false),
            ("main", true, false, false),
            ("main", true, true, true),
        ] {
            assert!(validate_term_confirmation(label, focused, confirmed, secure).is_err());
        }
    }
}
