//! 知识库的控制中心入口；磁盘和索引操作在阻塞线程执行。

use inputia_handy_runtime::decision::{DecisionLimits, DecisionQuestion, DecisionRequest};
use inputia_handy_runtime::embedding::{cosine_similarity, EmbeddingVector};
use inputia_handy_runtime::knowledge::KnowledgeStore;
use serde_json::Value;
use std::collections::BTreeMap;
use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;

static KNOWLEDGE_WORK: std::sync::Mutex<()> = std::sync::Mutex::new(());
static STOP_INDEXER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 目录对账与用户操作使用同一串行通道，不占用输入线程。
pub fn start_indexer(app: &AppHandle) {
    let Ok(root) = crate::portable::app_data_dir(app) else {
        return;
    };
    let lexicon = app
        .path()
        .resolve(
            "resources/personalization/base-lexicon.tsv",
            tauri::path::BaseDirectory::Resource,
        )
        .ok();
    STOP_INDEXER.store(false, std::sync::atomic::Ordering::Relaxed);
    std::thread::spawn(move || {
        if let Some(path) = lexicon.filter(|path| path.is_file()) {
            if let Err(error) =
                inputia_handy_runtime::personalization::install_base_lexicon(&root, &path)
            {
                log::warn!("Unable to prepare public prediction lexicon: {error}");
            }
        }
        while !STOP_INDEXER.load(std::sync::atomic::Ordering::Relaxed) {
            if let Ok(_guard) = KNOWLEDGE_WORK.lock() {
                if let Err(error) =
                    inputia_handy_runtime::personalization::reconcile_imports(&root, 64)
                {
                    log::warn!("Unable to reconcile learned source revisions: {error}");
                }
                if let Err(error) = KnowledgeStore::open(&root)
                    .and_then(|store| store.dispatch("sync", serde_json::json!({}), false))
                {
                    log::warn!("Knowledge index reconciliation failed: {error}");
                }
            }
            for _ in 0..30 {
                if STOP_INDEXER.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        }
    });
}

pub fn stop_indexer() {
    STOP_INDEXER.store(true, std::sync::atomic::Ordering::Relaxed);
}

#[tauri::command]
pub async fn knowledge_request(
    app: AppHandle,
    window: tauri::WebviewWindow,
    action: String,
    payload: Value,
) -> Result<Value, String> {
    if window.label() != "main" {
        return Err("knowledge settings are available only in the control center".into());
    }
    let root = crate::portable::app_data_dir(&app).map_err(|e| e.to_string())?;
    let worker_app = app.clone();
    let open_managed = action == "open_managed";
    let result = tauri::async_runtime::spawn_blocking(move || {
        let _guard = KNOWLEDGE_WORK
            .lock()
            .map_err(|_| "knowledge worker unavailable")?;
        if let Some(manager) = worker_app
            .try_state::<std::sync::Arc<crate::managers::integration::IntegrationManager>>()
        {
            // 新的源变更先进入统一历史；CLI 另外校验源记录版本，防止旧索引回填。
            if matches!(action.as_str(), "status" | "search" | "read" | "sync") {
                manager.service.synchronize()?;
            }
        }
        if action.starts_with("privacy_") {
            let manager=worker_app.try_state::<std::sync::Arc<crate::managers::integration::IntegrationManager>>().ok_or("privacy_service_unavailable")?;
            let service=&manager.service;
            return match action.as_str() {
                "privacy_status"=>Ok(serde_json::json!({"epoch":service.policy_epoch()?,"operations":service.privacy_operations()?})),
                "privacy_begin"=>serde_json::to_value(service.begin_privacy(serde_json::from_value(payload).map_err(|_|"privacy_request_invalid")?)?).map_err(|_|"privacy_reply_invalid".into()),
                "privacy_operation"=>serde_json::to_value(service.privacy_operation(payload.get("operation_id").and_then(Value::as_str).ok_or("privacy_operation_missing")?.into())?).map_err(|_|"privacy_reply_invalid".into()),
                _=>Err("privacy_action_unknown".into()),
            };
        }
        if action.starts_with("personalization_") {
            return inputia_handy_runtime::personalization::manage(&root, &action, &payload);
        }
        let store = KnowledgeStore::open(&root)?;
        if matches!(action.as_str(), "status" | "search") {
            store.dispatch("sync", serde_json::json!({}), false)?;
        }
        if action == "export_connection" {
            return inputia_handy_runtime::knowledge_connection::export_connection(
                &root,
                &std::env::current_exe().map_err(|e| e.to_string())?,
            );
        }
        let mut payload = payload;
        if action == "search" {
            if let Some(query) = payload.get("query").and_then(Value::as_str) {
                if let Some(intent) = local_query_intent(query) {
                    if let Some(object) = payload.as_object_mut() {
                        object.insert("_decision_intent".into(), Value::String(intent));
                    }
                }
            }
        }
        let semantic_payload = payload.clone();
        let result = store.dispatch(
            if open_managed { "status" } else { &action },
            payload,
            false,
        )?;
        if action == "search" {
            Ok(local_semantic_rerank(result, &semantic_payload))
        } else {
            Ok(result)
        }
    })
    .await
    .map_err(|e| e.to_string())??;
    if open_managed {
        let path = result["managed_path"]
            .as_str()
            .ok_or("managed path unavailable")?;
        app.opener()
            .open_path(path, None::<String>)
            .map_err(|e| e.to_string())?;
    }
    Ok(result)
}

fn local_semantic_rerank(mut result: Value, payload: &Value) -> Value {
    let Some(query) = payload.get("query").and_then(Value::as_str) else {
        return result;
    };
    let Some(items) = result.get("items").and_then(Value::as_array) else {
        return result;
    };
    if items.is_empty() || items.len() > 24 {
        return result;
    }
    let mut texts = Vec::with_capacity(items.len() + 1);
    texts.push(query.to_owned());
    texts.extend(items.iter().filter_map(|item| {
        item.get("text")
            .and_then(Value::as_str)
            .map(|text| text.chars().take(4000).collect::<String>())
    }));
    if texts.len() != items.len() + 1 {
        return result;
    }
    let Some(reply) = crate::embedding_worker::embed(&texts) else {
        return result;
    };
    let query_vector = EmbeddingVector {
        model_id: reply.model_id.clone(),
        model_revision: reply.model_revision.clone(),
        dimensions: reply.dimensions,
        values: reply.vectors[0].clone(),
    };
    let mut scored = Vec::with_capacity(items.len());
    for (index, item) in items.iter().enumerate() {
        let vector = EmbeddingVector {
            model_id: reply.model_id.clone(),
            model_revision: reply.model_revision.clone(),
            dimensions: reply.dimensions,
            values: reply.vectors[index + 1].clone(),
        };
        let Ok(score) = cosine_similarity(&query_vector, &vector) else {
            return result;
        };
        scored.push((score, item.clone()));
    }
    scored.sort_by(|(left, _), (right, _)| right.total_cmp(left));
    result["items"] = Value::Array(scored.into_iter().map(|(_, item)| item).collect());
    result["semantic_reranked"] = Value::Bool(true);
    result["embedding_model"] = Value::String(reply.model_id);
    result
}

fn local_query_intent(query: &str) -> Option<String> {
    let worker = crate::decision_worker::DecisionWorker::configured()?;
    let request = DecisionRequest {
        protocol_version: inputia_handy_runtime::decision::PROTOCOL_VERSION,
        request_id: format!(
            "knowledge-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or_default()
        ),
        model_id: "laya-multilingual-mlx".into(),
        state: serde_json::json!({ "query": query }),
        questions: [(
            "intent".into(),
            DecisionQuestion::Choice {
                instructions: "Classify the knowledge query intent.".into(),
                criteria: BTreeMap::from([
                    (
                        "exact_lookup".into(),
                        "Find an exact fact or original text".into(),
                    ),
                    (
                        "procedure".into(),
                        "Find steps or how-to instructions".into(),
                    ),
                    ("definition".into(), "Find a definition".into()),
                    (
                        "comparison".into(),
                        "Compare multiple objects or options".into(),
                    ),
                    (
                        "history_lookup".into(),
                        "Find a past event or history record".into(),
                    ),
                    ("unknown".into(), "Cannot determine".into()),
                ]),
            },
        )]
        .into_iter()
        .collect(),
        limits: DecisionLimits {
            deadline_ms: 180,
            max_input_chars: 4_000,
        },
    };
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let _ = sender.send(worker.request(request));
    });
    let response = receiver
        .recv_timeout(std::time::Duration::from_millis(250))
        .ok()
        .and_then(Result::ok)?;
    match response.answers.get("intent") {
        Some(inputia_handy_runtime::decision::DecisionAnswer::Choice {
            selected,
            abstained,
            ..
        }) if !abstained && selected != "unknown" => Some(selected.clone()),
        _ => None,
    }
}
