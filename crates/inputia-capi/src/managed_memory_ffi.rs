//! 所有Rime及会话访问在宿主主线程完成；网络线程只传递有界JSON值。
use super::*;
use inputia_core::memory_snapshot::MemoryQuery;
use managed_memory::{InstallSnapshot, PolicyIdentity};
use serde::de::DeserializeOwned;

unsafe fn request<T: DeserializeOwned>(raw: *const c_char) -> Result<T, &'static str> {
    if raw.is_null() {
        return Err("memory_request_invalid");
    }
    const LIMIT: usize = 192 * 1024;
    let mut length = 0;
    // C ABI要求指针直到首个NUL均可读；扫描也受同一预算约束。
    while length <= LIMIT && unsafe { *raw.add(length) } != 0 {
        length += 1;
    }
    if length == 0 || length > LIMIT {
        return Err("memory_request_invalid");
    }
    let raw = unsafe { std::slice::from_raw_parts(raw.cast::<u8>(), length) };
    let value = inputia_settings::store::strict_json(raw).map_err(|_| "memory_request_invalid")?;
    serde_json::from_value(value).map_err(|_| "memory_request_invalid")
}
fn failure(code: &'static str) -> *mut c_char {
    string_json(&serde_json::json!({"ok":false,"code":code}))
}
fn permitted(session: &InputiaSession) -> bool {
    cfg!(feature = "managed-memory")
        && session.context_verified.load(Ordering::Relaxed)
        && !session.policy.excludes(&session.context)
}

/// 应用已认证的隐私屏障；即使epoch不变也清除进程中全部查询正文和待处理票据。
/// 成功只证明本模块缓存已清，宿主须另行清面板/补全/提交缓存后才能发送完整ACK。
/// # Safety
/// JSON为有效NUL结尾字符串。必须在Rime唯一所有者线程调用，且宿主已验证服务身份。
#[no_mangle]
pub unsafe extern "C" fn inputia_memory_apply_policy(raw: *const c_char) -> *mut c_char {
    if !cfg!(feature = "managed-memory") {
        return failure("memory_managed_unavailable");
    }
    let identity: PolicyIdentity = match unsafe { request(raw) } {
        Ok(value) => value,
        Err(code) => return failure(code),
    };
    match managed_memory::apply_process_policy(identity) {
        Ok(generation) => string_json(&serde_json::json!({"ok":true,"generation":generation})),
        Err(code) => failure(code),
    }
}

/// 为当前输入状态登记一次异步查询，租约从本次调用起计。
/// # Safety
/// session必须存活、独占且在Rime唯一所有者线程；raw遵循C字符串合同。
#[no_mangle]
pub unsafe extern "C" fn inputia_session_memory_begin(
    session: *mut InputiaSession,
    raw: *const c_char,
) -> *mut c_char {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Begin {
        query: MemoryQuery,
        composing: String,
    }
    if session.is_null() {
        return failure("memory_session_invalid");
    }
    let session = unsafe { &mut *session };
    if !permitted(session) {
        return failure("memory_context_unverified");
    }
    let value: Begin = match unsafe { request(raw) } {
        Ok(value) => value,
        Err(code) => return failure(code),
    };
    refresh_revoked_memory(session);
    if value.composing != session.core.snapshot().composing {
        return failure("memory_query_retired");
    }
    if let MemoryQuery::Rank { candidate_texts } = &value.query {
        if candidate_texts.len() > 64
            || candidate_texts.iter().map(String::len).sum::<usize>() > 64 * 1024
        {
            return failure("memory_query_invalid");
        }
        let pool = session.core.candidate_pool(64);
        if candidate_texts
            .iter()
            .any(|text| !pool.iter().any(|(candidate, _)| &candidate.text == text))
        {
            return failure("memory_query_invalid");
        }
    }
    match session
        .managed_memory
        .lock()
        .map_err(|_| "memory_session_unavailable")
        .and_then(|mut memory| memory.begin(value.query, value.composing))
    {
        Ok(ticket) => string_json(&serde_json::json!({"ok":true,"query":ticket})),
        Err(code) => failure(code),
    }
}

/// 安装经认证且完整绑定请求的服务回复，过期/改过输入状态的回复拒绝。
/// # Safety
/// session与raw遵循唯一所有者及C字符串合同；调用方已验证wire回复身份、摘要及target。
#[no_mangle]
pub unsafe extern "C" fn inputia_session_memory_install(
    session: *mut InputiaSession,
    raw: *const c_char,
) -> *mut c_char {
    if session.is_null() {
        return failure("memory_session_invalid");
    }
    let session = unsafe { &mut *session };
    if !permitted(session) {
        return failure("memory_context_unverified");
    }
    let value: InstallSnapshot = match unsafe { request(raw) } {
        Ok(value) => value,
        Err(code) => return failure(code),
    };
    if value.composing != session.core.snapshot().composing {
        return failure("memory_query_retired");
    }
    let rank = matches!(value.query, MemoryQuery::Rank { .. });
    let result = session
        .managed_memory
        .lock()
        .map_err(|_| "memory_session_unavailable")
        .and_then(|mut memory| memory.install(value));
    if let Err(code) = result {
        return failure(code);
    }
    refresh_revoked_memory(session);
    if rank {
        session.core.refresh_candidate_snapshot();
    }
    outcome_json(OutputEnvelope::ok(None, false, session.core.snapshot()))
}

/// 撤销当前session的查询票据及正文缓存，其他session由进程屏障统一处理。
/// # Safety
/// session必须存活且由Rime所有者线程独占调用。
#[no_mangle]
pub unsafe extern "C" fn inputia_session_memory_clear(session: *mut InputiaSession) -> *mut c_char {
    if session.is_null() {
        return failure("memory_session_invalid");
    }
    let session = unsafe { &mut *session };
    let Ok(mut memory) = session.managed_memory.lock() else {
        return failure("memory_session_unavailable");
    };
    memory.clear();
    drop(memory);
    refresh_revoked_memory(session);
    outcome_json(OutputEnvelope::ok(None, false, session.core.snapshot()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_ffi_rejects_unknown_duplicate_and_null_requests() {
        for raw in [
            r#"{"server_instance":"s","profile_id":"p","policy_epoch":1,"verified":true}"#,
            r#"{"server_instance":"s","profile_id":"p","policy_epoch":1,"policy_epoch":2}"#,
        ] {
            let raw = CString::new(raw).unwrap();
            assert!(unsafe { request::<PolicyIdentity>(raw.as_ptr()) }.is_err());
        }
        let oversized = CString::new("x".repeat(192 * 1024 + 1)).unwrap();
        assert!(unsafe { request::<PolicyIdentity>(oversized.as_ptr()) }.is_err());
        assert!(unsafe { request::<PolicyIdentity>(std::ptr::null()) }.is_err());
        for ptr in [
            unsafe { inputia_session_memory_begin(null_mut(), std::ptr::null()) },
            unsafe { inputia_session_memory_install(null_mut(), std::ptr::null()) },
            unsafe { inputia_session_memory_clear(null_mut()) },
        ] {
            assert!(!ptr.is_null());
            let json: serde_json::Value =
                serde_json::from_str(unsafe { CStr::from_ptr(ptr) }.to_str().unwrap()).unwrap();
            assert_eq!(json["ok"], false);
            unsafe { inputia_string_free(ptr) };
        }
    }
}
