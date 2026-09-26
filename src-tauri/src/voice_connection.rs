//! 产品后台连接的认证入口；消费socket所有权，任何失败直接关闭，不保留半认证连接。
use crate::native_pair_auth::{PairManifest, PeerRole};
use crate::voice_dispatch::{
    AuthenticatedVoiceConnection, DispatchError, VoiceCoordinatorPort, VoiceDispatcher,
};
use inputia_handy_runtime::{
    output_ledger::{OutputIntent, OutputOutcome},
    protocol::{Handshake, HandshakePolicy, ProtocolError},
    service::HistoryService,
    transport,
};
use std::collections::HashMap;
use std::os::{fd::AsFd, unix::net::UnixStream};
use tauri::Manager;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionError {
    Authentication,
    PolicyUnavailable,
    Handshake,
    MissingContext,
    PolicySync,
    ControlFrame,
}

/// 持有原始已认证socket，禁止把该授权挪到其他fd；不提供裸context转移API。
pub struct VoiceConnection {
    stream: UnixStream,
    context: AuthenticatedVoiceConnection,
    client: Handshake,
    dispatched_outputs: HashMap<String, OutputIntent>,
}

impl VoiceConnection {
    /// 只在后台执行。Accepted仅表示身份/协议就绪，context尚未确认策略屏障。
    pub fn accept(
        mut stream: UnixStream,
        manifest: &PairManifest,
        mut server: Handshake,
        history: &HistoryService,
    ) -> Result<Self, ConnectionError> {
        // 不先读取握手/正文，再用客户端自报字段拼签名约束。
        let peer = manifest
            .authenticate(stream.as_fd(), PeerRole::Inputia)
            .map_err(|_| ConnectionError::Authentication)?;
        server.policy_epoch = history
            .policy_epoch()
            .map_err(|_| ConnectionError::PolicyUnavailable)?;
        let policy = HandshakePolicy {
            profile_id: server.profile_id.clone(),
            current_policy_epoch: server.policy_epoch,
        };
        let mut context = None;
        let client = transport::server_handshake_checked(&mut stream, &server, &policy, |client| {
            context = Some(
                AuthenticatedVoiceConnection::from_verified_peer(peer, client, &server, history)
                    .map_err(|_| ProtocolError::PeerIdentity)?,
            );
            Ok(())
        })
        .map_err(|_| ConnectionError::Handshake)?;
        Ok(Self {
            stream,
            context: context.ok_or(ConnectionError::MissingContext)?,
            client,
            dispatched_outputs: HashMap::new(),
        })
    }

    pub fn client_instance(&self) -> &str {
        &self.client.instance_id
    }

    /// 后台同连接完成全量失效屏障；读写错误后关闭socket避免帧边界不确定继续使用。
    pub fn synchronize_policy(&mut self, history: &HistoryService) -> Result<(), ConnectionError> {
        use inputia_handy_runtime::voice_protocol::{
            VoicePolicyAcknowledgement, VoicePolicyBarrier,
        };
        self.context.invalidate_policy();
        let result = (|| {
            let version = history
                .voice_terms_version()
                .map_err(|_| ConnectionError::PolicyUnavailable)?;
            let barrier =
                VoicePolicyBarrier::new(version).map_err(|_| ConnectionError::PolicySync)?;
            transport::write_frame(&mut self.stream, &barrier)
                .map_err(|_| ConnectionError::PolicySync)?;
            let ack: VoicePolicyAcknowledgement =
                transport::read_frame(&mut self.stream).map_err(|_| ConnectionError::PolicySync)?;
            let current = history
                .voice_terms_version()
                .map_err(|_| ConnectionError::PolicyUnavailable)?;
            barrier
                .validate_ack(&ack, &current)
                .map_err(|_| ConnectionError::PolicySync)?;
            self.context
                .acknowledge_policy(current.clone(), &current)
                .map_err(|_| ConnectionError::PolicySync)
        })();
        if result.is_err() {
            let _ = self.stream.shutdown(std::net::Shutdown::Both);
        }
        result
    }

    /// 一个有界控制帧对应一个带request_id回执；失败关闭，不在同一流上猜下一帧。
    pub fn process_one(
        &mut self,
        history: &HistoryService,
        coordinator: &impl VoiceCoordinatorPort,
    ) -> Result<(), ConnectionError> {
        self.process_one_with_app(history, coordinator, None)
    }

