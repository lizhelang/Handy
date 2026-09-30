//! 已认证输入法连接的旧学习域适配；路径、来源和读者登记均由后台确定。
use inputia_handy_runtime::{
    legacy_memory::{MemoryImportRequest, MemoryImportSelection},
    legacy_memory_wire::{
        ImportSelection, MemoryCommand, MemoryReply, MemoryRequest, MemoryResult,
        MemorySnapshotReply,
    },
    service::HistoryService,
    voice_protocol::TargetBridgePurpose,
};
use inputia_settings::store::{Snapshot, Store};

pub(crate) fn preferences() -> Result<Snapshot, String> {
    let profile = crate::candidate_profile::current().ok_or("memory_profile_unavailable")?;
    let user = inputia_settings::maintenance::current_user_context()
        .map_err(|_| "memory_settings_unavailable")?;
    Store::open(
        &profile.inputia_root.join("settings.json"),
        &user.home,
        user.uid,
    )
    .and_then(|store| store.read())
    .map_err(|_| "memory_settings_unavailable".into())
}
fn same_settings(first: &Snapshot, second: &Snapshot) -> bool {
    first.store_id == second.store_id
        && first.revision == second.revision
        && first.values_digest == second.values_digest
}
fn allowed(snapshot: &Snapshot) -> Result<(), String> {
    let settings = snapshot
        .settings()
        .map_err(|_| "memory_settings_unavailable")?;
    if settings.memory_enabled && settings.privacy_learning_enabled {
        Ok(())
    } else {
        Err("memory_disabled".into())
    }
}
fn safe_code(code: &str) -> &'static str {
    match code {
        "memory_disabled" => "memory_disabled",
        "memory_handoff_required" => "memory_handoff_required",
        "memory_not_configured" => "memory_not_configured",
        "legacy_coverage_unresolved" => "legacy_coverage_unresolved",
        "memory_source_changed" | "memory_source_busy" => "memory_source_busy",
        "memory_epoch_revoked" | "memory_privacy_pending" | "privacy_operation_pending" => {
            "memory_epoch_revoked"
        }
        "memory_commit_confirmation_required" => "memory_commit_confirmation_required",
        "memory_settings_changed" => "memory_settings_changed",
        "memory_target_unverified" => "memory_target_unverified",
        "memory_commit_budget" => "memory_commit_budget",
        "memory_commit_expired" => "memory_commit_expired",
        "memory_commit_unknown" => "memory_commit_unknown",
        "memory_commit_unknown_plan" => "memory_commit_unknown_plan",
        "memory_commit_replayed" => "memory_commit_replayed",
        "memory_commit_precondition_changed" => "memory_commit_precondition_changed",
        "memory_commit_postcondition_changed" => "memory_commit_postcondition_changed",
        "memory_commit_word_boundary" => "memory_commit_word_boundary",
        "memory_commit_no_observed_change" => "memory_commit_no_observed_change",
        "memory_commit_prefix_changed" => "memory_commit_prefix_changed",
        "memory_target_changed" => "memory_target_changed",
        "memory_target_timeout" => "memory_target_timeout",
        "memory_range_unsupported" => "memory_range_unsupported",
        "memory_range_unavailable" => "memory_range_unavailable",

        "memory_span_unknown" => "memory_span_unknown",
        "memory_span_sealed" => "memory_span_sealed",
        "memory_span_identity_changed" => "memory_span_identity_changed",
        "memory_span_expired" => "memory_span_expired",
        "memory_span_budget_exceeded" => "memory_span_budget_exceeded",
        "memory_span_sequence_invalid" => "memory_span_sequence_invalid",
        "memory_span_replay_conflict" => "memory_span_replay_conflict",
        "memory_span_edit_invalid" => "memory_span_edit_invalid",
        "memory_span_boundary_required" => "memory_span_boundary_required",
        "memory_span_readback_unsupported" => "memory_span_readback_unsupported",
        "memory_span_field_changed" => "memory_span_field_changed",
        "memory_span_readback_changed" => "memory_span_readback_changed",
        "memory_span_no_observed_change" => "memory_span_no_observed_change",
        "memory_span_revocation_pending" => "memory_span_revocation_pending",
        "memory_span_request_invalid" => "memory_span_request_invalid",
        "memory_span_recovery_unavailable" => "memory_span_recovery_unavailable",
        _ => "memory_service_unavailable",
    }
}

