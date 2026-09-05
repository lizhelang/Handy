//! 本地控制协议；正文、附件和输出状态由业务模块提供。

use serde::{Deserialize, Serialize};
use std::{fmt, io};

pub const PROTOCOL_MAJOR: u16 = 1;
pub const PROTOCOL_MINOR: u16 = 0;
pub const MAX_FRAME_BYTES: usize = 256 * 1024;

/// 每个进程重启必须更换 instance_id；profile_id 标识安装及数据域。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Handshake {
    pub protocol_major: u16,
    pub protocol_minor: u16,
    pub instance_id: String,
    pub profile_id: String,
    pub policy_epoch: u64,
    pub capabilities: Vec<String>,
}

/// 服务端当前策略；旧客户端须先刷新撤销屏障，再发送业务写入。
#[derive(Clone, Debug)]
pub struct HandshakePolicy {
    pub profile_id: String,
    pub current_policy_epoch: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandshakeRejection {
    InvalidIdentity,
    IncompatibleVersion,
    ProfileMismatch,
    FuturePolicyEpoch,
}

/// Accepted 只表示通讯建立，不能替代策略屏障和业务授权。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum HandshakeReply {
    Accepted {
        server: Handshake,
        negotiated_minor: u16,
        require_policy_refresh: bool,
    },
    Rejected {
        reason: HandshakeRejection,
    },
}

impl Handshake {
    /// 身份字段仅允许短的不透明标识，不允许控制字符。
    pub fn validate(&self, policy: &HandshakePolicy) -> Result<(), HandshakeRejection> {
        if !valid_id(&self.instance_id)
            || !valid_id(&self.profile_id)
            || self.capabilities.len() > 64
            || self.capabilities.iter().any(|value| !valid_id(value))
        {
            return Err(HandshakeRejection::InvalidIdentity);
        }
        if self.protocol_major != PROTOCOL_MAJOR {
            return Err(HandshakeRejection::IncompatibleVersion);
        }
        if self.profile_id != policy.profile_id {
            return Err(HandshakeRejection::ProfileMismatch);
        }
        if self.policy_epoch > policy.current_policy_epoch {
            return Err(HandshakeRejection::FuturePolicyEpoch);
        }
        Ok(())
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlVerb {
    StartVoice,
    StopVoice,
    CancelVoice,
    Status,
    Heartbeat,
}

/// operation_id 仅关联动作，不承诺跨崩溃 exactly-once。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlEnvelope<T> {
    pub request_id: String,
    pub session_id: Option<String>,
    pub operation_id: Option<String>,
    pub verb: ControlVerb,
    pub payload: T,
}

impl<T> ControlEnvelope<T> {
    /// 反序列化后、执行任何副作用前校验不透明 ID。
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if !valid_id(&self.request_id)
            || self.session_id.as_ref().is_some_and(|id| !valid_id(id))
            || self.operation_id.as_ref().is_some_and(|id| !valid_id(id))
            || (matches!(
                self.verb,
                ControlVerb::StartVoice | ControlVerb::StopVoice | ControlVerb::CancelVoice
            ) && self.session_id.is_none())
        {
            return Err(ProtocolError::InvalidEnvelope);
        }
        Ok(())
    }
}

/// 每条连接的写入屏障；只能在客户端已应用策略及遗忘快照后确认 epoch。
#[derive(Clone, Debug)]
pub struct PolicyGate {
    current_epoch: u64,
    acknowledged_epoch: Option<u64>,
}

impl PolicyGate {
    pub fn new(current_epoch: u64) -> Self {
        Self {
            current_epoch,
            acknowledged_epoch: None,
        }
    }

    /// 策略变化立即使旧回执失效，不能用较旧 epoch 回退服务端状态。
    pub fn advance(&mut self, epoch: u64) -> Result<(), ProtocolError> {
        if epoch < self.current_epoch {
            return Err(ProtocolError::PolicyRefreshRequired);
        }
        if epoch != self.current_epoch {
            self.current_epoch = epoch;
            self.acknowledged_epoch = None;
        }
        Ok(())
    }

    pub fn acknowledge(&mut self, epoch: u64) -> Result<(), ProtocolError> {
        if epoch != self.current_epoch {
            return Err(ProtocolError::PolicyRefreshRequired);
        }
        self.acknowledged_epoch = Some(epoch);
        Ok(())
    }

    /// 握手成功本身不开放新会话；停止/取消仍可关闭正在采集的会话。
    /// 既有 session 的归属及调用方认证由业务层验证。
    pub fn authorize(&self, verb: ControlVerb) -> Result<(), ProtocolError> {
        if matches!(
            verb,
            ControlVerb::Status
                | ControlVerb::Heartbeat
                | ControlVerb::StopVoice
                | ControlVerb::CancelVoice
        ) || self.acknowledged_epoch == Some(self.current_epoch)
        {
            Ok(())
        } else {
            Err(ProtocolError::PolicyRefreshRequired)
        }
    }
}

#[derive(Debug)]
pub enum ProtocolError {
    Io(io::Error),
    Json(serde_json::Error),
    FrameTooLarge,
    EmptyFrame,
    Timeout,
    UnsafeEndpoint,
    PeerIdentity,
    InvalidEnvelope,
    Handshake(HandshakeRejection),
    InvalidHandshakeReply,
    PolicyRefreshRequired,
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "本地通讯 I/O 错误: {error}"),
            Self::Json(_) => f.write_str("本地通讯 JSON 帧无效"),
            Self::FrameTooLarge => f.write_str("控制帧超过 256 KiB"),
            Self::EmptyFrame => f.write_str("控制帧不能为空"),
            Self::Timeout => f.write_str("本地通讯超过时限"),
            Self::UnsafeEndpoint => f.write_str("本地端点归属、类型或权限不安全"),
            Self::PeerIdentity => f.write_str("无法确认对端属于当前用户"),
            Self::InvalidEnvelope => f.write_str("控制请求标识无效"),
            Self::Handshake(reason) => write!(f, "本地握手被拒绝: {reason:?}"),
            Self::InvalidHandshakeReply => f.write_str("本地握手响应与请求不一致"),
            Self::PolicyRefreshRequired => f.write_str("策略屏障未确认，禁止业务写入"),
        }
    }
}

impl std::error::Error for ProtocolError {}

impl From<io::Error> for ProtocolError {
    fn from(error: io::Error) -> Self {
        if matches!(
            error.kind(),
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
        ) {
            Self::Timeout
        } else {
            Self::Io(error)
        }
    }
}

impl From<serde_json::Error> for ProtocolError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}