    pub fn process_one_with_app(
        &mut self,
        history: &HistoryService,
        coordinator: &impl VoiceCoordinatorPort,
        app: Option<&tauri::AppHandle>,
    ) -> Result<(), ConnectionError> {
        use inputia_handy_runtime::voice_protocol::{
            VoiceOutputCommand, VoiceOutputReply, VoiceReply, VoiceReplyError, VoiceWireRequest,
        };
        let result = (|| {
            let request: VoiceWireRequest = transport::read_frame(&mut self.stream)
                .map_err(|_| ConnectionError::ControlFrame)?;
            match request {
                VoiceWireRequest::Personalization(request) => {
                    use inputia_handy_runtime::personalization_wire::{
                        PersonalizationReply, CAPABILITY,
                    };
                    let accepted = history.policy_epoch().ok().is_some_and(|epoch| {
                        request
                            .validate_for(&self.context.voice_peer(epoch))
                            .is_ok()
                    }) && self.client.capabilities.iter().any(|c| c == CAPABILITY);
                    let reply = match (accepted, app) {
                        (true, Some(app)) => crate::personalization::respond(app, &request),
                        _ => PersonalizationReply {
                            status: "personalization".into(),
                            request_id: request.request_id,
                            server_instance: request.server_instance,
                            enabled: false,
                            epoch: 0,
                            result: None,
                            code: Some("personalization_unauthorized".into()),
                        },
                    };
                    transport::write_frame(&mut self.stream, &reply)
                        .map_err(|_| ConnectionError::ControlFrame)
                }
                VoiceWireRequest::TypedCapture(request) => {
                    use inputia_handy_runtime::voice_protocol::{
                        TypedCaptureReply, TYPED_CAPTURE_CAPABILITY,
                    };
                    let accepted = history.policy_epoch().ok().is_some_and(|epoch| {
                        request
                            .validate_for(&self.context.voice_peer(epoch))
                            .is_ok()
                    }) && self
                        .client
                        .capabilities
                        .iter()
                        .any(|c| c == TYPED_CAPTURE_CAPABILITY);
                    let reply = match (accepted, app) {
                        (true, Some(app)) => crate::typed_capture::respond(app, &request),
                        _ => TypedCaptureReply {
                            status: "typed_capture".into(),
                            request_id: request.request_id,
                            server_instance: request.server_instance,
                            enabled: false,
                            epoch: 0,
                            saved: false,
                            code: Some("typed_capture_unauthorized".into()),
                        },
                    };
                    transport::write_frame(&mut self.stream, &reply)
                        .map_err(|_| ConnectionError::ControlFrame)
                }
                VoiceWireRequest::TargetBridge(request) => {
                    use inputia_handy_runtime::voice_protocol::{
                        TargetBridgeCommand, TargetBridgePurpose, TargetBridgeReply,
                    };
                    let result = (|| {
                        let epoch = history
                            .policy_epoch()
                            .map_err(|_| "target_policy_unavailable")?;
                        request
                            .validate_for(&self.context.voice_peer(epoch))
                            .map_err(|_| "target_unauthorized")?;
                        if !self.client.capabilities.iter().any(|capability| {
                            capability
                                == inputia_handy_runtime::voice_protocol::IME_TARGET_CAPABILITY
                        }) {
                            return Err("target_capability_required");
                        }
                        if let TargetBridgeCommand::Validate {
                            target,
                            purpose: TargetBridgePurpose::Dispatch,
                            operation_id,
                        } = &request.target_bridge
                        {
                            let operation =
                                operation_id.as_ref().ok_or("target_operation_missing")?;
                            if self.dispatched_outputs.get(operation).is_none_or(|intent| {
                                intent.target_id.as_deref() != Some(&target.target_id)
                            }) {
                                return Err("target_delivery_not_owned");
                            }
                            let output = history
                                .output_record(operation.clone())
                                .map_err(|_| "target_output_unavailable")?
                                .ok_or("target_output_unavailable")?;
                            if output.state
                                != inputia_handy_runtime::output_ledger::OutputState::Dispatched
                                || self.dispatched_outputs.get(operation) != Some(&output.intent)
                            {
                                return Err("target_delivery_retired");
                            }
                        }
                        Ok(crate::ime_target_broker::respond(
                            app.ok_or("target_app_unavailable")?,
                            request.clone(),
                        ))
                    })();
                    let reply = result.unwrap_or_else(|code| TargetBridgeReply {
                        status: "target_bridge".into(),
                        request_id: request.request_id,
                        ready: false,
                        server_instance: request.server_instance,
                        permission_epoch: 0,
                        valid_for_ms: 0,
                        target: None,
                        selection: None,
                        dispatch_nonce: None,
                        code: Some(code.into()),
                    });
                    transport::write_frame(&mut self.stream, &reply)
                        .map_err(|_| ConnectionError::ControlFrame)
                }
                VoiceWireRequest::SharedTerms(request) => {
                    use inputia_core::integration::{
                        privacy::{HistoryMode, PrivacyContext, PrivacyPolicy, SourceTrust},
                        terms::HotwordBudget,
                    };
                    use inputia_handy_runtime::voice_protocol::{
                        SharedTermsReply, VoiceTermsVersion,
                    };
                    let request_id = request.request_id.clone();
                    let result = (|| {
                        let epoch = history
                            .policy_epoch()
                            .map_err(|_| VoiceReplyError::Unknown)?;
                        request
                            .validate_for(&self.context.voice_peer(epoch))
                            .map_err(|_| VoiceReplyError::Unauthorized)?;
                        let app = app.ok_or(VoiceReplyError::Unauthorized)?;
                        let broker = app
                            .try_state::<crate::host_shortcut_broker::HostShortcutBroker>()
                            .ok_or(VoiceReplyError::Unauthorized)?;
                        let (lease, expires_at) = broker.shared_terms_lease(&request)?;
                        crate::ime_target_broker::check(
                            app,
                            &request.client_instance,
                            &request.server_instance,
                            &lease.target,
                            inputia_handy_runtime::voice_protocol::TargetBridgePurpose::SharedTerms,
                        )
                        .map_err(|_| VoiceReplyError::Unauthorized)?;
                        let snapshot = history
                            .session_hotwords(
                                PrivacyPolicy {
                                    epoch,
                                    history_enabled: false,
                                    history_mode: HistoryMode::Strict,
                                    learning_enabled: false,
                                    remote_learning_terms_enabled: false,
                                },
                                PrivacyContext {
                                    source_trust: SourceTrust::Unknown,
                                    source_sensitive: false,
                                    target_known: true,
                                    target_sensitive: false,
                                    secure_input: false,
                                    transient_or_concealed: false,
                                },
                                vec![],
                                HotwordBudget::default(),
                            )
                            .map_err(|_| VoiceReplyError::Unknown)?;
                        if !history
                            .term_snapshot_is_current(snapshot.clone())
                            .map_err(|_| VoiceReplyError::Unknown)?
                        {
                            return Err(VoiceReplyError::Unauthorized);
                        }
                        let (current, current_expiry) = broker.shared_terms_lease(&request)?;
                        if current != lease || current_expiry != expires_at {
                            return Err(VoiceReplyError::Unauthorized);
                        }
                        let max_age_ms = expires_at
                            .saturating_duration_since(std::time::Instant::now())
                            .as_millis()
                            .min(1000) as u64;
                        if max_age_ms == 0 {
                            return Err(VoiceReplyError::Unauthorized);
                        }
                        crate::ime_target_broker::check(
                            app,
                            &request.client_instance,
                            &request.server_instance,
                            &lease.target,
                            inputia_handy_runtime::voice_protocol::TargetBridgePurpose::SharedTerms,
                        )
                        .map_err(|_| VoiceReplyError::Unauthorized)?;
                        Ok(SharedTermsReply::SharedTerms {
                            request_id: request.request_id,
                            lease_id: lease.lease_id,
                            lease_epoch: lease.lease_epoch,
                            version: VoiceTermsVersion {
                                policy_epoch: snapshot.policy_epoch,
                                learning_generation: snapshot.learning_generation,
                            },
                            terms: snapshot.terms,
                            max_age_ms,
                        })
                    })();
                    let reply = result
                        .unwrap_or_else(|code| SharedTermsReply::Rejected { request_id, code });
                    transport::write_frame(&mut self.stream, &reply)
                        .map_err(|_| ConnectionError::ControlFrame)
                }
                VoiceWireRequest::Menu(request) => {
                    use inputia_handy_runtime::voice_protocol::MenuReply;
                    let request_id = request.request_id.clone();
                    let result = self
                        .context
                        .authorize_menu(&request, history)
                        .and_then(|()| app.ok_or(DispatchError::Unauthorized))
                        .and_then(|app| menu_action(app, &request));
                    let reply = result.unwrap_or_else(|error| MenuReply::Rejected {
                        request_id,
                        code: reply_error(error),
                    });
                    transport::write_frame(&mut self.stream, &reply)
                        .map_err(|_| ConnectionError::ControlFrame)
                }
                VoiceWireRequest::HostShortcut(request) => {
                    use inputia_handy_runtime::voice_protocol::HostShortcutReply;
                    let request_id = request.request_id.clone();
                    let reply = match app.ok_or(DispatchError::Unauthorized).and_then(|app| {
                        let broker = app
                            .try_state::<crate::host_shortcut_broker::HostShortcutBroker>()
                            .ok_or(DispatchError::Unauthorized)?;
                        let epoch = history.policy_epoch().map_err(|_| DispatchError::Unknown)?;
                        request
                            .validate_for(&self.context.voice_peer(epoch))
                            .map_err(|_| DispatchError::Unauthorized)?;
                        if let inputia_handy_runtime::voice_protocol::HostShortcutCommand::Register { lease } = &request.shortcut {
                            crate::ime_target_broker::check(app, &request.client_instance, &request.server_instance,
                                &lease.target, inputia_handy_runtime::voice_protocol::TargetBridgePurpose::Start)
                                .map_err(|_| DispatchError::Unauthorized)?;
                        }
                        Ok(broker.process_request(request))
                    }) {
                        Ok(reply) => reply,
                        Err(error) => HostShortcutReply::Rejected {
                            request_id,
                            code: reply_error(error),
                        },
                    };
                    transport::write_frame(&mut self.stream, &reply)
                        .map_err(|_| ConnectionError::ControlFrame)
                }
                VoiceWireRequest::Control(request) => {
                    if request.request_id.is_empty()
                        || request.request_id.len() > 256
                        || request.request_id.chars().any(char::is_control)
                    {
                        return Err(ConnectionError::ControlFrame);
                    }
                    let request_id = request.request_id.clone();
                    if let Some((target, _)) = request.strict_start_identity() {
                        let authorized = app.ok_or(DispatchError::Unauthorized).and_then(|app| {
                            let epoch =
                                history.policy_epoch().map_err(|_| DispatchError::Unknown)?;
                            request
                                .validate_for(&self.context.voice_peer(epoch))
                                .map_err(|_| DispatchError::Unauthorized)?;
                            crate::ime_target_broker::check(
                                app,
                                &request.client_instance,
                                &request.server_instance,
                                target,
                                inputia_handy_runtime::voice_protocol::TargetBridgePurpose::Start,
                            )
                            .map_err(|_| DispatchError::Unauthorized)
                        });
                        if let Err(error) = authorized {
                            return transport::write_frame(
                                &mut self.stream,
                                &VoiceReply::Rejected {
                                    request_id,
                                    code: reply_error(error),
                                },
                            )
                            .map_err(|_| ConnectionError::ControlFrame);
                        }
                    }
                    if matches!(
                        request.command,
                        inputia_handy_runtime::voice_protocol::VoiceCommand::HostShortcut { .. }
                    ) {
                        let authorized = app
                            .and_then(|app| {
                                app.try_state::<crate::host_shortcut_broker::HostShortcutBroker>()
                            })
                            .ok_or(DispatchError::Unauthorized)
                            .and_then(|broker| {
                                broker
                                    .consume_voice_command(&request)
                                    .map_err(|_| DispatchError::Unauthorized)
                            });
                        if let Err(error) = authorized {
                            let reply = VoiceReply::Rejected {
                                request_id,
                                code: reply_error(error),
                            };
                            return transport::write_frame(&mut self.stream, &reply)
                                .map_err(|_| ConnectionError::ControlFrame);
                        }
                    }
                    let reply = match VoiceDispatcher::new(history, coordinator)
                        .dispatch(&self.context, request)
                    {
                        Ok(view) => VoiceReply::Session { request_id, view },
                        Err(error) => VoiceReply::Rejected {
                            request_id,
                            code: reply_error(error),
                        },
                    };
                    transport::write_frame(&mut self.stream, &reply)
                        .map_err(|_| ConnectionError::ControlFrame)
                }
                VoiceWireRequest::Output(request) => {
                    if request.request_id.is_empty()
                        || request.request_id.len() > 256
                        || request.request_id.chars().any(char::is_control)
                    {
                        return Err(ConnectionError::ControlFrame);
                    }
                    let request_id = request.request_id.clone();
                    let operation_id = match &request.output {
                        VoiceOutputCommand::Receipt { operation_id, .. } => {
                            Some(operation_id.clone())
                        }
                        VoiceOutputCommand::Fetch {} => None,
                    };
                    if operation_id.is_none() {
                        let authorized = (|| -> Result<(), DispatchError> {
                            let epoch =
                                history.policy_epoch().map_err(|_| DispatchError::Unknown)?;
                            request
                                .validate_for(&self.context.voice_peer(epoch))
                                .map_err(|_| DispatchError::Unauthorized)?;
                            let record = history
                                .voice_session(request.session_id.clone())
                                .map_err(|_| DispatchError::Unknown)?
                                .ok_or(DispatchError::MissingSession)?;
                            if record.start.client_instance != request.client_instance
                                || record.start.server_instance != request.server_instance
                            {
                                return Err(DispatchError::Unauthorized);
                            }
                            let (target, _) = record
                                .start
                                .start_identity()
                                .ok_or(DispatchError::Unauthorized)?;
                            crate::ime_target_broker::check(app.ok_or(DispatchError::Unauthorized)?, &request.client_instance,
                                &request.server_instance, target, inputia_handy_runtime::voice_protocol::TargetBridgePurpose::SharedTerms)
                                .map_err(|_| DispatchError::Unauthorized)
                        })();
                        if let Err(error) = authorized {
                            return transport::write_frame(
                                &mut self.stream,
                                &VoiceOutputReply::Rejected {
                                    request_id,
                                    code: reply_error(error),
                                },
                            )
                            .map_err(|_| ConnectionError::ControlFrame);
                        }
                    }
                    let dispatcher = VoiceDispatcher::new(history, coordinator);
                    let (mut reply, sent_intent) = match operation_id {
                        Some(operation_id) => {
                            if let Some(intent) =
                                self.dispatched_outputs.get(&operation_id).cloned()
                            {
                                match dispatcher.receipt(&self.context, request, &intent) {
                                    Ok(reply) => (reply, None),
                                    Err(error) => (
                                        VoiceOutputReply::Rejected {
                                            request_id: request_id.clone(),
                                            code: reply_error(error),
                                        },
                                        None,
                                    ),
                                }
                            } else {
                                (
                                    VoiceOutputReply::Rejected {
                                        request_id: request_id.clone(),
                                        code: VoiceReplyError::Unauthorized,
                                    },
                                    None,
                                )
                            }
                        }
                        None => match dispatcher.fetch_output(&self.context, request) {
                            Ok(result) => result,
                            Err(error) => (
                                VoiceOutputReply::Rejected {
                                    request_id: request_id.clone(),
                                    code: reply_error(error),
                                },
                                None,
                            ),
                        },
                    };
                    let sent_intent = if let Some((intent, permit)) = sent_intent {
                        if permit.check().is_err() {
                            let output = history
                                .finish_output(intent.clone(), OutputOutcome::NotDispatchedRejected)
                                .map_err(|_| ConnectionError::ControlFrame)?;
                            reply = VoiceOutputReply::Output {
                                request_id: request_id.clone(),
                                operation_id: output.intent.operation_id,
                                state: output.state,
                            };
                            None
                        } else {
                            Some(intent)
                        }
                    } else {
                        None
                    };
                    transport::write_frame(&mut self.stream, &reply)
                        .map_err(|_| ConnectionError::ControlFrame)?;
                    if let Some(intent) = sent_intent {
                        self.dispatched_outputs
                            .insert(intent.operation_id.clone(), intent);
                    }
                    Ok(())
                }
            }
        })();
        if result.is_err() {
            let _ = self.stream.shutdown(std::net::Shutdown::Both);
        }
        result
    }
}

