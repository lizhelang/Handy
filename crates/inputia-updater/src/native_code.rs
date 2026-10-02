//! 原生 Security 只读制品证据，不是安装授权，也不是完整 NativeAdapter。
//! 发布/配对清单真实性由上层离线信任链负责，期望值不能来自待验 app 自报。

#[cfg(all(target_os = "macos", feature = "native-code-verification"))]
use crate::fingerprint;
use crate::{Fingerprint, Subject};
use serde::{Deserialize, Serialize};
#[cfg(all(target_os = "macos", feature = "native-code-verification"))]
use sha2::{Digest, Sha256};
use std::path::PathBuf;

pub const MAX_NATIVE_REQUEST_BYTES: usize = 32_768;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeRole {
    Control,
    Ime,
    Settings,
    Updater,
    Bootstrap,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodePurpose {
    NewRelease,
    PreviousRelease,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeExpectation {
    pub schema_version: u32,
    pub subject: Subject,
    pub purpose: CodePurpose,
    pub product_id: String,
    pub role: CodeRole,
    pub exact_bundle_path: PathBuf,
    pub bundle_id: String,
    pub release_id: String,
    pub version: String,
    pub build: u64,
    pub source_commit: String,
    pub team_id: String,
    pub architectures: Vec<String>,
    pub cdhashes: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SliceEvidence {
    pub architecture: String,
    pub cdhash: String,
    pub identifier: String,
    pub team_id: String,
    pub hardened_runtime: bool,
    pub forbidden_entitlements_absent: bool,
    pub flags: u32,
    pub entitlements_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct NativeEvidence {
    schema_version: u32,
    request: CodeExpectation,
    slices: Vec<SliceEvidence>,
    developer_id_requirement: bool,
    notarized_requirement: bool,
    nested_code_integrity_checked: bool,
    bundle_device: u64,
    bundle_inode: u64,
}

/// 只能由本模块的实际原生验证建立；没有 Deserialize 或公开字段构造通路。
#[derive(Clone, Debug)]
pub struct VerifiedCodeEvidence {
    native: NativeEvidence,
    tree: Fingerprint,
    evidence_sha256: String,
}
impl VerifiedCodeEvidence {
    pub fn expectation(&self) -> &CodeExpectation {
        &self.native.request
    }
    pub fn slices(&self) -> &[SliceEvidence] {
        &self.native.slices
    }
    pub fn tree(&self) -> &Fingerprint {
        &self.tree
    }
    pub fn evidence_sha256(&self) -> &str {
        &self.evidence_sha256
    }
}

#[derive(Debug)]
pub enum NativeCodeError {
    InvalidExpectation,
    NativeUnavailable,
    Rejected { code: String, os_status: i32 },
    InvalidReply,
    ChangedArtifact,
    Filesystem(crate::Error),
    Json(serde_json::Error),
}
impl std::fmt::Display for NativeCodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for NativeCodeError {}
impl From<serde_json::Error> for NativeCodeError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

fn digest_text(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn sorted_unique(values: &[String]) -> bool {
    values.windows(2).all(|pair| pair[0] < pair[1])
}
impl CodeExpectation {
    pub fn validate(&self) -> Result<(), NativeCodeError> {
        use inputia_settings::installation::{valid_release_id, valid_uuid, PRODUCT_ID};
        let version: Vec<_> = self.version.split('.').collect();
        let path = self
            .exact_bundle_path
            .to_str()
            .ok_or(NativeCodeError::InvalidExpectation)?;
        if self.schema_version != 1
            || self.product_id != PRODUCT_ID
            || !valid_uuid(&self.subject.transaction_id)
            || !valid_uuid(&self.subject.installation_id)
            || !digest_text(&self.subject.plan_sha256, 64)
            || !valid_release_id(&self.subject.new_release_id)
            || match self.purpose {
                CodePurpose::NewRelease => self.subject.new_release_id != self.release_id,
                CodePurpose::PreviousRelease => self.subject.new_release_id == self.release_id,
            }
            || !valid_release_id(&self.release_id)
            || !(2..=191).contains(&self.bundle_id.len())
            || !self
                .bundle_id
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphanumeric)
            || !self
                .bundle_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
            || self.team_id.len() != 10
            || !self
                .team_id
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
            || !(digest_text(&self.source_commit, 40) || digest_text(&self.source_commit, 64))
            || self.build == 0
            || version.len() != 3
            || version
                .iter()
                .any(|v| v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit()))
            || self.architectures.is_empty()
            || self.architectures.len() > 3
            || !sorted_unique(&self.architectures)
            || !self
                .architectures
                .iter()
                .all(|v| matches!(v.as_str(), "arm64" | "arm64e" | "x86_64"))
            || self.cdhashes.len() != self.architectures.len()
            || !sorted_unique(&self.cdhashes)
            || !self.cdhashes.iter().all(|v| digest_text(v, 40))
            || !path.starts_with('/')
            || !path.ends_with(".app")
            || path.len() > 4096
            || path.contains("//")
            || path.chars().any(char::is_control)
            || path.split('/').any(|v| v == "." || v == "..")
        {
            return Err(NativeCodeError::InvalidExpectation);
        }
        Ok(())
    }
    pub fn canonical_request(&self) -> Result<Vec<u8>, NativeCodeError> {
        self.validate()?;
        let bytes = canonical(self)?;
        if bytes.len() > MAX_NATIVE_REQUEST_BYTES {
            return Err(NativeCodeError::InvalidExpectation);
        }
        Ok(bytes)
    }
}

fn canonical(value: &impl Serialize) -> Result<Vec<u8>, serde_json::Error> {
    fn sorted(value: serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(object) => {
                let ordered: std::collections::BTreeMap<_, _> =
                    object.into_iter().map(|(k, v)| (k, sorted(v))).collect();
                serde_json::Value::Object(ordered.into_iter().collect())
            }
            serde_json::Value::Array(values) => {
                serde_json::Value::Array(values.into_iter().map(sorted).collect())
            }
            value => value,
        }
    }
    serde_json::to_vec(&sorted(serde_json::to_value(value)?))
}

#[cfg(any(test, all(target_os = "macos", feature = "native-code-verification")))]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeReply {
    ok: bool,
    evidence: Option<NativeEvidence>,
    code: Option<String>,
    os_status: Option<i32>,
}

#[cfg(any(test, all(target_os = "macos", feature = "native-code-verification")))]
fn checked_reply(
    raw: &[u8],
    expectation: &CodeExpectation,
) -> Result<NativeEvidence, NativeCodeError> {
    if raw.len() > 65_536 {
        return Err(NativeCodeError::InvalidReply);
    }
    let reply: NativeReply = serde_json::from_slice(raw)?;
    if !reply.ok {
        if reply.evidence.is_some() {
            return Err(NativeCodeError::InvalidReply);
        }
        let code = reply.code.ok_or(NativeCodeError::InvalidReply)?;
        if code.len() > 64 || !code.bytes().all(|v| v.is_ascii_lowercase() || v == b'_') {
            return Err(NativeCodeError::InvalidReply);
        }
        return Err(NativeCodeError::Rejected {
            code,
            os_status: reply.os_status.ok_or(NativeCodeError::InvalidReply)?,
        });
    }
    if reply.code.is_some() || reply.os_status.is_some() {
        return Err(NativeCodeError::InvalidReply);
    }
    let proof = reply.evidence.ok_or(NativeCodeError::InvalidReply)?;
    if proof.schema_version != 1
        || &proof.request != expectation
        || !proof.developer_id_requirement
        || !proof.notarized_requirement
        || !proof.nested_code_integrity_checked
        || proof.bundle_inode == 0
        || proof
            .slices
            .iter()
            .map(|v| &v.architecture)
            .collect::<Vec<_>>()
            != expectation.architectures.iter().collect::<Vec<_>>()
        || proof.slices.iter().any(|v| {
            v.identifier != expectation.bundle_id
                || v.team_id != expectation.team_id
                || !v.hardened_runtime
                || !v.forbidden_entitlements_absent
                || v.flags & 0x10000 == 0
                || v.flags & 2 != 0
                || !digest_text(&v.entitlements_sha256, 64)
        })
    {
        return Err(NativeCodeError::InvalidReply);
    }
    let mut hashes: Vec<_> = proof.slices.iter().map(|v| v.cdhash.clone()).collect();
    hashes.sort();
    if hashes != expectation.cdhashes {
        return Err(NativeCodeError::InvalidReply);
    }
    Ok(proof)
}

#[cfg(any(test, all(target_os = "macos", feature = "native-code-verification")))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RootIdentity {
    device: u64,
    inode: u64,
}

