//! 个人学习服务只接收已认证输入法事件，原字段证明与全文收录开关相互独立。
use inputia_handy_runtime::{
    decision::{DecisionAnswer, DecisionLimits, DecisionQuestion, DecisionRequest},
    personalization as model,
    personalization_wire::{PersonalizationCommand, PersonalizationReply, PersonalizationRequest},
    voice_protocol::TargetBridgePurpose,
};
use serde_json::json;
use std::collections::BTreeMap;
use tauri::Manager;

pub fn respond(app: &tauri::AppHandle, request: &PersonalizationRequest) -> PersonalizationReply {
    let mut reply = PersonalizationReply {
        status: "personalization".into(),
        request_id: request.request_id.clone(),
        server_instance: request.server_instance.clone(),
        enabled: false,
        epoch: 0,
        result: None,
        code: None,
        privacy_barrier: None,
    };
    let result = (|| -> Result<(), String> {
        let integration = app
            .try_state::<std::sync::Arc<crate::managers::integration::IntegrationManager>>()
            .ok_or("privacy_service_unavailable")?;
        integration.service.privacy_readable()?;
        let root = crate::portable::app_data_dir(app).map_err(|e| e.to_string())?;
        let policy = model::policy(&root)?;
        reply.enabled = policy.enabled;
        reply.epoch = policy.epoch;
        let (target, epoch) = match &request.personalization {
            PersonalizationCommand::Policy => return Ok(()),
            PersonalizationCommand::Query {
                target,
                learning_epoch,
                ..
            }
            | PersonalizationCommand::Admit {
                target,
                learning_epoch,
                ..
            }
            | PersonalizationCommand::Feedback {
                target,
                learning_epoch,
                ..
            } => (target, *learning_epoch),
        };
        if !policy.enabled || epoch != policy.epoch {
            return Err("personalization_disabled_or_changed".into());
        }
        crate::ime_target_broker::check(
            app,
            &request.client_instance,
            &request.server_instance,
            target,
            TargetBridgePurpose::Personalization,
        )?;
        if crate::secure_input::is_enabled_now() {
            return Err("personalization_secure_input".into());
        }
        let source = target
            .source_app
            .as_deref()
            .ok_or("personalization_source_missing")?;
        let context_id = format!(
            "{}:{}:{}",
            request.server_instance, request.client_instance, target.target_id
        );
        let value = match &request.personalization {
            PersonalizationCommand::Admit {
                context,
                input_code,
                schema_id,
                text,
                prediction_id,
                ..
            } => {
                let current = model::query(
                    &root,
                    model::Query {
                        input_code: input_code.clone(),
                        schema_id: schema_id.clone(),
                        context: context.clone(),
                        context_id,
                        source_app: source.into(),
                        learning_epoch: epoch,
                        candidates: Vec::new(),
                        limit: 5,
                    },
                )?;
                let collection = if input_code.is_empty() {
                    "predictions"
                } else {
                    "recalled_candidates"
                };
                if current[collection].as_array().is_none_or(|items| {
                    !items
                        .iter()
                        .any(|item| item["id"] == *prediction_id && item["text"] == *text)
                }) {
                    return Err("prediction_no_longer_available".into());
                }
                json!({"admitted":true,"prediction_id":prediction_id,"context_id":target.target_id})
            }
            PersonalizationCommand::Query {
                input_code,
                schema_id,
                context,
                candidates,
                limit,
                ..
            } => {
                let mut q = model::Query {
                    input_code: input_code.clone(),
                    schema_id: schema_id.clone(),
                    context: context.clone(),
                    context_id,
                    source_app: source.into(),
                    learning_epoch: epoch,
                    candidates: serde_json::from_value(json!(candidates))
                        .map_err(|_| "invalid rank candidates")?,
                    limit: *limit,
                };
                // 可选本地模型只提供先验，不能在个人偏好/拒绝之后覆盖其结果。
                let mut decision_reranked = false;
                if let Some(ordered_ids) = local_decision_rerank(input_code, context, &q.candidates)
                {
                    if let Ok(mut reordered) = model::apply_rerank(&q.candidates, &ordered_ids) {
                        for (rank, candidate) in reordered.iter_mut().enumerate() {
                            candidate.base_rank = rank;
                        }
                        q.candidates = reordered;
                        decision_reranked = true;
                    }
                }
                let local_candidates = q.candidates.clone();
                let mut v = model::query(&root, q)?;
                if decision_reranked {
                    v["decision_reranked"] = json!(true);
                }
                let current_ids: Vec<String> = serde_json::from_value(v["ordered_ids"].clone())
                    .map_err(|_| "invalid_candidate_order")?;
                let explicit = crate::settings::get_settings(app).custom_words;
                if let Ok(ids) = model::prioritize_explicit_candidates(
                    &local_candidates,
                    &current_ids,
                    &explicit,
                ) {
                    v["ordered_ids"] = json!(ids);
                }
                // 对外继续使用当前字段opaque ID，不暴露内部证据namespace。
                v["context_id"] = json!(target.target_id);
                v
            }
            PersonalizationCommand::Feedback {
                event_id,
                input_code,
                schema_id,
                text,
                previous,
                explicit_selection,
                original_rank,
                operation,
                ..
            } => model::feedback(
                &root,
                model::Feedback {
                    event_id: format!("{}:{event_id}", request.client_instance),
                    context_id,
                    input_code: input_code.clone(),
                    schema_id: schema_id.clone(),
                    text: text.clone(),
                    previous: previous.clone(),
                    explicit_selection: *explicit_selection,
                    original_rank: *original_rank,
                    source_app: source.into(),
                    learning_epoch: epoch,
                    operation: operation.clone(),
                },
            )?,
            PersonalizationCommand::Policy => unreachable!(),
        };
        // 查询后台I/O期间字段或隐私状态发生变化，结果不得再交给Host。
        crate::ime_target_broker::check(
            app,
            &request.client_instance,
            &request.server_instance,
            target,
            TargetBridgePurpose::Personalization,
        )?;
        let current = model::policy(&root)?;
        if !current.enabled || current.epoch != epoch {
            reply.enabled = current.enabled;
            reply.epoch = current.epoch;
            return Err("personalization_policy_changed".into());
        }
        integration.service.privacy_readable()?;
        reply.result = Some(value);
        Ok(())
    })();
    if let Err(error) = result {
        reply.code = Some(error)
    }
    reply
}