/// 已认证后台菜单入口；请求先占用去重槽，失败/回执未知均不可自动重放。
fn menu_action(
    app: &tauri::AppHandle,
    request: &inputia_handy_runtime::voice_protocol::MenuRequest,
) -> Result<inputia_handy_runtime::voice_protocol::MenuReply, DispatchError> {
    use inputia_handy_runtime::voice_protocol::{MenuCommand, MenuModel, MenuReply};
    use std::sync::{Arc, Mutex, OnceLock};
    use tauri::{Emitter, Manager};
    use tauri_plugin_clipboard_manager::ClipboardExt;

    static CLAIMS: OnceLock<Mutex<std::collections::HashSet<(String, String)>>> = OnceLock::new();
    let busy = crate::tray::service_is_busy(app);
    if request.menu != MenuCommand::Status {
        if busy && !request.menu.allowed_while_busy() {
            return Err(DispatchError::CoordinatorRejected);
        }
        let mut claims = CLAIMS
            .get_or_init(Mutex::default)
            .lock()
            .map_err(|_| DispatchError::Unknown)?;
        // 不淘汰旧请求，否则失去同一服务实例的禁止重放保证。
        if claims.len() >= 16384
            || !claims.insert((request.client_instance.clone(), request.request_id.clone()))
        {
            return Err(DispatchError::Unknown);
        }
    }
    match &request.menu {
        MenuCommand::Status => {}
        MenuCommand::CopyLatest => {
            let entry = app
                .state::<Arc<crate::managers::history::HistoryManager>>()
                .get_latest_completed_entry()
                .map_err(|_| DispatchError::Unknown)?
                .ok_or(DispatchError::CoordinatorRejected)?;
            let text = entry
                .post_processed_text
                .as_deref()
                .unwrap_or(&entry.transcription_text);
            if text.trim().is_empty() {
                return Err(DispatchError::CoordinatorRejected);
            }
            app.clipboard()
                .write_text(text)
                .map_err(|_| DispatchError::Unknown)?;
        }
        MenuCommand::SelectModel { model_id } => {
            // 管理器执行已下载目录验证和唯一加载槽，不接受路径或远程下载指令。
            crate::commands::models::switch_active_model(app, model_id)
                .map_err(|_| DispatchError::CoordinatorRejected)?;
        }
        MenuCommand::UnloadModel => {
            app.state::<Arc<crate::managers::transcription::TranscriptionManager>>()
                .unload_model()
                .map_err(|_| DispatchError::CoordinatorRejected)?;
        }
        command => {
            if *command == MenuCommand::CheckUpdates
                && !crate::settings::update_checks_effectively_enabled(
                    &crate::settings::get_settings(app),
                )
            {
                return Err(DispatchError::CoordinatorRejected);
            }
            let command = command.clone();
            let handle = app.clone();
            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            let claim = Arc::new(std::sync::atomic::AtomicU8::new(0));
            let scheduled = claim.clone();
            app.run_on_main_thread(move || {
                use std::sync::atomic::Ordering;
                if scheduled
                    .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
                    .is_err()
                {
                    return;
                }
                let result = match command {
                    MenuCommand::History => {
                        // 先由召回浮窗捕获原目标，不打开控制中心抢走焦点。
                        crate::overlay::show_clipboard_overlay(&handle);
                        Ok(())
                    }
                    MenuCommand::Settings => {
                        crate::overlay::hide_clipboard_overlay(&handle);
                        crate::show_main_window(&handle);
                        handle
                            .emit("navigate-to", "general")
                            .map_err(|_| DispatchError::Unknown)
                    }
                    MenuCommand::CheckUpdates => {
                        crate::show_main_window(&handle);
                        handle
                            .emit("check-for-updates", ())
                            .map_err(|_| DispatchError::Unknown)
                    }
                    MenuCommand::QuitService => {
                        handle.exit(0);
                        Ok(())
                    }
                    _ => Err(DispatchError::Unauthorized),
                };
                let _ = tx.send(result);
            })
            .map_err(|_| DispatchError::Unknown)?;
            match rx.recv_timeout(std::time::Duration::from_secs(1)) {
                Ok(result) => result?,
                Err(_) => {
                    let _ = claim.compare_exchange(
                        0,
                        2,
                        std::sync::atomic::Ordering::AcqRel,
                        std::sync::atomic::Ordering::Acquire,
                    );
                    return Err(DispatchError::Unknown);
                }
            }
        }
    }
    let mut models: Vec<MenuModel> = app
        .state::<Arc<crate::managers::model::ModelManager>>()
        .get_available_models()
        .into_iter()
        .map(|model| MenuModel {
            id: model.id,
            name: model.name,
            available: model.is_downloaded,
        })
        .collect();
    models.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(MenuReply::Menu {
        request_id: request.request_id.clone(),
        selected_model: crate::settings::get_settings(app).selected_model,
        models,
        busy: crate::tray::service_is_busy(app),
    })
}

