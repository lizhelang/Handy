//! macOS 候选配对认证。构建常量建立信任，运行时数据仅提供待验证 manifest。
//! 独立后台认证队列使用 RAII handle；不在按键/GUI 线程执行 Security 查询。

use std::fmt;
use std::marker::PhantomData;
use std::os::fd::{AsRawFd, BorrowedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::ptr::NonNull;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum PeerRole {
    Handy = 1,
    Inputia = 2,
}

#[cfg(unified_paired_build)]
mod embedded {
    include!(concat!(env!("OUT_DIR"), "/unified_pair_trust.rs"));
}

/// 只供已经完成候选安装身份校验的后台服务使用。没有签前公钥则不开放认证服务。
pub fn candidate_build_trust(profile_id: &str) -> Result<Option<EmbeddedPairTrust>, PairAuthError> {
    #[cfg(unified_paired_build)]
    {
        let Some((run_id, expected_profile)) = embedded::LEGACY_PROFILE else {
            return Err(PairAuthError::InvalidArgument);
        };
        if profile_id != expected_profile {
            return Err(PairAuthError::InvalidArgument);
        }
        Ok(Some(EmbeddedPairTrust::from_build_constants(
            &embedded::PUBLIC_KEY,
            embedded::KEY_ID,
            run_id,
            expected_profile,
            PeerRole::Handy,
        )))
    }
    #[cfg(not(unified_paired_build))]
    {
        let _ = profile_id;
        Ok(None)
    }
}

/// v2 的产品/发布信任来自编译常量；运行时收据只用于独立的安装与 profile 绑定。
pub fn release_build_trust() -> Result<Option<EmbeddedReleasePairTrust>, PairAuthError> {
    #[cfg(unified_paired_build)]
    {
        let Some((product_id, release_id, protocol_major)) = embedded::RELEASE_BINDING else {
            return Ok(None);
        };
        Ok(Some(EmbeddedReleasePairTrust::from_build_constants(
            &embedded::PUBLIC_KEY,
            embedded::KEY_ID,
            product_id,
            release_id,
            protocol_major,
            PeerRole::Handy,
        )))
    }
    #[cfg(not(unified_paired_build))]
    {
        Ok(None)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReleasePairBinding {
    pub product_id: &'static str,
    pub release_id: &'static str,
    pub protocol_major: u16,
}

/// 此类型不接受 Deserialize；不得从 manifest/收据/握手反向制造信任根。
pub struct EmbeddedReleasePairTrust {
    public_key: &'static [u8; 65],
    key_id: &'static str,
    binding: ReleasePairBinding,
    role: PeerRole,
}

impl EmbeddedReleasePairTrust {
    pub const fn from_build_constants(
        public_key: &'static [u8; 65],
        key_id: &'static str,
        product_id: &'static str,
        release_id: &'static str,
        protocol_major: u16,
        role: PeerRole,
    ) -> Self {
        Self {
            public_key,
            key_id,
            binding: ReleasePairBinding {
                product_id,
                release_id,
                protocol_major,
            },
            role,
        }
    }

    pub const fn release_binding(&self) -> ReleasePairBinding {
        self.binding
    }
}

/// 仅由签名前生成的静态构建数据创建，不实现 Deserialize，也不读环境/设置/握手。
pub struct EmbeddedPairTrust {
    public_key: &'static [u8; 65],
    key_id: &'static str,
    run_id: &'static str,
    profile_id: &'static str,
    role: PeerRole,
}

impl EmbeddedPairTrust {
    pub const fn from_build_constants(
        public_key: &'static [u8; 65],
        key_id: &'static str,
        run_id: &'static str,
        profile_id: &'static str,
        role: PeerRole,
    ) -> Self {
        Self {
            public_key,
            key_id,
            run_id,
            profile_id,
            role,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PairAuthError {
    InvalidArgument,
    ManifestRejected,
    PeerRejected,
    BridgeContract,
}
impl fmt::Display for PairAuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidArgument => "invalid pair authentication argument",
            Self::ManifestRejected => "signed pair manifest rejected",
            Self::PeerRejected => "peer code identity or hardening rejected",
            Self::BridgeContract => "native pair bridge contract failed",
        })
    }
}
impl std::error::Error for PairAuthError {}

fn result(code: i32) -> Result<(), PairAuthError> {
    match code {
        0 => Ok(()),
        1 => Err(PairAuthError::InvalidArgument),
        2 => Err(PairAuthError::ManifestRejected),
        3 => Err(PairAuthError::PeerRejected),
        _ => Err(PairAuthError::BridgeContract),
    }
}

#[repr(C)]
#[derive(Default)]
struct NativeVerifiedPeer {
    audit_token: [u8; 32],
    uid: u32,
    role: u32,
}

unsafe extern "C" {
    fn uipa_manifest_load(
        envelope: *const u8,
        envelope_len: usize,
        public_key: *const u8,
        public_key_len: usize,
        key_id: *const u8,
        key_id_len: usize,
        run_id: *const u8,
        run_id_len: usize,
        profile_id: *const u8,
        profile_id_len: usize,
        local_role: u32,
        output: *mut *mut std::ffi::c_void,
    ) -> i32;
    fn uipa_manifest_load_v2(
        envelope: *const u8,
        envelope_len: usize,
        public_key: *const u8,
        public_key_len: usize,
        key_id: *const u8,
        key_id_len: usize,
        product_id: *const u8,
        product_id_len: usize,
        release_id: *const u8,
        release_id_len: usize,
        protocol_major: u32,
        local_role: u32,
        output: *mut *mut std::ffi::c_void,
    ) -> i32;
    fn uipa_authenticate(
        manifest: *mut std::ffi::c_void,
        fd: i32,
        expected_role: u32,
        output: *mut NativeVerifiedPeer,
    ) -> i32;
    fn uipa_authenticate_v2(
        manifest: *mut std::ffi::c_void,
        fd: i32,
        expected_role: u32,
        expected_path: *const u8,
        expected_path_len: usize,
        output: *mut NativeVerifiedPeer,
    ) -> i32;
    fn uipa_manifest_free(manifest: *mut std::ffi::c_void);
}

/// 每个后台认证队列独立加载；不跨线程共享 Swift handle，不允许释放与认证并发。
pub struct PairManifest {
    handle: NonNull<std::ffi::c_void>,
    local_role: PeerRole,
    release_binding: Option<ReleasePairBinding>,
    _thread_bound: PhantomData<Rc<()>>,
}

impl PairManifest {
    pub fn load(envelope: &[u8], trust: &EmbeddedPairTrust) -> Result<Self, PairAuthError> {
        if envelope.is_empty() || envelope.len() > 16_384 {
            return Err(PairAuthError::InvalidArgument);
        }
        let mut handle = std::ptr::null_mut();
        // SAFETY: 全部切片在同步调用期间有效；output 指向唯一有效槽位。
        let code = unsafe {
            uipa_manifest_load(
                envelope.as_ptr(),
                envelope.len(),
                trust.public_key.as_ptr(),
                trust.public_key.len(),
                trust.key_id.as_ptr(),
                trust.key_id.len(),
                trust.run_id.as_ptr(),
                trust.run_id.len(),
                trust.profile_id.as_ptr(),
                trust.profile_id.len(),
                trust.role as u32,
                &mut handle,
            )
        };
        if let Err(error) = result(code) {
            if !handle.is_null() {
                // SAFETY: 若桥异常返回了 handle，仍按同一 ABI 释放，避免泄漏。
                unsafe { uipa_manifest_free(handle) };
            }
            return Err(error);
        }
        Ok(Self {
            handle: NonNull::new(handle).ok_or(PairAuthError::BridgeContract)?,
            local_role: trust.role,
            release_binding: None,
            _thread_bound: PhantomData,
        })
    }

    /// 独立 v2 入口。任何失败立即关闭；不把相同字节送入旧版解析器。
    pub fn load_release(
        envelope: &[u8],
        trust: &EmbeddedReleasePairTrust,
    ) -> Result<Self, PairAuthError> {
        if envelope.is_empty() || envelope.len() > 16_384 {
            return Err(PairAuthError::InvalidArgument);
        }
        let mut handle = std::ptr::null_mut();
        let binding = trust.binding;
        // SAFETY: 同步调用期间所有借用切片有效；output 为唯一初始化槽。
        let code = unsafe {
            uipa_manifest_load_v2(
                envelope.as_ptr(),
                envelope.len(),
                trust.public_key.as_ptr(),
                trust.public_key.len(),
                trust.key_id.as_ptr(),
                trust.key_id.len(),
                binding.product_id.as_ptr(),
                binding.product_id.len(),
                binding.release_id.as_ptr(),
                binding.release_id.len(),
                u32::from(binding.protocol_major),
                trust.role as u32,
                &mut handle,
            )
        };
        if let Err(error) = result(code) {
            if !handle.is_null() {
                // SAFETY: 清理桥在失败路径可能返回的唯一句柄，禁止泄漏。
                unsafe { uipa_manifest_free(handle) };
            }
            return Err(error);
        }
        Ok(Self {
            handle: NonNull::new(handle).ok_or(PairAuthError::BridgeContract)?,
            local_role: trust.role,
            release_binding: Some(binding),
            _thread_bound: PhantomData,
        })
    }

    /// 只有成功验签后返回已嵌入产品/发布绑定；不代表运行时 profile 已认证。
    pub fn release_binding(&self) -> Option<ReleasePairBinding> {
        self.release_binding
    }

    pub fn authenticate(
        &self,
        fd: BorrowedFd<'_>,
        expected_role: PeerRole,
    ) -> Result<VerifiedPeer, PairAuthError> {
        if expected_role == self.local_role || self.release_binding.is_some() {
            return Err(PairAuthError::InvalidArgument);
        }
        let mut output = NativeVerifiedPeer::default();
        // SAFETY: self 持有仍存活且不可并发释放的 handle；fd 借用覆盖本次同步调用。
        result(unsafe {
            uipa_authenticate(
                self.handle.as_ptr(),
                fd.as_raw_fd(),
                expected_role as u32,
                &mut output,
            )
        })?;
        if output.role != expected_role as u32 {
            return Err(PairAuthError::BridgeContract);
        }
        Ok(VerifiedPeer {
            audit_token: output.audit_token,
            uid: output.uid,
            role: expected_role,
        })
    }
    /// v2 在动态代码身份认证后，核验进程 bundle 位于已验证收据指定的位置。
    pub fn authenticate_at(
        &self,
        fd: BorrowedFd<'_>,
        expected_role: PeerRole,
        expected_bundle_path: &Path,
    ) -> Result<VerifiedPeer, PairAuthError> {
        let path = expected_bundle_path.as_os_str().as_bytes();
        if self.release_binding.is_none()
            || expected_role == self.local_role
            || !expected_bundle_path.is_absolute()
            || path.len() > 4095
            || path.contains(&0)
            || std::str::from_utf8(path).is_err()
        {
            return Err(PairAuthError::InvalidArgument);
        }
        let mut output = NativeVerifiedPeer::default();
        // SAFETY: 句柄由 self 独占，fd 与路径切片在同步 FFI 期间有效。
        result(unsafe {
            uipa_authenticate_v2(
                self.handle.as_ptr(),
                fd.as_raw_fd(),
                expected_role as u32,
                path.as_ptr(),
                path.len(),
                &mut output,
            )
        })?;
        if output.role != expected_role as u32 {
            return Err(PairAuthError::BridgeContract);
        }
        Ok(VerifiedPeer {
            audit_token: output.audit_token,
            uid: output.uid,
            role: expected_role,
        })
    }
}

impl Drop for PairManifest {
    fn drop(&mut self) {
        // SAFETY: 唯一 RAII owner；没有复制或对外暴露原始指针。
        unsafe { uipa_manifest_free(self.handle.as_ptr()) };
    }
}

/// 不能通过握手反序列化/公开字段构造；只在真实签名及硬化认证成功后创建。
pub struct VerifiedPeer {
    audit_token: [u8; 32],
    uid: u32,
    role: PeerRole,
}
impl VerifiedPeer {
    pub fn audit_token(&self) -> &[u8; 32] {
        &self.audit_token
    }
    pub fn uid(&self) -> u32 {
        self.uid
    }
    pub fn role(&self) -> PeerRole {
        self.role
    }
}

#[cfg(test)]
mod build_trust_tests {
    use super::*;

    #[test]
    fn unconfigured_binary_never_uses_runtime_profile_as_a_trust_root() {
        #[cfg(not(unified_paired_build))]
        for profile in ["handy-local", "unified-candidate:trial-20260905", ""] {
            assert!(candidate_build_trust(profile).unwrap().is_none());
        }
        #[cfg(unified_paired_build)]
        {
            if let Some((run, profile)) = embedded::LEGACY_PROFILE {
                let trust = candidate_build_trust(profile).unwrap().unwrap();
                assert_eq!(trust.public_key, &embedded::PUBLIC_KEY);
                assert_eq!(trust.run_id, run);
                assert!(release_build_trust().unwrap().is_none());
            } else {
                assert!(release_build_trust().unwrap().is_some());
            }
            for profile in ["handy-local", "unified-candidate:wrong-run", ""] {
                assert!(matches!(
                    candidate_build_trust(profile),
                    Err(PairAuthError::InvalidArgument)
                ));
            }
        }
    }
}
