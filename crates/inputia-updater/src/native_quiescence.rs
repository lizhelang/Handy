//! 存活期间暂停已知写者的 RAII 租约；不证明进程退出或崩溃安全恢复。
use crate::{
    native_code::{
        CodeExpectation, CodePurpose, CodeRole, NativeCodeVerifier, VerifiedCodeEvidence,
    },
    MaintenanceMarker, Subject,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriterProcessIdentity {
    pub pid: i32,
    pub uid: u32,
    pub start_seconds: u64,
    pub start_microseconds: u64,
    pub pid_version: u32,
    pub role: String,
    pub bundle_id: String,
    pub release_id: String,
    pub executable_path: String,
    pub cdhash: String,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdditionalWriterGate {
    InputSourceDeselected,
    LegacyPathIsolated,
    ExclusiveServiceLease,
    CrashRecoveryGuardian,
}
/// 不能 Clone/Deserialize，也不能转换为完整 QuiescenceReceipt。
/// 仅本对象存活且 assert_suspended 成功时证明已核实例处于暂停状态。
/// 正常结束必须调用 resume；Drop 只是兜底，崩溃恢复需尚未接入的 guardian。
pub struct SuspendedWriterLease {
    subject: Subject,
    epoch: String,
    roles: Vec<VerifiedCodeEvidence>,
    suspended: Vec<WriterProcessIdentity>,
    evidence_sha256: String,
    handle: NativeSuspensionHandle,
    resumed: bool,
}
impl SuspendedWriterLease {
    pub fn subject(&self) -> &Subject {
        &self.subject
    }
    pub fn epoch(&self) -> &str {
        &self.epoch
    }
    pub fn suspended(&self) -> &[WriterProcessIdentity] {
        &self.suspended
    }
    pub fn evidence_sha256(&self) -> &str {
        &self.evidence_sha256
    }
    pub fn additional_gates(&self) -> [AdditionalWriterGate; 4] {
        [
            AdditionalWriterGate::InputSourceDeselected,
            AdditionalWriterGate::LegacyPathIsolated,
            AdditionalWriterGate::ExclusiveServiceLease,
            AdditionalWriterGate::CrashRecoveryGuardian,
        ]
    }
    /// 每次关键操作前重核实际维护授权、代码、进程集合、暂停状态和子树。
    pub fn assert_suspended(&self, subject: &Subject, marker: &MaintenanceMarker) -> Result<()> {
        if self.resumed {
            return Err(NativeQuiescenceError::AlreadyResumed);
        }
        if &self.subject != subject || self.epoch != marker.epoch {
            return Err(NativeQuiescenceError::MarkerMismatch);
        }
        let expected: Vec<_> = self.roles.iter().map(|v| v.expectation().clone()).collect();
        let request = validate_request(subject, marker, &expected, "suspend")?;
        let uid = actual_marker(subject, marker)?;
        for role in &self.roles {
            NativeCodeVerifier
                .verify(role.expectation(), role.tree())
                .map_err(NativeQuiescenceError::Code)?;
        }
        let evidence = checked_reply(
            &native_assert(&self.handle, subject, marker)?,
            &request,
            uid,
        )?;
        if evidence.suspended != self.suspended {
            return Err(NativeQuiescenceError::InvalidReply);
        }
        actual_marker(subject, marker)?;
        Ok(())
    }
    /// 幂等恢复，仅对本租约确实从运行态暂停的原 audit 实例发 CONT。
    /// 失败保留 handle，可以重试；维护 marker 已撤销也必须能够恢复。
    pub fn resume(&mut self) -> Result<()> {
        if self.resumed {
            return Ok(());
        }
        native_resume(&self.handle)?;
        self.resumed = true;
        Ok(())
    }
}
#[derive(Debug)]
pub enum NativeQuiescenceError {
    InvalidCoverage,
    MarkerMismatch,
    NativeUnavailable,
    InvalidReply,
    AlreadyResumed,
    Rejected { code: String, os_status: i32 },
    Code(crate::native_code::NativeCodeError),
    Maintenance(inputia_settings::maintenance::MaintenanceError),
    Json(serde_json::Error),
}
impl std::fmt::Display for NativeQuiescenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for NativeQuiescenceError {}
type Result<T> = std::result::Result<T, NativeQuiescenceError>;
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    schema_version: u32,
    action: String,
    subject: Subject,
    epoch: String,
    old_release_id: String,
    roles: Vec<CodeExpectation>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Evidence {
    request: Request,
    suspended: Vec<WriterProcessIdentity>,
    enumerated_roles: Vec<String>,
    user_id: u32,
    rescanned_suspended: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Reply {
    ok: bool,
    evidence: Option<Evidence>,
    code: Option<String>,
    os_status: Option<i32>,
}
fn validate_request(
    subject: &Subject,
    marker: &MaintenanceMarker,
    roles: &[CodeExpectation],
    action: &str,
) -> Result<Request> {
    use inputia_settings::installation::valid_uuid;
    if marker.schema_version != 1
        || marker.transaction_id != subject.transaction_id
        || marker.installation_id != subject.installation_id
        || marker.plan_sha256 != subject.plan_sha256
        || marker.new_release_id != subject.new_release_id
        || !valid_uuid(&marker.epoch)
        || action != "suspend"
        || roles.len() != 3
    {
        return Err(NativeQuiescenceError::InvalidCoverage);
    }
    let previous = marker
        .old_release_id
        .as_ref()
        .ok_or(NativeQuiescenceError::InvalidCoverage)?;
    let expected = [CodeRole::Control, CodeRole::Ime, CodeRole::Settings];
    let mut roots = std::collections::BTreeSet::new();
    let mut ids = std::collections::BTreeSet::new();
    for (role, kind) in roles.iter().zip(expected) {
        role.validate().map_err(NativeQuiescenceError::Code)?;
        if role.subject != *subject
            || role.purpose != CodePurpose::PreviousRelease
            || role.release_id != *previous
            || role.role != kind
            || !roots.insert(&role.exact_bundle_path)
            || !ids.insert(&role.bundle_id)
        {
            return Err(NativeQuiescenceError::InvalidCoverage);
        }
    }
    Ok(Request {
        schema_version: 1,
        action: action.into(),
        subject: subject.clone(),
        epoch: marker.epoch.clone(),
        old_release_id: previous.clone(),
        roles: roles.to_vec(),
    })
}
fn actual_marker(subject: &Subject, marker: &MaintenanceMarker) -> Result<u32> {
    let context = inputia_settings::maintenance::current_user_context()
        .map_err(NativeQuiescenceError::Maintenance)?;
    let observed = inputia_settings::maintenance::inspect(&context.home, context.uid)
        .map_err(NativeQuiescenceError::Maintenance)?;
    if observed.as_ref() != Some(marker)
        || marker.transaction_id != subject.transaction_id
        || marker.installation_id != subject.installation_id
        || marker.plan_sha256 != subject.plan_sha256
        || marker.new_release_id != subject.new_release_id
    {
        return Err(NativeQuiescenceError::MarkerMismatch);
    }
    Ok(context.uid)
}
fn canonical(v: &impl Serialize) -> Result<Vec<u8>> {
    fn sorted(v: serde_json::Value) -> serde_json::Value {
        match v {
            serde_json::Value::Object(v) => serde_json::Value::Object(
                v.into_iter()
                    .map(|(k, v)| (k, sorted(v)))
                    .collect::<std::collections::BTreeMap<_, _>>()
                    .into_iter()
                    .collect(),
            ),
            serde_json::Value::Array(v) => {
                serde_json::Value::Array(v.into_iter().map(sorted).collect())
            }
            v => v,
        }
    }
    serde_json::to_vec(&sorted(
        serde_json::to_value(v).map_err(NativeQuiescenceError::Json)?,
    ))
    .map_err(NativeQuiescenceError::Json)
}
fn checked_reply(raw: &[u8], request: &Request, uid: u32) -> Result<Evidence> {
    if raw.len() > 131_072 {
        return Err(NativeQuiescenceError::InvalidReply);
    }
    let reply: Reply = serde_json::from_slice(raw).map_err(NativeQuiescenceError::Json)?;
    if !reply.ok {
        let code = reply
            .code
            .filter(|s| {
                !s.is_empty()
                    && s.len() <= 64
                    && s.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
            })
            .ok_or(NativeQuiescenceError::InvalidReply)?;
        if reply.evidence.is_some() {
            return Err(NativeQuiescenceError::InvalidReply);
        }
        return Err(NativeQuiescenceError::Rejected {
            code,
            os_status: reply.os_status.unwrap_or(0),
        });
    }
    let evidence = reply.evidence.ok_or(NativeQuiescenceError::InvalidReply)?;
    if reply.code.is_some()
        || reply.os_status.is_some()
        || evidence.request != *request
        || evidence.user_id != uid
        || !evidence.rescanned_suspended
        || evidence.enumerated_roles != ["control", "ime", "settings"]
        || evidence.suspended.len() > 64
        || !evidence.suspended.windows(2).all(|p| p[0].pid < p[1].pid)
    {
        return Err(NativeQuiescenceError::InvalidReply);
    }
    for process in &evidence.suspended {
        let role = request
            .roles
            .iter()
            .find(|r| match r.role {
                CodeRole::Control => process.role == "control",
                CodeRole::Ime => process.role == "ime",
                CodeRole::Settings => process.role == "settings",
                _ => false,
            })
            .ok_or(NativeQuiescenceError::InvalidReply)?;
        let prefix = role.exact_bundle_path.join("Contents/MacOS");
        let executable = std::path::Path::new(&process.executable_path);
        if process.pid <= 1
            || process.uid != uid
            || process.start_seconds == 0
            || process.start_microseconds >= 1_000_000
            || process.pid_version == 0
            || process.bundle_id != role.bundle_id
            || process.release_id != role.release_id
            || !role.cdhashes.contains(&process.cdhash)
            || executable.parent() != Some(prefix.as_path())
            || executable.file_name().is_none()
        {
            return Err(NativeQuiescenceError::InvalidReply);
        }
    }
    Ok(evidence)
}
pub struct NativeWriterSuspender;
impl NativeWriterSuspender {
    /// 当前没有生产调用；安装接线前还必须具备独立的崩溃恢复 guardian。
    /// 期望三角色必须来自真实 Security 制品证据；没有任意 PID / 布尔授权入口。
    pub fn suspend(
        &self,
        subject: &Subject,
        marker: &MaintenanceMarker,
        roles: &[VerifiedCodeEvidence],
    ) -> Result<SuspendedWriterLease> {
        let expected: Vec<_> = roles.iter().map(|v| v.expectation().clone()).collect();
        let request = validate_request(subject, marker, &expected, "suspend")?;
        let uid = actual_marker(subject, marker)?;
        for role in roles {
            NativeCodeVerifier
                .verify(role.expectation(), role.tree())
                .map_err(NativeQuiescenceError::Code)?;
        }
        // 先接管原生 handle；此后解析、marker 或核验失败都由其 Drop 恢复。
        let (raw, handle) = native_suspend(&canonical(&request)?, subject, marker)?;
        let evidence = checked_reply(&raw, &request, uid)?;
        actual_marker(subject, marker)?;
        let evidence_sha256 = format!("{:x}", Sha256::digest(canonical(&evidence)?));
        let lease = SuspendedWriterLease {
            subject: subject.clone(),
            epoch: marker.epoch.clone(),
            roles: roles.to_vec(),
            suspended: evidence.suspended,
            evidence_sha256,
            handle: handle.ok_or(NativeQuiescenceError::InvalidReply)?,
            resumed: false,
        };
        lease.assert_suspended(subject, marker)?;
        Ok(lease)
    }
}
// 裸指针使租约非 Send/Sync；所有原生调用在拥有者线程串行，不公开复制 handle。
struct NativeSuspensionHandle(std::ptr::NonNull<std::ffi::c_void>);
impl Drop for NativeSuspensionHandle {
    fn drop(&mut self) {
        #[cfg(all(target_os = "macos", feature = "native-code-verification"))]
        unsafe {
            iuis_writer_suspension_free(self.0.as_ptr())
        };
    }
}
#[cfg(all(target_os = "macos", feature = "native-code-verification"))]
unsafe extern "C" {
    fn iuis_writer_suspend(
        bytes: *const u8,
        length: usize,
        check: extern "C" fn(*mut std::ffi::c_void) -> i32,
        context: *mut std::ffi::c_void,
        handle: *mut *mut std::ffi::c_void,
    ) -> *mut std::ffi::c_char;
    fn iuis_writer_assert_suspended(
        handle: *mut std::ffi::c_void,
        check: extern "C" fn(*mut std::ffi::c_void) -> i32,
        context: *mut std::ffi::c_void,
    ) -> *mut std::ffi::c_char;
    fn iuis_writer_resume(handle: *mut std::ffi::c_void) -> *mut std::ffi::c_char;
    fn iuis_writer_suspension_free(handle: *mut std::ffi::c_void);
    fn iuis_string_free(string: *mut std::ffi::c_char);
}
#[cfg(all(target_os = "macos", feature = "native-code-verification"))]
struct EffectAuthority<'a> {
    subject: &'a Subject,
    marker: &'a MaintenanceMarker,
}
#[cfg(all(target_os = "macos", feature = "native-code-verification"))]
extern "C" fn authorize_effect(context: *mut std::ffi::c_void) -> i32 {
    if context.is_null() {
        return -1;
    }
    // 同步FFI栈指针不被原生保存；每个真实 STOP 前读取安全维护文件。
    let authority = unsafe { &*context.cast::<EffectAuthority<'_>>() };
    if actual_marker(authority.subject, authority.marker).is_ok() {
        0
    } else {
        -1
    }
}
#[cfg(all(target_os = "macos", feature = "native-code-verification"))]
fn consume_native_string(ptr: *mut std::ffi::c_char) -> Result<Vec<u8>> {
    if ptr.is_null() {
        return Err(NativeQuiescenceError::NativeUnavailable);
    }
    let count = unsafe { libc::strnlen(ptr, 131_073) };
    let result = if count > 131_072 {
        Err(NativeQuiescenceError::InvalidReply)
    } else {
        Ok(unsafe { std::slice::from_raw_parts(ptr.cast::<u8>(), count) }.to_vec())
    };
    unsafe { iuis_string_free(ptr) };
    result
}
fn native_suspend(
    bytes: &[u8],
    subject: &Subject,
    marker: &MaintenanceMarker,
) -> Result<(Vec<u8>, Option<NativeSuspensionHandle>)> {
    if bytes.is_empty() || bytes.len() > 131_072 {
        return Err(NativeQuiescenceError::InvalidCoverage);
    }
    #[cfg(not(all(target_os = "macos", feature = "native-code-verification")))]
    {
        let _ = (subject, marker);
        Err(NativeQuiescenceError::NativeUnavailable)
    }
    #[cfg(all(target_os = "macos", feature = "native-code-verification"))]
    {
        let mut authority = EffectAuthority { subject, marker };
        let mut ptr = std::ptr::null_mut();
        let raw = unsafe {
            iuis_writer_suspend(
                bytes.as_ptr(),
                bytes.len(),
                authorize_effect,
                (&mut authority as *mut EffectAuthority<'_>).cast(),
                &mut ptr,
            )
        };
        let handle = std::ptr::NonNull::new(ptr).map(NativeSuspensionHandle);
        let bytes = consume_native_string(raw)?;
        Ok((bytes, handle))
    }
}
fn native_assert(
    handle: &NativeSuspensionHandle,
    subject: &Subject,
    marker: &MaintenanceMarker,
) -> Result<Vec<u8>> {
    #[cfg(not(all(target_os = "macos", feature = "native-code-verification")))]
    {
        let _ = (handle.0, subject, marker);
        Err(NativeQuiescenceError::NativeUnavailable)
    }
    #[cfg(all(target_os = "macos", feature = "native-code-verification"))]
    {
        let mut authority = EffectAuthority { subject, marker };
        consume_native_string(unsafe {
            iuis_writer_assert_suspended(
                handle.0.as_ptr(),
                authorize_effect,
                (&mut authority as *mut EffectAuthority<'_>).cast(),
            )
        })
    }
}
fn native_resume(handle: &NativeSuspensionHandle) -> Result<()> {
    #[cfg(not(all(target_os = "macos", feature = "native-code-verification")))]
    {
        let _ = handle;
        Err(NativeQuiescenceError::NativeUnavailable)
    }
    #[cfg(all(target_os = "macos", feature = "native-code-verification"))]
    {
        check_resume_reply(&consume_native_string(unsafe {
            iuis_writer_resume(handle.0.as_ptr())
        })?)
    }
}
#[cfg(any(test, all(target_os = "macos", feature = "native-code-verification")))]
fn check_resume_reply(raw: &[u8]) -> Result<()> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ResumeReply {
        ok: bool,
        code: Option<String>,
        os_status: Option<i32>,
    }
    let reply: ResumeReply = serde_json::from_slice(raw).map_err(NativeQuiescenceError::Json)?;
    if reply.ok && reply.code.is_none() && reply.os_status.is_none() {
        return Ok(());
    }
    if !reply.ok && reply.code.as_deref() == Some("writer_resume_failed") {
        return Err(NativeQuiescenceError::Rejected {
            code: "writer_resume_failed".into(),
            os_status: reply.os_status.unwrap_or(0),
        });
    }
    Err(NativeQuiescenceError::InvalidReply)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Subject, MaintenanceMarker, Vec<CodeExpectation>) {
        let subject = Subject {
            transaction_id: "11111111-1111-4111-8111-111111111111".into(),
            plan_sha256: "a".repeat(64),
            installation_id: "22222222-2222-4222-8222-222222222222".into(),
            new_release_id: "inputia-new".into(),
        };
        let marker = MaintenanceMarker {
            schema_version: 1,
            transaction_id: subject.transaction_id.clone(),
            installation_id: subject.installation_id.clone(),
            old_release_id: Some("inputia-old".into()),
            new_release_id: subject.new_release_id.clone(),
            epoch: "33333333-3333-4333-8333-333333333333".into(),
            plan_sha256: subject.plan_sha256.clone(),
        };
        let roles = [
            (CodeRole::Control, "control"),
            (CodeRole::Ime, "ime"),
            (CodeRole::Settings, "settings"),
        ]
        .into_iter()
        .map(|(role, name)| CodeExpectation {
            schema_version: 1,
            subject: subject.clone(),
            purpose: CodePurpose::PreviousRelease,
            product_id: "com.inputia".into(),
            role,
            exact_bundle_path: format!("/synthetic/{name}.app").into(),
            bundle_id: format!("com.inputia.{name}"),
            release_id: "inputia-old".into(),
            version: "1.0.0".into(),
            build: 1,
            source_commit: "b".repeat(40),
            team_id: "TESTTEAM01".into(),
            architectures: vec!["arm64".into()],
            cdhashes: vec!["c".repeat(40)],
        })
        .collect();
        (subject, marker, roles)
    }
    fn evidence(request: Request) -> Evidence {
        Evidence {
            request,
            suspended: vec![WriterProcessIdentity {
                pid: 101,
                uid: 501,
                start_seconds: 100,
                start_microseconds: 12,
                pid_version: 8,
                role: "control".into(),
                bundle_id: "com.inputia.control".into(),
                release_id: "inputia-old".into(),
                executable_path: "/synthetic/control.app/Contents/MacOS/control".into(),
                cdhash: "c".repeat(40),
            }],
            enumerated_roles: vec!["control".into(), "ime".into(), "settings".into()],
            user_id: 501,
            rescanned_suspended: true,
        }
    }
    fn encoded(evidence: Evidence) -> Vec<u8> {
        serde_json::to_vec(
            &serde_json::json!({"ok":true,"evidence":evidence,"code":null,"os_status":null}),
        )
        .unwrap()
    }
    #[test]
    fn every_old_writer_role_must_be_bound_to_the_subject_and_marker() {
        let (subject, marker, roles) = fixture();
        assert!(validate_request(&subject, &marker, &roles, "suspend").is_ok());
        for count in 0..3 {
            assert!(validate_request(&subject, &marker, &roles[..count], "suspend").is_err());
        }
        let mut missing = marker.clone();
        missing.old_release_id = None;
        assert!(validate_request(&subject, &missing, &roles, "suspend").is_err());
        let mut wrong = roles.clone();
        wrong[2].exact_bundle_path = wrong[1].exact_bundle_path.clone();
        assert!(validate_request(&subject, &marker, &wrong, "suspend").is_err());
        let mut wrong = roles.clone();
        wrong[1].release_id = "inputia-other-old".into();
        assert!(validate_request(&subject, &marker, &wrong, "suspend").is_err());
        let mut wrong = roles.clone();
        wrong[0].purpose = CodePurpose::NewRelease;
        wrong[0].release_id = subject.new_release_id.clone();
        assert!(validate_request(&subject, &marker, &wrong, "suspend").is_err());
        let mut wrong = marker.clone();
        wrong.epoch = "unbound".into();
        assert!(validate_request(&subject, &wrong, &roles, "suspend").is_err());
    }
    #[test]
    fn returned_process_facts_cannot_replace_identity_or_invent_suspension() {
        let (subject, marker, roles) = fixture();
        let request = validate_request(&subject, &marker, &roles, "suspend").unwrap();
        assert!(checked_reply(&encoded(evidence(request.clone())), &request, 501).is_ok());
        for mode in 0..11 {
            let mut e = evidence(request.clone());
            match mode {
                0 => e.request.epoch = "44444444-4444-4444-8444-444444444444".into(),
                1 => e.rescanned_suspended = false,
                2 => e.enumerated_roles.pop().map(|_| ()).unwrap(),
                3 => e.suspended[0].pid_version = 0,
                4 => e.suspended[0].uid = 0,
                5 => e.suspended[0].start_seconds = 0,
                6 => e.suspended[0].executable_path = "/outside.app/Contents/MacOS/control".into(),
                7 => e.suspended[0].release_id = "inputia-new".into(),
                8 => e.suspended.push(e.suspended[0].clone()),
                9 => e.suspended[0].cdhash = "d".repeat(40),
                _ => e.suspended[0].start_microseconds = 1_000_000,
            };
            assert!(
                checked_reply(&encoded(e), &request, 501).is_err(),
                "mode {mode}"
            );
        }
        assert!(validate_request(&subject, &marker, &roles, "assert_absent").is_err());
        assert!(validate_request(&subject, &marker, &roles, "quiesce").is_err());
    }
    #[test]
    fn resume_errors_remain_retryable_and_cannot_claim_success() {
        assert!(check_resume_reply(br#"{"ok":true,"code":null,"os_status":null}"#).is_ok());
        assert!(matches!(
            check_resume_reply(br#"{"ok":false,"code":"writer_resume_failed","os_status":1}"#),
            Err(NativeQuiescenceError::Rejected { .. })
        ));
        assert!(
            check_resume_reply(br#"{"ok":true,"code":"writer_resume_failed","os_status":1}"#)
                .is_err()
        );
        assert!(check_resume_reply(br#"{"ok":true,"resumed":true}"#).is_err());
    }
    #[test]
    fn native_capability_failure_and_malformed_replies_never_create_a_proof() {
        let (subject, marker, roles) = fixture();
        let request = validate_request(&subject, &marker, &roles, "suspend").unwrap();
        for raw in [
            b"{}".as_slice(),
            b"{\"ok\":true}".as_slice(),
            b"{\"ok\":false,\"evidence\":null,\"code\":\"arbitrary message\",\"os_status\":0}"
                .as_slice(),
        ] {
            assert!(checked_reply(raw, &request, 501).is_err());
        }
        let raw=serde_json::to_vec(&serde_json::json!({"ok":false,"evidence":null,"code":"audit_signal_unavailable","os_status":0})).unwrap();
        assert!(
            matches!(checked_reply(&raw,&request,501),Err(NativeQuiescenceError::Rejected{code,..}) if code=="audit_signal_unavailable")
        );
        assert!(native_suspend(&[], &subject, &marker).is_err());
    }
}