#[cfg(any(test, all(target_os = "macos", feature = "native-code-verification")))]
fn hold_root(
    path: &std::path::Path,
    uid: u32,
) -> Result<(std::fs::File, RootIdentity), NativeCodeError> {
    use std::os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::MetadataExt,
    };
    let (parent, name) =
        crate::filesystem::parent(path, uid, false).map_err(NativeCodeError::Filesystem)?;
    // SAFETY: 父句柄和单段名称已核验，成功后由 File 独占关闭；持有根直到验证结束。
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(NativeCodeError::Filesystem(
            std::io::Error::last_os_error().into(),
        ));
    }
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    let meta = file
        .metadata()
        .map_err(|e| NativeCodeError::Filesystem(e.into()))?;
    if !meta.is_dir() || meta.uid() != uid || meta.mode() & 0o7022 != 0 {
        return Err(NativeCodeError::Filesystem(crate::Error::UnsafePath));
    }
    let identity = RootIdentity {
        device: meta.dev(),
        inode: meta.ino(),
    };
    Ok((file, identity))
}

#[cfg(any(test, all(target_os = "macos", feature = "native-code-verification")))]
fn bind_root(
    native: &NativeEvidence,
    before: RootIdentity,
    after: RootIdentity,
) -> Result<(), NativeCodeError> {
    if before != after
        || native.bundle_device != before.device
        || native.bundle_inode != before.inode
    {
        return Err(NativeCodeError::ChangedArtifact);
    }
    Ok(())
}