fn reply_error(error: DispatchError) -> inputia_handy_runtime::voice_protocol::VoiceReplyError {
    use inputia_handy_runtime::voice_protocol::VoiceReplyError;
    match error {
        DispatchError::Unauthorized => VoiceReplyError::Unauthorized,
        DispatchError::MissingSession => VoiceReplyError::MissingSession,
        DispatchError::Unknown => VoiceReplyError::Unknown,
        DispatchError::CoordinatorRejected => VoiceReplyError::CoordinatorRejected,
    }
}

/// 接到实际候选App启动；日常构建不启用，缺少已签名配对材料时明确拒绝监听。
struct VoiceClientConnectionGuard {
    app: tauri::AppHandle,
    client: String,
    server: String,
}

impl Drop for VoiceClientConnectionGuard {
    fn drop(&mut self) {
        crate::ime_target_broker::disconnected(&self.app, &self.client, &self.server);
        use tauri::Manager;
        let notify = || {
            self.app
                .state::<crate::TranscriptionCoordinator>()
                .notify_voice_client_disconnected(&self.client);
        };
        if let Some(broker) = self
            .app
            .try_state::<crate::host_shortcut_broker::HostShortcutBroker>()
        {
            broker.note_disconnected_and_notify(&self.client, notify);
        } else {
            notify();
        }
    }
}

