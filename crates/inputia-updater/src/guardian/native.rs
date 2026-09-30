//! 只有真实角色扫描才能建立 NativePlan；通道传入的PID不会变成效应能力。
use super::{
    protocol::{Entry, Resolution},
    recovery::{Effects, ExecutionState, Peer, PeerState},
    transport::ProcessWatch,
    GuardianError,
};
use crate::{native_code::CodeExpectation, native_quiescence, MaintenanceMarker, Subject};
use serde::{Deserialize, Serialize};
use std::{
    ffi::{c_char, c_void},
    path::PathBuf,
    ptr::NonNull,
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Configuration {
    pub writers: native_quiescence::Request,
    pub updater: CodeExpectation,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Inventory {
    entries: Vec<Entry>,
    executable_path: PathBuf,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeReply<T> {
    ok: bool,
    value: Option<T>,
    code: Option<String>,
    os_status: Option<i32>,
}
fn result<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, GuardianError> {
    let value: NativeReply<T> =
        serde_json::from_slice(bytes).map_err(|_| GuardianError::NativeReply)?;
    if value.ok && value.code.is_none() && value.os_status.is_none() {
        return value.value.ok_or(GuardianError::NativeReply);
    }
    if !value.ok && value.value.is_none() {
        if let Some(code) = value.code.filter(|c| {
            !c.is_empty() && c.len() <= 64 && c.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
        }) {
            return Err(GuardianError::Native {
                code,
                status: value.os_status.unwrap_or(0),
            });
        }
    }
    Err(GuardianError::NativeReply)
}
unsafe extern "C" {
    fn iuis_guardian_prepare(
        bytes: *const u8,
        count: usize,
        output: *mut *mut c_void,
    ) -> *mut c_char;
    fn iuis_guardian_plan_action(
        handle: *mut c_void,
        action: u32,
        index: u32,
        check: Option<extern "C" fn(*mut c_void) -> i32>,
        context: *mut c_void,
    ) -> *mut c_char;
    fn iuis_guardian_plan_free(handle: *mut c_void);
    fn iuis_guardian_peer_open(
        pid: i32,
        bytes: *const u8,
        count: usize,
        output: *mut *mut c_void,
    ) -> *mut c_char;
    fn iuis_guardian_peer_state(handle: *mut c_void) -> *mut c_char;
    fn iuis_guardian_peer_free(handle: *mut c_void);
    fn iuis_string_free(value: *mut c_char);
}
fn consume(pointer: *mut c_char) -> Result<Vec<u8>, GuardianError> {
    if pointer.is_null() {
        return Err(GuardianError::NativeUnavailable);
    }
    let count = unsafe { libc::strnlen(pointer, 131_073) };
    let result = if count > 131_072 {
        Err(GuardianError::NativeReply)
    } else {
        Ok(unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), count) }.to_vec())
    };
    unsafe { iuis_string_free(pointer) };
    result
}
struct PlanHandle(NonNull<c_void>);
impl Drop for PlanHandle {
    fn drop(&mut self) {
        unsafe { iuis_guardian_plan_free(self.0.as_ptr()) }
    }
}
pub(super) struct NativePlan {
    handle: PlanHandle,
    entries: Vec<Entry>,
    pub executable: PathBuf,
}
impl NativePlan {
    pub fn prepare(configuration: &Configuration) -> Result<Self, GuardianError> {
        let raw = native_quiescence::canonical(configuration)?;
        let mut ptr = std::ptr::null_mut();
        let response = unsafe { iuis_guardian_prepare(raw.as_ptr(), raw.len(), &mut ptr) };
        let handle = NonNull::new(ptr).map(PlanHandle);
        let inventory: Inventory = result(&consume(response)?)?;
        Ok(Self {
            handle: handle.ok_or(GuardianError::NativeReply)?,
            entries: inventory.entries,
            executable: inventory.executable_path,
        })
    }
    fn action(&self, action: u32, index: u32) -> Result<String, GuardianError> {
        result(&consume(unsafe {
            iuis_guardian_plan_action(
                self.handle.0.as_ptr(),
                action,
                index,
                None,
                std::ptr::null_mut(),
            )
        })?)
    }
}
struct EffectCheck<'a> {
    action: &'a mut dyn FnMut() -> bool,
}
extern "C" fn authorized(context: *mut c_void) -> i32 {
    if context.is_null() {
        return -1;
    }
    let context = unsafe { &mut *context.cast::<EffectCheck<'_>>() };
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (context.action)()))
        .unwrap_or(false)
    {
        0
    } else {
        -1
    }
}
impl Effects for NativePlan {
    fn entries(&self) -> &[Entry] {
        &self.entries
    }
    fn stop(&mut self, index: u32, check: &mut dyn FnMut() -> bool) -> Result<(), GuardianError> {
        let mut context = EffectCheck { action: check };
        let state: String = result(&consume(unsafe {
            iuis_guardian_plan_action(
                self.handle.0.as_ptr(),
                1,
                index,
                Some(authorized),
                (&mut context as *mut EffectCheck<'_>).cast(),
            )
        })?)?;
        if state != "stopped" {
            return Err(GuardianError::NativeReply);
        }
        Ok(())
    }
    fn close_stop(&mut self) -> Result<(), GuardianError> {
        if self.action(2, 0)? != "recovering" {
            return Err(GuardianError::NativeReply);
        }
        Ok(())
    }
    fn resume(&mut self, index: u32) -> Result<Resolution, GuardianError> {
        match self.action(3, index)?.as_str() {
            "running" => Ok(Resolution::Running),
            "resumed" => Ok(Resolution::Resumed),
            "original_exited" => Ok(Resolution::OriginalExited),
            _ => Err(GuardianError::NativeReply),
        }
    }
    fn state(&self, index: u32) -> Result<ExecutionState, GuardianError> {
        match self.action(4, index)?.as_str() {
            "running" => Ok(ExecutionState::Running),
            "stopped" => Ok(ExecutionState::Stopped),
            "original_exited" => Ok(ExecutionState::OriginalExited),
            _ => Err(GuardianError::NativeReply),
        }
    }
    fn assert_holding(&self) -> Result<(), GuardianError> {
        if self.action(5, 0)? != "holding" {
            return Err(GuardianError::NativeReply);
        }
        Ok(())
    }
}
struct PeerHandle(NonNull<c_void>);
impl Drop for PeerHandle {
    fn drop(&mut self) {
        unsafe { iuis_guardian_peer_free(self.0.as_ptr()) }
    }
}
pub(super) struct NativePeer {
    handle: PeerHandle,
    watch: ProcessWatch,
    forked: std::cell::Cell<bool>,
}
impl NativePeer {
    pub fn open(pid: i32, expected: &CodeExpectation) -> Result<Self, GuardianError> {
        let raw = native_quiescence::canonical(expected)?;
        let mut ptr = std::ptr::null_mut();
        let response = unsafe { iuis_guardian_peer_open(pid, raw.as_ptr(), raw.len(), &mut ptr) };
        let handle = NonNull::new(ptr).map(PeerHandle);
        let identity: crate::native_quiescence::WriterProcessIdentity =
            result(&consume(response)?)?;
        if identity.pid != pid || identity.role != "updater" {
            return Err(GuardianError::NativeReply);
        }
        let value = Self {
            handle: handle.ok_or(GuardianError::NativeReply)?,
            watch: ProcessWatch::new(pid)?,
            forked: std::cell::Cell::new(false),
        };
        if value.state()? != PeerState::Alive {
            return Err(GuardianError::PeerStillAlive);
        } // 注册前后核同一原生audit实例。
        Ok(value)
    }
}
impl Peer for NativePeer {
    fn state(&self) -> Result<PeerState, GuardianError> {
        if self.watch.events()? & libc::NOTE_FORK != 0 {
            self.forked.set(true);
        }
        if self.forked.get() {
            return Ok(PeerState::Unknown);
        }
        match result::<String>(&consume(unsafe {
            iuis_guardian_peer_state(self.handle.0.as_ptr())
        })?)?
        .as_str()
        {
            "alive" => Ok(PeerState::Alive),
            "exited" => Ok(PeerState::Exited),
            "exec" => Ok(PeerState::Exec),
            _ => Err(GuardianError::NativeReply),
        }
    }
}
pub(super) fn marker_valid(subject: &Subject, marker: &MaintenanceMarker) -> bool {
    native_quiescence::actual_marker(subject, marker).is_ok()
}