pub struct NativeCodeVerifier;
impl NativeCodeVerifier {
    /// 同步只读验证；调用前后复核整棵树，证据只对当下未修改的制品有效。
    pub fn verify(
        &self,
        expectation: &CodeExpectation,
        expected_tree: &Fingerprint,
    ) -> Result<VerifiedCodeEvidence, NativeCodeError> {
        let request = expectation.canonical_request()?;
        #[cfg(not(all(target_os = "macos", feature = "native-code-verification")))]
        {
            let _ = (request, expected_tree);
            Err(NativeCodeError::NativeUnavailable)
        }
        #[cfg(all(target_os = "macos", feature = "native-code-verification"))]
        {
            let uid = unsafe { libc::geteuid() };
            let (_root_guard, identity) = hold_root(&expectation.exact_bundle_path, uid)?;
            let before = fingerprint(&expectation.exact_bundle_path, uid)
                .map_err(NativeCodeError::Filesystem)?;
            if &before != expected_tree
                || hold_root(&expectation.exact_bundle_path, uid)?.1 != identity
            {
                return Err(NativeCodeError::ChangedArtifact);
            }
            let raw = native_call(&request)?;
            let native = checked_reply(&raw, expectation)?;
            bind_root(
                &native,
                identity,
                hold_root(&expectation.exact_bundle_path, uid)?.1,
            )?;
            let after = fingerprint(&expectation.exact_bundle_path, uid)
                .map_err(NativeCodeError::Filesystem)?;
            if before != after {
                return Err(NativeCodeError::ChangedArtifact);
            }
            bind_root(
                &native,
                identity,
                hold_root(&expectation.exact_bundle_path, uid)?.1,
            )?;
            let evidence_sha256 = format!("{:x}", Sha256::digest(canonical(&(&native, &after))?));
            Ok(VerifiedCodeEvidence {
                native,
                tree: after,
                evidence_sha256,
            })
        }
    }
}