pub(crate) fn start_candidate_listener(app: &tauri::AppHandle) {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    use std::{
        io::Read,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        time::Duration,
    };
    use tauri::Manager;
    let Some(profile) = crate::candidate_profile::current().cloned() else {
        return;
    };
    let Some(manager) = app.try_state::<Arc<crate::managers::integration::IntegrationManager>>()
    else {
        return;
    };
    let service = manager.service.clone();
    let app = app.clone();
    let stop = Arc::new(AtomicBool::new(false));
    app.manage(ListenerLifetime(stop.clone()));
    std::thread::spawn(move || {
        let run = || -> Result<(), String> {
            let trust = crate::native_pair_auth::candidate_build_trust(&profile.profile_id)
                .map_err(|_| "embedded_profile_mismatch")?
                .ok_or("missing_embedded_pair_key")?;
            let manifest_path = profile
                .handy_root
                .parent()
                .ok_or("missing_profile_root")?
                .join("pair-manifest.json");
            let file = std::fs::OpenOptions::new()
                .read(true)
                // Darwin SDK sys/fcntl.h: O_NOFOLLOW = 0x00000100。
                .custom_flags(0x00000100)
                .open(&manifest_path)
                .map_err(|_| "missing_pair_manifest")?;
            let metadata = file.metadata().map_err(|_| "manifest_metadata")?;
            // SAFETY: geteuid无指针或副作用，仅核对当前用户拥有的候选材料。
            unsafe extern "C" {
                fn geteuid() -> u32;
            }
            let uid = unsafe { geteuid() };
            if !metadata.is_file()
                || metadata.len() > 16_384
                || metadata.uid() != uid
                || metadata.nlink() != 1
                || metadata.mode() & 0o022 != 0
            {
                return Err("unsafe_pair_manifest".into());
            }
            let mut bytes = Vec::new();
            file.take(16_385)
                .read_to_end(&mut bytes)
                .map_err(|_| "manifest_read")?;
            let manifest =
                PairManifest::load(&bytes, &trust).map_err(|_| "pair_manifest_rejected")?;
            let version = service.voice_terms_version()?;
            let instance = inputia_handy_runtime::voice_protocol::VoicePolicyBarrier::new(version)
                .map_err(|_| "instance_entropy")?
                .barrier_id;
            use sha2::{Digest, Sha256};
            let profile_hash = format!("{:x}", Sha256::digest(profile.profile_id.as_bytes()));
            let socket = std::path::PathBuf::from(format!(
                "/private/tmp/handy-unified-{uid}-{}",
                &profile_hash[..12]
            ))
            .join(format!("{}.sock", &instance[..24]));
            let listener =
                transport::PrivateListener::bind(&socket).map_err(|_| "listener_bind")?;
            listener
                .set_nonblocking(true)
                .map_err(|_| "listener_nonblocking")?;
            let discovery = serde_json::json!({"protocol_major":1,"profile_id":profile.profile_id,"server_instance":instance,"socket_path":socket});
            let temporary = profile
                .handy_root
                .join(format!(".endpoint-{instance}.json"));
            let mut output = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)
                .map_err(|_| "endpoint_create")?;
            use std::io::Write;
            output
                .write_all(&serde_json::to_vec(&discovery).map_err(|_| "endpoint_encode")?)
                .map_err(|_| "endpoint_write")?;
            output.sync_all().map_err(|_| "endpoint_sync")?;
            std::fs::rename(
                &temporary,
                profile.handy_root.join("integration-endpoint.json"),
            )
            .map_err(|_| "endpoint_publish")?;
            log::info!(
                "unified_voice_listener_ready profile={}",
                profile.profile_id
            );
            let server = Handshake {
                protocol_major: 1,
                protocol_minor: 0,
                instance_id: instance,
                profile_id: profile.profile_id.clone(),
                policy_epoch: service.policy_epoch()?,
                capabilities: vec![
                    inputia_handy_runtime::voice_protocol::VOICE_CAPABILITY.into(),
                    inputia_handy_runtime::voice_protocol::SHARED_TERMS_CAPABILITY.into(),
                    inputia_handy_runtime::voice_protocol::IME_TARGET_CAPABILITY.into(),
                    inputia_handy_runtime::voice_protocol::TYPED_CAPTURE_CAPABILITY.into(),
                    inputia_handy_runtime::personalization_wire::CAPABILITY.into(),
                ],
            };
            drop(manifest);
            // Swift 认证 handle 有线程归属；只共享已验证的字节和静态构建信任。
            let manifest_bytes = Arc::new(bytes);
            let trust = Arc::new(trust);
            let mut clients: Vec<std::thread::JoinHandle<()>> = Vec::new();
            while !stop.load(Ordering::Acquire) {
                let stream = match listener.accept() {
                    Ok(stream) => stream,
                    Err(ProtocolError::Timeout) => {
                        std::thread::sleep(Duration::from_millis(50));
                        continue;
                    }
                    Err(_) => return Err("listener_accept".into()),
                };
                clients.retain(|client| !client.is_finished());
                if clients.len() >= 8 {
                    drop(stream);
                    log::warn!("unified_voice_connection_rejected stage=connection_limit");
                    continue;
                }
                let app = app.clone();
                let service = service.clone();
                let stop = stop.clone();
                let manifest_bytes = manifest_bytes.clone();
                let trust = trust.clone();
                let server = server.clone();
                clients.push(std::thread::spawn(move || {
                    let manifest = match PairManifest::load(&manifest_bytes, &trust) {
                        Ok(manifest) => manifest,
                        Err(_) => {
                            log::warn!("unified_voice_connection_rejected stage=thread_manifest");
                            return;
                        }
                    };
                    let mut connection = match VoiceConnection::accept(
                        stream,
                        &manifest,
                        server.clone(),
                        &service,
                    ) {
                        Ok(connection) => connection,
                        Err(error) => {
                            log::warn!("unified_voice_connection_rejected stage={error:?}");
                            return;
                        }
                    };
                    if let Some(broker) =
                        app.try_state::<crate::host_shortcut_broker::HostShortcutBroker>()
                    {
                        broker.note_connected(connection.client_instance());
                    }
                    crate::ime_target_broker::connected(
                        connection.client_instance(),
                        &server.instance_id,
                    );
                    let _client_guard = VoiceClientConnectionGuard {
                        app: app.clone(),
                        client: connection.client_instance().to_owned(),
                        server: server.instance_id.clone(),
                    };
                    if connection.synchronize_policy(&service).is_err() {
                        log::warn!("unified_voice_connection_rejected stage=policy_sync");
                        return;
                    }
                    let coordinator = app.state::<crate::TranscriptionCoordinator>();
                    while !stop.load(Ordering::Acquire) {
                        if connection
                            .process_one_with_app(&service, &*coordinator, Some(&app))
                            .is_err()
                        {
                            break;
                        }
                    }
                }));
            }
            for client in clients {
                let _ = client.join();
            }
            Ok(())
        };
        if let Err(reason) = run() {
            log::warn!("unified_voice_listener_unavailable reason={reason}");
        }
    });
}

struct ListenerLifetime(std::sync::Arc<std::sync::atomic::AtomicBool>);
impl Drop for ListenerLifetime {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Release);
    }
}

pub(crate) fn stop_candidate_listener(app: &tauri::AppHandle) {
    use tauri::Manager;
    if let Some(lifetime) = app.try_state::<ListenerLifetime>() {
        lifetime.0.store(true, std::sync::atomic::Ordering::Release);
    }
}

pub(crate) fn candidate_listener_stopping(app: &tauri::AppHandle) -> bool {
    use tauri::Manager;
    app.try_state::<ListenerLifetime>()
        .is_some_and(|lifetime| lifetime.0.load(std::sync::atomic::Ordering::Acquire))
}
