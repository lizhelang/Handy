//! 产品后台连接的认证入口；消费socket所有权，任何失败直接关闭，不保留半认证连接。
use crate::native_pair_auth::{PairManifest, PeerRole};
use crate::voice_dispatch::AuthenticatedVoiceConnection;
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

    /// 仅在同一连接中执行handler；不允许将context提取并用于另一个连接。
    pub fn handle<T>(
        &mut self,
        handler: impl FnOnce(&mut UnixStream, &mut AuthenticatedVoiceConnection) -> T,
    ) -> T {
        handler(&mut self.stream, &mut self.context)
    }
}
