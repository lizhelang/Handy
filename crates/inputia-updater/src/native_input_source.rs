//! 更新事务内的TIS切离/条件恢复观察。没有TIS原子CAS，不是完整NativeAdapter回执。
use crate::{
    native_code::{CodePurpose, CodeRole, VerifiedCodeEvidence},
    MaintenanceMarker, Subject, Transaction,
};
use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub enum InputSourceError {
    NativeUnavailable,
    InvalidAuthority,
    InvalidReply,
    Rejected(String),
    Transaction(crate::Error),
    Code(crate::native_code::NativeCodeError),
    Maintenance(crate::native_quiescence::NativeQuiescenceError),
}
impl std::fmt::Display for InputSourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for InputSourceError {}
type Result<T> = std::result::Result<T, InputSourceError>;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InputSourceState {
    Prepared,
    AlreadyDetached,
    DetachedObserved,
    PreservedUserSelection,
    RestoredObserved,
    Uncertain,
}
/// 只有当次读回；ownership_exact必须为false，不能序列化成排他授权。
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InputSourceObservation {
    pub state: InputSourceState,
    pub original_source_id: String,
    pub observed_source_id: Option<String>,
    pub fallback_source_id: Option<String>,
    pub selection_attempted: bool,
    pub restoration_attempted: bool,
    pub ownership_exact: bool,
    pub reason: Option<String>,
}
impl InputSourceObservation {
    #[cfg(any(test, all(target_os = "macos", feature = "native-code-verification")))]
    fn validate(&self) -> Result<()> {
        let id = |s: &str| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control);
        if self.ownership_exact
            || !id(&self.original_source_id)
            || self.observed_source_id.as_deref().is_some_and(|s| !id(s))
            || self.fallback_source_id.as_deref().is_some_and(|s| !id(s))
            || self.reason.as_deref().is_some_and(|s| {
                s.is_empty()
                    || s.len() > 64
                    || !s.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
            })
            || (self.restoration_attempted && !self.selection_attempted)
            || (matches!(
                self.state,
                InputSourceState::Prepared | InputSourceState::AlreadyDetached
            ) && self.selection_attempted)
            || (matches!(
                self.state,
                InputSourceState::DetachedObserved | InputSourceState::RestoredObserved
            ) && (!self.selection_attempted || self.fallback_source_id.is_none()))
            || (self.state == InputSourceState::DetachedObserved
                && self.observed_source_id != self.fallback_source_id)
            || (self.state == InputSourceState::RestoredObserved
                && (!self.restoration_attempted
                    || self.observed_source_id.as_ref() != Some(&self.original_source_id)))
        {
            return Err(InputSourceError::InvalidReply);
        }
        Ok(())
    }
}
fn validate_binding(
    subject: &Subject,
    marker: &MaintenanceMarker,
    code: &crate::native_code::CodeExpectation,
    rebinding: bool,
) -> Result<()> {
    code.validate().map_err(InputSourceError::Code)?;
    if marker.schema_version != 1
        || code.role != CodeRole::Ime
        || &code.subject != subject
        || marker.transaction_id != subject.transaction_id
        || marker.installation_id != subject.installation_id
        || marker.new_release_id != subject.new_release_id
        || marker.plan_sha256 != subject.plan_sha256
        || !inputia_settings::installation::valid_uuid(&marker.epoch)
        || match code.purpose {
            CodePurpose::PreviousRelease => {
                marker.old_release_id.as_ref() != Some(&code.release_id)
            }
            CodePurpose::NewRelease => !rebinding || code.release_id != subject.new_release_id,
        }
    {
        return Err(InputSourceError::InvalidAuthority);
    }
    Ok(())
}