/// 身份/能力/当前连接屏障在VoiceConnection验证，当前字段和设置在正文出口前后再次确认。
pub fn respond(
    app: &tauri::AppHandle,
    history: &HistoryService,
    request: &MemoryRequest,
    profile_id: &str,
) -> MemoryReply {
    let mut reply = MemoryReply {
        status: "memory_domain".into(),
        request_id: request.request_id.clone(),
        server_instance: request.server_instance.clone(),
        profile_id: profile_id.into(),
        policy_epoch: request.policy_epoch,
        result: None,
        code: None,
        privacy_barrier: None,
    };
    let result = (|| -> Result<MemoryResult, String> {
        if let MemoryCommand::Outcome { operation_id } = &request.memory_domain {
            return Ok(MemoryResult::Outcome {
                operation: history.memory_operation(operation_id.clone())?,
            });
        }
        if let MemoryCommand::RetireWordSpan { span_id } = &request.memory_domain {
            if let Some(inputia_handy_runtime::legacy_memory::MemoryOperationStatus::Learn(
                receipt,
            )) = history.memory_operation(format!("span-revoke-{span_id}"))?
            {
                if receipt.state
                    == inputia_handy_runtime::legacy_memory::MemoryMutationState::Revoked
                {
                    return Ok(MemoryResult::WordSpanRetired {});
                }
            }
            crate::ime_target_broker::retire_memory_word_span(
                app,
                &request.client_instance,
                &request.server_instance,
                span_id.clone(),
            )?;
            crate::ime_target_broker::flush_word_span_revocations(app, history)?;
            return Ok(MemoryResult::WordSpanRetired {});
        }
        let settings = preferences()?;
        allowed(&settings)?;
        if history.policy_epoch()? != request.policy_epoch {
            return Err("memory_epoch_revoked".into());
        }
        match &request.memory_domain {
            MemoryCommand::Policy {} => Ok(MemoryResult::Policy {
                domain: history.memory_domain_status()?,
            }),
            MemoryCommand::Query {
                query_id,
                query_generation,
                target,
                query,
                ..
            } => {
                let check = || -> Result<(), String> {
                    if crate::secure_input::is_enabled_now() {
                        return Err("memory_target_unverified".into());
                    }
                    crate::ime_target_broker::check(
                        app,
                        &request.client_instance,
                        &request.server_instance,
                        target,
                        TargetBridgePurpose::Personalization,
                    )
                    .map_err(|_| "memory_target_unverified".into())
                };
                check()?;
                let lease = history.memory_query(
                    query.clone(),
                    request.policy_epoch,
                    request.client_instance.clone(),
                )?;
                check()?;
                let current = preferences()?;
                if current.store_id != settings.store_id
                    || current.revision != settings.revision
                    || current.values_digest != settings.values_digest
                {
                    return Err("memory_settings_changed".into());
                }
                if history.policy_epoch()? != request.policy_epoch
                    || lease.epoch != request.policy_epoch
                {
                    return Err("memory_epoch_revoked".into());
                }
                let snapshot = MemorySnapshotReply {
                    format_version: 1,
                    request_id: request.request_id.clone(),
                    server_instance: request.server_instance.clone(),
                    profile_id: profile_id.into(),
                    policy_epoch: lease.epoch,
                    domain_uuid: lease.domain_uuid,
                    generation: lease.generation,
                    query_id: query_id.clone(),
                    query_generation: *query_generation,
                    query_digest: request.query_digest().map_err(|_| "memory_query_invalid")?,
                    query: query.clone(),
                    terms: lease.terms,
                    lease_id: request.client_instance.clone(),
                    max_age_ms: lease.max_age_ms,
                };
                snapshot
                    .validate_for(request, profile_id)
                    .map_err(|_| "memory_snapshot_invalid")?;
                Ok(MemoryResult::Snapshot { snapshot })
            }
            MemoryCommand::Import {
                operation_id,
                selection,
                limit,
            } => {
                let selection = match selection {
                    ImportSelection::History => MemoryImportSelection::History,
                    ImportSelection::Clipboard => MemoryImportSelection::Clipboard,
                    ImportSelection::Both => MemoryImportSelection::Both,
                };
                Ok(MemoryResult::Import {
                    operation: history.memory_import(MemoryImportRequest {
                        operation_id: operation_id.clone(),
                        expected_epoch: request.policy_epoch,
                        selection,
                        limit: *limit,
                    })?,
                })
            }
            MemoryCommand::PrepareWordSpan { target } => {
                history.privacy_readable()?;
                if history.memory_domain_status()?.state
                    != inputia_handy_runtime::legacy_memory::MemoryDomainState::Ready
                {
                    return Err("memory_handoff_required".into());
                }
                crate::ime_target_broker::flush_word_span_revocations(app, history)?;
                let started = std::time::Instant::now();
                history
                    .issue_privacy_reader(request.client_instance.clone(), request.policy_epoch)?;
                let permit = crate::ime_target_broker::prepare_memory_word_span(
                    app,
                    &request.client_instance,
                    &request.server_instance,
                    target,
                    request.policy_epoch,
                    started,
                )?;
                if !same_settings(&settings, &preferences()?)
                    || history.policy_epoch()? != request.policy_epoch
                {
                    crate::ime_target_broker::retire_memory_word_span(
                        app,
                        &request.client_instance,
                        &request.server_instance,
                        permit.span_id,
                    )?;
                    crate::ime_target_broker::flush_word_span_revocations(app, history)?;
                    return Err("memory_settings_changed".into());
                }
                Ok(MemoryResult::PreparedWordSpan { permit })
            }
            MemoryCommand::RecordWordSpan {
                target,
                span_id,
                sequence,
                edit,
            } => {
                let progress = crate::ime_target_broker::record_memory_word_span(
                    app,
                    &request.client_instance,
                    &request.server_instance,
                    target,
                    request.policy_epoch,
                    span_id.clone(),
                    *sequence,
                    edit.clone(),
                )?;
                Ok(MemoryResult::WordSpanProgress { progress })
            }
            MemoryCommand::CheckpointWordSpan {
                target,
                span_id,
                operation_id,
                through_sequence,
                finish,
            } => {
                // checkpoint 重放也必须复核原许可的完整身份；单独查询历史结果使用 Outcome。
                let started = std::time::Instant::now();
                history
                    .issue_privacy_reader(request.client_instance.clone(), request.policy_epoch)?;
                let proof = crate::ime_target_broker::checkpoint_memory_word_span(
                    app,
                    &request.client_instance,
                    &request.server_instance,
                    target,
                    request.policy_epoch,
                    span_id.clone(),
                    inputia_handy_runtime::memory_word_span::WordSpanCheckpoint {
                        operation_id: operation_id.clone(),
                        through_sequence: *through_sequence,
                        finish: *finish,
                    },
                    started,
                )?;
                if !same_settings(&settings, &preferences()?)
                    || history.policy_epoch()? != request.policy_epoch
                {
                    crate::ime_target_broker::clear_memory_commits(
                        app,
                        &request.client_instance,
                        &request.server_instance,
                    )?;
                    return Err("memory_settings_changed".into());
                }
                let receipt = history.memory_apply_word_span(proof.clone())?;
                if proof.sealed() && receipt.operation_id == proof.operation_id()
                    && receipt.applied_at_epoch == proof.identity().policy_epoch
                    && matches!(receipt.state, inputia_handy_runtime::legacy_memory::MemoryMutationState::Applied
                        | inputia_handy_runtime::legacy_memory::MemoryMutationState::AlreadyContributed) {
                    // 回执已耐久；主线程ACK失败只保留恢复token，不把真实提交伪装成未发生。
                    let _ = crate::ime_target_broker::acknowledge_memory_word_span(app, proof);
                }
                Ok(MemoryResult::Learn { receipt })
            }
            MemoryCommand::RetireWordSpan { .. } => unreachable!(),
            MemoryCommand::PrepareCommit {
                target,
                request: plans,
            } => {
                history.privacy_readable()?;
                if history.memory_domain_status()?.state
                    != inputia_handy_runtime::legacy_memory::MemoryDomainState::Ready
                {
                    return Err("memory_handoff_required".into());
                }
                // 先登记旧epoch读者，再接触有界正文；期限从登记前计，不能被AX排队延长。
                let prepared_started = std::time::Instant::now();
                history
                    .issue_privacy_reader(request.client_instance.clone(), request.policy_epoch)?;
                let permit = crate::ime_target_broker::prepare_memory_commit(
                    app,
                    &request.client_instance,
                    &request.server_instance,
                    target,
                    request.policy_epoch,
                    plans.clone(),
                    prepared_started,
                )?;
                let current = preferences()?;
                let settings_changed = current.store_id != settings.store_id
                    || current.revision != settings.revision
                    || current.values_digest != settings.values_digest;
                if settings_changed || history.policy_epoch()? != request.policy_epoch {
                    crate::ime_target_broker::clear_memory_commits(
                        app,
                        &request.client_instance,
                        &request.server_instance,
                    )?;
                    return Err(if settings_changed {
                        "memory_settings_changed".into()
                    } else {
                        "memory_epoch_revoked".into()
                    });
                }
                Ok(MemoryResult::PreparedCommit { permit })
            }
            MemoryCommand::ConfirmCommit {
                target,
                operation_id,
                commit_id,
                plan_id,
            } => {
                // ACK丢失先查耐久结果。成功后的重试不再读取当前字段补造旧证据。
                if let Some(inputia_handy_runtime::legacy_memory::MemoryOperationStatus::Learn(
                    receipt,
                )) = history.memory_operation(operation_id.clone())?
                {
                    return Ok(MemoryResult::Learn { receipt });
                }
                let confirmed = crate::ime_target_broker::confirm_memory_commit(
                    app,
                    &request.client_instance,
                    &request.server_instance,
                    target,
                    request.policy_epoch,
                    commit_id.clone(),
                    plan_id.clone(),
                    operation_id.clone(),
                )?;
                let current = preferences()?;
                if current.store_id != settings.store_id
                    || current.revision != settings.revision
                    || current.values_digest != settings.values_digest
                {
                    return Err("memory_settings_changed".into());
                }
                let (intent, evidence) = confirmed
                    .into_learning(operation_id.clone())
                    .map_err(str::to_owned)?;
                Ok(MemoryResult::Learn {
                    receipt: history.memory_learn(intent, evidence)?,
                })
            }
            // 两阶段AX提交证明接线期间不接受客户端自报的已提交正文作为学习证据。
            MemoryCommand::LearnTyped { .. } => Err("memory_commit_confirmation_required".into()),
            MemoryCommand::Outcome { .. } => unreachable!(),
        }
    })();
    match result {
        Ok(value) => reply.result = Some(value),
        Err(code) => reply.code = Some(safe_code(&code).into()),
    }
    reply
}
