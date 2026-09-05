//! macOS 候选配对认证。构建常量建立信任，运行时数据仅提供待验证 manifest。
//! 独立后台认证队列使用 RAII handle；不在按键/GUI 线程执行 Security 查询。

use std::fmt;
use std::marker::PhantomData;
use std::os::fd::{AsRawFd, BorrowedFd};
use std::ptr::NonNull;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum PeerRole {
    Handy = 1,
    Inputia = 2,
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
    fn uipa_authenticate(
        manifest: *mut std::ffi::c_void,
        fd: i32,
        expected_role: u32,
        output: *mut NativeVerifiedPeer,
    ) -> i32;
    fn uipa_manifest_free(manifest: *mut std::ffi::c_void);
}

/// 每个后台认证队列独立加载；不跨线程共享 Swift handle，不允许释放与认证并发。
pub struct PairManifest {
    handle: NonNull<std::ffi::c_void>,
    local_role: PeerRole,
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
            _thread_bound: PhantomData,
        })
    }

    pub fn authenticate(
        &self,
        fd: BorrowedFd<'_>,
        expected_role: PeerRole,
    ) -> Result<VerifiedPeer, PairAuthError> {
        if expected_role == self.local_role {
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
