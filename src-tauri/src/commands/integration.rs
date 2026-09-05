use crate::managers::clipboard::ClipboardManager;
use crate::managers::integration::IntegrationManager;
use inputia_handy_runtime::store::{ContentType, HistoryQuery, IndexedItem, SourceKind};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::sync::Arc;
use tauri::{AppHandle, State};

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
    manager: State<'_, Arc<IntegrationManager>>,
    clipboard: State<'_, Arc<ClipboardManager>>,
    item_id: String,
    expected_revision: u64,
) -> Result<(), String> {
    let service = manager.service.clone();
    let clipboard = Arc::clone(&clipboard);
    tauri::async_runtime::spawn_blocking(move || {
        let item = service.get_item(item_id, expected_revision)?;
        if item.snapshot.content_type == ContentType::Files {
            return clipboard
                .copy_files_strict(
                    item.snapshot
                        .text
                        .as_deref()
                        .ok_or_else(|| "file payload unavailable".to_owned())?,
                )
                .map_err(|_| "unable to restore native file clipboard".to_owned());
        }
        let kind = match item.snapshot.content_type {
            ContentType::Text => "text",
            ContentType::Image => "image",
            ContentType::Files => "file",
            ContentType::Html | ContentType::Rtf => {
                return Err("rich content requires an explicit plain-text copy action".into())
            }
        };
        clipboard
            .copy_content_to_clipboard(kind, item.snapshot.text, item.snapshot.asset_ref)
            .map_err(|_| "unable to copy history item".to_owned())
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