/// 不可Clone/Deserialize/Send/Sync。Drop只释放观察和锁，不替用户切换输入源。
pub struct InputSourceLease {
    #[cfg(all(target_os = "macos", feature = "native-code-verification"))]
    native: implementation::Lease,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl InputSourceLease {
    /// 只登记观察，不产生TIS选择效应。必须在主线程调用，且当前marker确实属于此Transaction。
    pub fn prepare(transaction: &Transaction, ime: &VerifiedCodeEvidence) -> Result<Self> {
        let authority = transaction
            .guardian_authority()
            .map_err(InputSourceError::Transaction)?;
        validate_binding(
            &authority.subject,
            &authority.marker,
            ime.expectation(),
            false,
        )?;
        #[cfg(all(target_os = "macos", feature = "native-code-verification"))]
        {
            Ok(Self {
                native: implementation::Lease::prepare(authority, ime)?,
                _thread: std::marker::PhantomData,
            })
        }
        #[cfg(not(all(target_os = "macos", feature = "native-code-verification")))]
        {
            let _ = authority;
            Err(InputSourceError::NativeUnavailable)
        }
    }
    /// 单次切离；未知不自动重放。Result中的Observed不保证未来选择不变。
    pub fn detach(&mut self) -> Result<InputSourceObservation> {
        self.act(1, None)
    }
    /// 用当前/回滚IME重新绑定合同并读回；调用者不可缓存此观察作为长期停写证明。
    pub fn assert_detached(
        &mut self,
        ime: &VerifiedCodeEvidence,
    ) -> Result<InputSourceObservation> {
        self.act(2, Some(ime))
    }
    /// 只在未观察到用户改变且当前仍为本次fallback时条件恢复；当前/回滚IME必须重新验签。
    pub fn restore(&mut self, ime: &VerifiedCodeEvidence) -> Result<InputSourceObservation> {
        self.act(3, Some(ime))
    }
    fn act(
        &mut self,
        action: u32,
        code: Option<&VerifiedCodeEvidence>,
    ) -> Result<InputSourceObservation> {
        #[cfg(all(target_os = "macos", feature = "native-code-verification"))]
        {
            self.native.action(action, code)
        }
        #[cfg(not(all(target_os = "macos", feature = "native-code-verification")))]
        {
            let _ = (action, code);
            Err(InputSourceError::NativeUnavailable)
        }
    }
}

#[cfg(all(target_os = "macos", feature = "native-code-verification"))]
mod implementation {
    use super::*;
    use crate::{
        guardian::GuardianTransactionAuthority,
        native_code::{CodeExpectation, NativeCodeVerifier},
        native_quiescence,
    };
    use std::{
        ffi::{c_char, c_void},
        ptr::NonNull,
        time::{Duration, Instant},
    };
    const MAX_AGE_MS: u64 = 120_000;
    #[derive(Serialize)]
    struct Request<'a> {
        ime: &'a CodeExpectation,
        lease_id: &'a str,
        epoch: &'a str,
        max_age_ms: u64,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Reply {
        ok: bool,
        value: Option<InputSourceObservation>,
        code: Option<String>,
    }
    unsafe extern "C" {
        fn iuis_input_source_prepare(
            bytes: *const u8,
            count: usize,
            handle: *mut *mut c_void,
        ) -> *mut c_char;
        fn iuis_input_source_action(
            handle: *mut c_void,
            action: u32,
            bytes: *const u8,
            count: usize,
            check: Option<extern "C" fn(*mut c_void) -> i32>,
            context: *mut c_void,
        ) -> *mut c_char;
        fn iuis_input_source_free(handle: *mut c_void);
        fn iuis_string_free(value: *mut c_char);
    }
    struct Handle(NonNull<c_void>);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe { iuis_input_source_free(self.0.as_ptr()) };
        }
    }
    fn response(pointer: *mut c_char) -> Result<InputSourceObservation> {
        if pointer.is_null() {
            return Err(InputSourceError::NativeUnavailable);
        }
        let count = unsafe { libc::strnlen(pointer, 32_769) };
        let parsed = if count > 32_768 {
            None
        } else {
            serde_json::from_slice::<Reply>(unsafe {
                std::slice::from_raw_parts(pointer.cast::<u8>(), count)
            })
            .ok()
        };
        unsafe { iuis_string_free(pointer) };
        let parsed = parsed.ok_or(InputSourceError::InvalidReply)?;
        if parsed.ok && parsed.code.is_none() {
            let observation = parsed.value.ok_or(InputSourceError::InvalidReply)?;
            observation.validate()?;
            return Ok(observation);
        }
        if !parsed.ok && parsed.value.is_none() {
            if let Some(code) = parsed.code.filter(|s| {
                !s.is_empty()
                    && s.len() <= 64
                    && s.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
            }) {
                return Err(InputSourceError::Rejected(code));
            }
        }
        Err(InputSourceError::InvalidReply)
    }
    struct Check<'a> {
        authority: &'a GuardianTransactionAuthority,
        started: Instant,
    }
    impl Check<'_> {
        fn valid(&self) -> bool {
            native_quiescence::actual_marker(&self.authority.subject, &self.authority.marker)
                .is_ok()
                && self.started.elapsed() < Duration::from_millis(MAX_AGE_MS)
        }
    }
    extern "C" fn authorize(context: *mut c_void) -> i32 {
        if context.is_null() {
            return -1;
        }
        let check = unsafe { &*context.cast::<Check<'_>>() };
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check.valid())).unwrap_or(false)
        {
            0
        } else {
            -1
        }
    }
    pub(super) struct Lease {
        handle: Handle,
        authority: GuardianTransactionAuthority,
        code: CodeExpectation,
        evidence: VerifiedCodeEvidence,
        id: String,
        started: Instant,
    }
    impl Lease {
        pub fn prepare(
            authority: GuardianTransactionAuthority,
            ime: &VerifiedCodeEvidence,
        ) -> Result<Self> {
            let started = Instant::now();
            native_quiescence::actual_marker(&authority.subject, &authority.marker)
                .map_err(InputSourceError::Maintenance)?;
            NativeCodeVerifier
                .verify(ime.expectation(), ime.tree())
                .map_err(InputSourceError::Code)?;
            let id = uuid::Uuid::new_v4().to_string();
            let bytes = native_quiescence::canonical(&Request {
                ime: ime.expectation(),
                lease_id: &id,
                epoch: &authority.marker.epoch,
                max_age_ms: MAX_AGE_MS,
            })
            .map_err(InputSourceError::Maintenance)?;
            let mut pointer = std::ptr::null_mut();
            let raw =
                unsafe { iuis_input_source_prepare(bytes.as_ptr(), bytes.len(), &mut pointer) };
            let handle = NonNull::new(pointer).map(Handle);
            let observation = response(raw)?;
            if observation.state != InputSourceState::Prepared
                || !(Check {
                    authority: &authority,
                    started,
                })
                .valid()
            {
                return Err(InputSourceError::InvalidReply);
            }
            Ok(Self {
                handle: handle.ok_or(InputSourceError::InvalidReply)?,
                authority,
                code: ime.expectation().clone(),
                evidence: ime.clone(),
                id,
                started,
            })
        }
        pub fn action(
            &mut self,
            action: u32,
            ime: Option<&VerifiedCodeEvidence>,
        ) -> Result<InputSourceObservation> {
            let bytes = if let Some(ime) = ime {
                validate_binding(
                    &self.authority.subject,
                    &self.authority.marker,
                    ime.expectation(),
                    true,
                )?;
                if ime.expectation().bundle_id != self.code.bundle_id
                    || ime.expectation().exact_bundle_path != self.code.exact_bundle_path
                {
                    return Err(InputSourceError::InvalidAuthority);
                }
                NativeCodeVerifier
                    .verify(ime.expectation(), ime.tree())
                    .map_err(InputSourceError::Code)?;
                native_quiescence::canonical(&Request {
                    ime: ime.expectation(),
                    lease_id: &self.id,
                    epoch: &self.authority.marker.epoch,
                    max_age_ms: MAX_AGE_MS,
                })
                .map_err(InputSourceError::Maintenance)?
            } else {
                NativeCodeVerifier
                    .verify(self.evidence.expectation(), self.evidence.tree())
                    .map_err(InputSourceError::Code)?;
                vec![]
            };
            let mut check = Check {
                authority: &self.authority,
                started: self.started,
            };
            let mut observation = response(unsafe {
                iuis_input_source_action(
                    self.handle.0.as_ptr(),
                    action,
                    bytes.as_ptr(),
                    bytes.len(),
                    Some(authorize),
                    (&mut check as *mut Check<'_>).cast(),
                )
            })?;
            // FFI返回/反序列化也可能跨期限，返回前最后核一次，不扩大原生的观察结论。
            if !check.valid() {
                observation.state = InputSourceState::Uncertain;
                observation.reason = Some("maintenance_authority_expired".into());
            }
            if let Some(ime) = ime {
                self.evidence = ime.clone();
            }
            Ok(observation)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_code::CodeExpectation;
    fn fixture() -> (Subject, MaintenanceMarker, CodeExpectation) {
        let subject = Subject {
            transaction_id: "11111111-1111-4111-8111-111111111111".into(),
            installation_id: "22222222-2222-4222-8222-222222222222".into(),
            new_release_id: "inputia-new".into(),
            plan_sha256: "a".repeat(64),
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
        let code = CodeExpectation {
            schema_version: 1,
            subject: subject.clone(),
            purpose: CodePurpose::PreviousRelease,
            product_id: "com.inputia".into(),
            role: CodeRole::Ime,
            exact_bundle_path: "/fixture/Inputia.app".into(),
            bundle_id: "com.inputia.inputmethod.Fixture".into(),
            release_id: "inputia-old".into(),
            version: "1.0.0".into(),
            build: 1,
            source_commit: "b".repeat(40),
            team_id: "TESTTEAM01".into(),
            architectures: vec!["arm64".into()],
            cdhashes: vec!["c".repeat(40)],
        };
        (subject, marker, code)
    }
    #[test]
    fn exact_transaction_epoch_and_ime_role_bind_authority() {
        let (subject, marker, code) = fixture();
        assert!(validate_binding(&subject, &marker, &code, false).is_ok());
        let mut bad = code.clone();
        bad.role = CodeRole::Control;
        assert!(validate_binding(&subject, &marker, &bad, false).is_err());
        let mut bad = code.clone();
        bad.subject.transaction_id = uuid::Uuid::new_v4().to_string();
        assert!(validate_binding(&subject, &marker, &bad, false).is_err());
        let mut bad = marker.clone();
        bad.epoch = "forged".into();
        assert!(validate_binding(&subject, &bad, &code, false).is_err());
        let mut bad = marker.clone();
        bad.old_release_id = Some("inputia-other".into());
        assert!(validate_binding(&subject, &bad, &code, false).is_err());
        let mut bad = marker.clone();
        bad.schema_version = 2;
        assert!(validate_binding(&subject, &bad, &code, false).is_err());
    }
    #[test]
    fn new_release_allowed_only_for_explicit_rebinding_and_same_subject() {
        let (subject, marker, mut code) = fixture();
        code.purpose = CodePurpose::NewRelease;
        code.release_id = subject.new_release_id.clone();
        assert!(validate_binding(&subject, &marker, &code, false).is_err());
        assert!(validate_binding(&subject, &marker, &code, true).is_ok());
        code.release_id = "inputia-unrelated".into();
        assert!(validate_binding(&subject, &marker, &code, true).is_err());
    }
    #[test]
    fn observed_result_cannot_claim_atomic_ownership_or_invent_completion() {
        let value = InputSourceObservation {
            state: InputSourceState::DetachedObserved,
            original_source_id: "com.inputia.fixture.Hans".into(),
            observed_source_id: Some("com.apple.keylayout.ABC".into()),
            fallback_source_id: Some("com.apple.keylayout.ABC".into()),
            selection_attempted: true,
            restoration_attempted: false,
            ownership_exact: false,
            reason: None,
        };
        assert!(value.validate().is_ok());
        let mut bad = value.clone();
        bad.ownership_exact = true;
        assert!(bad.validate().is_err());
        let mut bad = value.clone();
        bad.observed_source_id = Some("com.fixture.unrelated".into());
        assert!(bad.validate().is_err());
        let mut bad = value.clone();
        bad.state = InputSourceState::RestoredObserved;
        assert!(bad.validate().is_err());
        let mut bad = value.clone();
        bad.selection_attempted = false;
        assert!(bad.validate().is_err());
        let mut raw = serde_json::to_value(value).unwrap();
        raw["verified"] = true.into();
        assert!(serde_json::from_value::<InputSourceObservation>(raw).is_err());
    }
}
