//! 产品后台连接的认证入口；消费socket所有权，任何失败直接关闭，不保留半认证连接。
use crate::native_pair_auth::{PairManifest, PeerRole};
use crate::voice_dispatch::{
    AuthenticatedVoiceConnection, DispatchError, VoiceCoordinatorPort, VoiceDispatcher,
};
use inputia_handy_runtime::{
    protocol::{Handshake, HandshakePolicy, ProtocolError},
    service::HistoryService,
    transport,
};
use std::os::{fd::AsFd, unix::net::UnixStream};

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
        use inputia_handy_runtime::voice_protocol::{VoiceReply, VoiceReplyError, VoiceRequest};
        let result = (|| {
            let request: VoiceRequest = transport::read_frame(&mut self.stream)
                .map_err(|_| ConnectionError::ControlFrame)?;
            if request.request_id.is_empty()
                || request.request_id.len() > 256
                || request.request_id.chars().any(char::is_control)
            {
                return Err(ConnectionError::ControlFrame);
            }
            let request_id = request.request_id.clone();
            let reply = match VoiceDispatcher::new(history, coordinator)
                .dispatch(&self.context, request)
            {
                Ok(view) => VoiceReply::Session { request_id, view },
                Err(error) => VoiceReply::Rejected {
                    request_id,
                    code: match error {
                        DispatchError::Unauthorized => VoiceReplyError::Unauthorized,
                        DispatchError::MissingSession => VoiceReplyError::MissingSession,
                        DispatchError::Unknown => VoiceReplyError::Unknown,
                        DispatchError::CoordinatorRejected => VoiceReplyError::CoordinatorRejected,
                    },
                },
            };
            transport::write_frame(&mut self.stream, &reply)
                .map_err(|_| ConnectionError::ControlFrame)
        })();
        if result.is_err() {
            let _ = self.stream.shutdown(std::net::Shutdown::Both);
        }
        result
    }
}

/// 接到实际候选App启动；日常构建不启用，缺少已签名配对材料时明确拒绝监听。
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
                capabilities: vec![inputia_handy_runtime::voice_protocol::VOICE_CAPABILITY.into()],
            };
            while !stop.load(Ordering::Acquire) {
                let stream = match listener.accept() {
                    Ok(stream) => stream,
                    Err(ProtocolError::Timeout) => {
                        std::thread::sleep(Duration::from_millis(50));
                        continue;
                    }
                    Err(_) => return Err("listener_accept".into()),
                };
                let mut connection =
                    match VoiceConnection::accept(stream, &manifest, server.clone(), &service) {
                        Ok(connection) => connection,
                        Err(error) => {
                            log::warn!("unified_voice_connection_rejected stage={error:?}");
                            continue;
                        }
                    };
                if connection.synchronize_policy(&service).is_err() {
                    log::warn!("unified_voice_connection_rejected stage=policy_sync");
                    continue;
                }
                let coordinator = app.state::<crate::TranscriptionCoordinator>();
                while !stop.load(Ordering::Acquire) {
                    if connection.process_one(&service, &*coordinator).is_err() {
                        break;
                    }
                }
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
