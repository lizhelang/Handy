//! 发布目录信任：密码验证、文档语义、新鲜度和安装授权分别处理。
//! 本库不访问凭据、不联网、不安装、不把数学验签等同于已通过产品验收。

#[cfg(unix)]
pub mod artifacts;
pub mod canonical;
#[cfg(unix)]
pub mod catalog;
pub mod feed;
pub mod manifest;
mod schema;
#[cfg(unix)]
pub mod state;
pub mod trust;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseError {
    InvalidDocument,
    InvalidSignature,
    UntrustedKey,
    Expired,
    WrongProduct,
    WrongPurpose,
    Replay,
    ConflictingSequence,
    Incompatible,
    UnsafePath,
    DigestMismatch,
    StateUnavailable,
}

impl std::fmt::Display for ReleaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidDocument => "发布文档格式无效",
            Self::InvalidSignature => "发布签名验证失败",
            Self::UntrustedKey => "发布密钥未获授权或已撤销",
            Self::Expired => "发布授权不在有效时段",
            Self::WrongProduct => "发布产品身份不匹配",
            Self::WrongPurpose => "发布签名用途不匹配",
            Self::Replay => "发布序号发生回退",
            Self::ConflictingSequence => "同一发布序号出现不同内容",
            Self::Incompatible => "发布目标或兼容合同不匹配",
            Self::UnsafePath => "发布制品路径不安全",
            Self::DigestMismatch => "发布文档或制品摘要不匹配",
            Self::StateUnavailable => "发布信任状态无法安全读写",
        })
    }
}
impl std::error::Error for ReleaseError {}
pub type Result<T> = std::result::Result<T, ReleaseError>;

pub fn digest(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub(crate) fn valid_id(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'-'))
}

pub fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

pub fn safe_relative(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 1024
        || value.contains(['\\', ':'])
        || value.chars().any(char::is_control)
        || value.split('/').any(|part| matches!(part, "" | "." | ".."))
    {
        return Err(ReleaseError::UnsafePath);
    }
    Ok(())
}

pub(crate) fn utc(value: &str) -> Result<i64> {
    if value.len() != 20 || !value.ends_with('Z') {
        return Err(ReleaseError::InvalidDocument);
    }
    time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
        .map(|value| value.unix_timestamp())
        .map_err(|_| ReleaseError::InvalidDocument)
}