fn local_decision_rerank(
    input_code: &str,
    context: &str,
    candidates: &[model::Candidate],
) -> Option<Vec<String>> {
    let experimental = std::env::var("INPUTIA_EXPERIMENTAL_CANDIDATE_MODEL").ok();
    if !model::experimental_candidate_model_enabled(experimental.as_deref()) {
        return None;
    }
    if candidates.is_empty() || candidates.len() > 64 {
        return None;
    }
    let worker = crate::decision_worker::DecisionWorker::configured()?;
    let criteria: BTreeMap<String, String> = candidates
        .iter()
        .map(|candidate| (candidate.id.clone(), candidate.text.clone()))
        .collect();
    let request = DecisionRequest {
        protocol_version: inputia_handy_runtime::decision::PROTOCOL_VERSION,
        request_id: format!("candidate-rerank-{}", candidates.len()),
        model_id: "laya-multilingual-mlx".into(),
        state: json!({ "context": context, "input_code": input_code }),
        questions: [("candidate".into(), DecisionQuestion::Choice {
            instructions: "Choose the most likely candidate for this context and input code. Use only the supplied candidates.".into(),
            criteria,
        })].into_iter().collect(),
        limits: DecisionLimits { deadline_ms: 150, max_input_chars: 4000 },
    };
    let response = worker.request(request).ok()?;
    let DecisionAnswer::Choice {
        probabilities,
        confidence,
        abstained,
        ..
    } = response.answers.get("candidate")?
    else {
        return None;
    };
    if *abstained || *confidence < 0.5 || probabilities.len() != candidates.len() {
        return None;
    }
    let mut ordered: Vec<_> = probabilities.iter().collect();
    ordered.sort_by(|(_, left), (_, right)| right.total_cmp(left));
    Some(ordered.into_iter().map(|(id, _)| id.clone()).collect())
}