#[cfg(all(target_os = "macos", feature = "native-code-verification"))]
unsafe extern "C" {
    fn iuis_verify_code(bytes: *const u8, length: usize) -> *mut std::ffi::c_char;
    fn iuis_string_free(string: *mut std::ffi::c_char);
}

#[cfg(all(target_os = "macos", feature = "native-code-verification"))]
fn native_call(bytes: &[u8]) -> Result<Vec<u8>, NativeCodeError> {
    if bytes.is_empty() || bytes.len() > MAX_NATIVE_REQUEST_BYTES {
        return Err(NativeCodeError::InvalidExpectation);
    }
    let pointer = unsafe { iuis_verify_code(bytes.as_ptr(), bytes.len()) };
    if pointer.is_null() {
        return Err(NativeCodeError::NativeUnavailable);
    }
    let count = unsafe { libc::strnlen(pointer, 65_537) };
    let result = if count > 65_536 {
        Err(NativeCodeError::InvalidReply)
    } else {
        Ok(unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), count) }.to_vec())
    };
    unsafe { iuis_string_free(pointer) };
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expectation() -> CodeExpectation {
        CodeExpectation {
            schema_version: 1,
            subject: Subject {
                transaction_id: "11111111-1111-4111-8111-111111111111".into(),
                plan_sha256: "a".repeat(64),
                installation_id: "22222222-2222-4222-8222-222222222222".into(),
                new_release_id: "inputia-1.1.0-test".into(),
            },
            purpose: CodePurpose::NewRelease,
            product_id: "com.inputia".into(),
            role: CodeRole::Control,
            exact_bundle_path: "/missing-inputia-native-fixture/合成.app".into(),
            bundle_id: "com.inputia.test.native".into(),
            release_id: "inputia-1.1.0-test".into(),
            version: "1.1.0".into(),
            build: 84,
            source_commit: "b".repeat(40),
            team_id: "TESTTEAM01".into(),
            architectures: vec!["arm64".into()],
            cdhashes: vec!["11".repeat(20)],
        }
    }
    #[test]
    fn native_expectation_rejects_injection_wrong_binding_and_ambiguous_sets() {
        let value = expectation();
        value.validate().unwrap();
        let mut sha256_commit = value.clone();
        sha256_commit.source_commit = "b".repeat(64);
        sha256_commit.validate().unwrap();
        sha256_commit.source_commit.pop();
        assert!(sha256_commit.validate().is_err());
        sha256_commit.source_commit = "A".repeat(64);
        assert!(sha256_commit.validate().is_err());
        let mut invalid = value.clone();
        invalid.team_id.push('"');
        assert!(invalid.validate().is_err());
        invalid = value.clone();
        invalid.bundle_id.push('\n');
        assert!(invalid.validate().is_err());
        invalid = value.clone();
        invalid.subject.new_release_id = "inputia-other".into();
        assert!(invalid.validate().is_err());
        invalid.purpose = CodePurpose::PreviousRelease;
        invalid.validate().unwrap();
        invalid.subject.new_release_id = value.release_id.clone();
        assert!(invalid.validate().is_err());
        invalid = value.clone();
        invalid.exact_bundle_path = "/tmp/../other.app".into();
        assert!(invalid.validate().is_err());
        invalid = value.clone();
        invalid.architectures.push("arm64".into());
        invalid.cdhashes.push("22".repeat(20));
        assert!(invalid.validate().is_err());
        invalid = value.clone();
        invalid.cdhashes[0] = "AA".repeat(20);
        assert!(invalid.validate().is_err());
        invalid = value;
        invalid.version = format!("{}.0.0", "1".repeat(MAX_NATIVE_REQUEST_BYTES));
        assert!(invalid.canonical_request().is_err());
    }

    // 合成 reply 仅测试 IPC 绑定校验，不从本测试返回生产 VerifiedCodeEvidence。
    fn reply(value: &CodeExpectation) -> serde_json::Value {
        serde_json::json!({"ok":true,"evidence":{
            "schema_version":1,"request":value,"slices":[{"architecture":"arm64","cdhash":"11".repeat(20),
                "identifier":value.bundle_id,"team_id":value.team_id,"hardened_runtime":true,
                "forbidden_entitlements_absent":true,"flags":65536,"entitlements_sha256":"c".repeat(64)}],
            "developer_id_requirement":true,"notarized_requirement":true,"nested_code_integrity_checked":true,
            "bundle_device":1,"bundle_inode":1}})
    }
    #[test]
    fn native_reply_is_exactly_bound_and_missing_checks_never_authorize() {
        let value = expectation();
        let original = reply(&value);
        checked_reply(&serde_json::to_vec(&original).unwrap(), &value).unwrap();
        for field in [
            "developer_id_requirement",
            "notarized_requirement",
            "nested_code_integrity_checked",
        ] {
            let mut bad = original.clone();
            bad["evidence"][field] = false.into();
            assert!(checked_reply(&serde_json::to_vec(&bad).unwrap(), &value).is_err());
        }
        for (field, data) in [
            ("flags", serde_json::json!(65538)),
            ("team_id", serde_json::json!("OTHERTEAM1")),
            ("cdhash", serde_json::json!("22".repeat(20))),
            ("architecture", serde_json::json!("x86_64")),
            ("entitlements_sha256", serde_json::json!("not-a-hash")),
        ] {
            let mut bad = original.clone();
            bad["evidence"]["slices"][0][field] = data;
            assert!(checked_reply(&serde_json::to_vec(&bad).unwrap(), &value).is_err());
        }
        let mut wrong = value.clone();
        wrong.subject.plan_sha256 = "d".repeat(64);
        assert!(checked_reply(&serde_json::to_vec(&original).unwrap(), &wrong).is_err());
        let mut extra = original;
        extra["ignored"] = true.into();
        assert!(checked_reply(&serde_json::to_vec(&extra).unwrap(), &value).is_err());
        let raw = br#"{"ok":false,"code":"untrusted_signature","code":"other","os_status":-67050}"#;
        assert!(checked_reply(raw, &value).is_err());
    }

    #[test]
    fn native_evidence_must_match_held_root_even_when_paths_return_to_original() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let app = base.join("Fixture.app");
        std::fs::create_dir(&app).unwrap();
        let uid = unsafe { libc::geteuid() };
        let (_guard, before) = hold_root(&app, uid).unwrap();
        std::fs::rename(&app, base.join("Old.app")).unwrap();
        std::fs::create_dir(&app).unwrap();
        let (_, changed) = hold_root(&app, uid).unwrap();
        let expected = expectation();
        let mut proof =
            checked_reply(&serde_json::to_vec(&reply(&expected)).unwrap(), &expected).unwrap();
        proof.bundle_device = changed.device;
        proof.bundle_inode = changed.inode;
        std::fs::remove_dir(&app).unwrap();
        std::fs::rename(base.join("Old.app"), &app).unwrap();
        let (_, after) = hold_root(&app, uid).unwrap();
        assert_eq!(before, after);
        assert!(matches!(
            bind_root(&proof, before, after),
            Err(NativeCodeError::ChangedArtifact)
        ));
        proof.bundle_device = before.device;
        proof.bundle_inode = before.inode;
        bind_root(&proof, before, after).unwrap();
        assert!(matches!(
            bind_root(&proof, before, changed),
            Err(NativeCodeError::ChangedArtifact)
        ));
    }

    #[cfg(not(feature = "native-code-verification"))]
    #[test]
    fn no_native_feature_never_returns_simulated_success() {
        let fake = Fingerprint {
            sha256: "a".repeat(64),
            bytes: 1,
            entries: 1,
        };
        assert!(matches!(
            NativeCodeVerifier.verify(&expectation(), &fake),
            Err(NativeCodeError::NativeUnavailable)
        ));
    }

    #[cfg(all(target_os = "macos", feature = "native-code-verification"))]
    #[test]
    fn actual_native_verifier_rejects_temporary_adhoc_and_changed_tree() {
        use std::{fs, process::Command};
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let app = base.join("合成.app");
        fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        let source = base.join("fixture.c");
        fs::write(&source, b"int main(void) { return 0; }\n").unwrap();
        let binary = app.join("Contents/MacOS/Fixture");
        assert!(Command::new("/usr/bin/clang")
            .args(["-target", "arm64-apple-macos13.0"])
            .arg(&source)
            .arg("-o")
            .arg(&binary)
            .status()
            .unwrap()
            .success());
        let mut expected = expectation();
        expected.exact_bundle_path = app.clone();
        let plist = format!(
            r#"<?xml version="1.0"?><plist version="1.0"><dict>
            <key>CFBundleIdentifier</key><string>{}</string><key>CFBundleExecutable</key><string>Fixture</string>
            <key>CFBundlePackageType</key><string>APPL</string><key>CFBundleVersion</key><string>84</string>
            <key>CFBundleShortVersionString</key><string>1.1.0</string><key>InputiaReleaseID</key><string>{}</string>
            <key>InputiaSourceCommit</key><string>{}</string></dict></plist>"#,
            expected.bundle_id, expected.release_id, expected.source_commit
        );
        fs::write(app.join("Contents/Info.plist"), plist).unwrap();
        assert!(Command::new("/usr/bin/codesign")
            .args(["--force", "--sign", "-", "--options", "runtime"])
            .arg(&app)
            .output()
            .unwrap()
            .status
            .success());
        let tree = fingerprint(&app, unsafe { libc::geteuid() }).unwrap();
        assert!(matches!(NativeCodeVerifier.verify(&expected, &tree),
            Err(NativeCodeError::Rejected{code,..}) if code == "untrusted_signature"));
        fs::write(app.join("Contents/new-resource"), b"new bytes retained").unwrap();
        assert!(matches!(
            NativeCodeVerifier.verify(&expected, &tree),
            Err(NativeCodeError::ChangedArtifact)
        ));
        assert_eq!(
            fs::read(app.join("Contents/new-resource")).unwrap(),
            b"new bytes retained"
        );
    }

    #[cfg(all(target_os = "macos", feature = "native-code-verification"))]
    #[test]
    fn real_native_abi_decodes_rust_canonical_unicode_and_refuses_missing_app() {
        let expected = expectation();
        let raw = native_call(&expected.canonical_request().unwrap()).unwrap();
        assert!(
            matches!(checked_reply(&raw, &expected), Err(NativeCodeError::Rejected { code, .. }) if code == "unsafe_path")
        );
        assert!(matches!(
            native_call(&vec![b' '; MAX_NATIVE_REQUEST_BYTES + 1]),
            Err(NativeCodeError::InvalidExpectation)
        ));
        let pointer = unsafe { iuis_verify_code(std::ptr::null(), usize::MAX) };
        assert!(!pointer.is_null());
        let response = unsafe { std::ffi::CStr::from_ptr(pointer) }
            .to_bytes()
            .to_vec();
        unsafe { iuis_string_free(pointer) };
        assert!(
            matches!(checked_reply(&response, &expected), Err(NativeCodeError::Rejected { code, .. }) if code == "invalid_request")
        );
    }
}
