//! 新更新必须经过当前根授权的频道、固定字节摘要和本机兼容检查。
//! 返回值仅证明发布目录授权；安装仍需逐文件校验、Apple 验签和事务预检。
use crate::{
    digest, manifest, safe_relative,
    trust::{DocumentKind, Envelope, Track, VerifiedKeyset},
    utc, valid_digest, ReleaseError, Result,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RollbackRecord {
    pub from_release_id: String,
    pub from_manifest_digest: String,
    pub to_release_id: String,
    pub to_manifest_digest: String,
    pub report_digest: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Feed {
    pub schema_version: u32,
    pub product_id: String,
    pub channel: String,
    pub platform: String,
    pub architecture: String,
    pub sequence: u64,
    pub keyset_version: u64,
    pub keyset_digest: String,
    pub archive_policy_id: String,
    pub issued_at: String,
    pub expires_at: String,
    pub release_id: String,
    pub manifest_digest: String,
    pub attestation_digest: String,
    pub release_path: String,
    pub rollback: Option<RollbackRecord>,
}
impl Feed {
    pub fn track(&self) -> Track {
        Track {
            channel: self.channel.clone(),
            platform: self.platform.clone(),
            architecture: self.architecture.clone(),
        }
    }
    fn validate(&self, now: i64) -> Result<()> {
        self.track().validate()?;
        if self.schema_version != 2
            || self.product_id != "com.inputia"
            || self.sequence == 0
            || self.sequence > 9_007_199_254_740_991
            || self.keyset_version == 0
            || self.keyset_version > 9_007_199_254_740_991
            || ![
                &self.keyset_digest,
                &self.manifest_digest,
                &self.attestation_digest,
            ]
            .into_iter()
            .all(|v| valid_digest(v))
            || !crate::valid_id(&self.archive_policy_id)
            || !valid_release_id(&self.release_id)
            || self.release_path != format!("releases/{}", self.release_id)
        {
            return Err(ReleaseError::InvalidDocument);
        }
        safe_relative(&self.release_path)?;
        let start = utc(&self.issued_at)?;
        let end = utc(&self.expires_at)?;
        if end <= start || end - start > 7 * 24 * 60 * 60 {
            return Err(ReleaseError::InvalidDocument);
        }
        if now < start || now >= end {
            return Err(ReleaseError::Expired);
        }
        if let Some(record) = &self.rollback {
            if !valid_release_id(&record.from_release_id)
                || record.to_release_id != self.release_id
                || record.to_manifest_digest != self.manifest_digest
                || !valid_digest(&record.from_manifest_digest)
                || !valid_digest(&record.report_digest)
                || record.from_release_id == record.to_release_id
            {
                return Err(ReleaseError::InvalidDocument);
            }
        }
        Ok(())
    }
}
fn valid_release_id(id: &str) -> bool {
    id.strip_prefix("inputia-")
        .map(|id| {
            (1..=180).contains(&id.len())
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        })
        .unwrap_or(false)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FeedCheckpoint {
    pub track: Track,
    pub sequence: u64,
    pub document_digest: String,
}

/// 只能由验签入口产生；持久高水位提交成功前不得据此安装。
#[derive(Clone, Debug)]
pub struct VerifiedFeed {
    feed: Feed,
    document_digest: String,
}
impl VerifiedFeed {
    pub fn payload(&self) -> &Feed {
        &self.feed
    }
    pub fn checkpoint(&self) -> FeedCheckpoint {
        FeedCheckpoint {
            track: self.feed.track(),
            sequence: self.feed.sequence,
            document_digest: self.document_digest.clone(),
        }
    }
    pub fn check_high_water(&self, previous: Option<&FeedCheckpoint>) -> Result<()> {
        if let Some(previous) = previous {
            if previous.track != self.feed.track() {
                return Err(ReleaseError::WrongPurpose);
            }
            if self.feed.sequence < previous.sequence {
                return Err(ReleaseError::Replay);
            }
            if self.feed.sequence == previous.sequence
                && self.document_digest != previous.document_digest
            {
                return Err(ReleaseError::ConflictingSequence);
            }
        }
        Ok(())
    }
}

impl VerifiedKeyset {
    pub fn authorize_feed(&self, bytes: &[u8], expected: &Track, now: i64) -> Result<VerifiedFeed> {
        let envelope = Envelope::parse(bytes)?;
        if envelope.payload_kind != DocumentKind::Feed {
            return Err(ReleaseError::WrongPurpose);
        }
        crate::schema::validate(
            &envelope.payload,
            &crate::canonical::parse(include_bytes!(
                "../../../release/schema/channel-feed-v2.schema.json"
            ))?,
        )?;
        if envelope.payload.get("rollback").is_none() {
            return Err(ReleaseError::InvalidDocument);
        }
        let document_digest = crate::trust::semantic_digest(DocumentKind::Feed, &envelope.payload)?;
        let feed: Feed =
            serde_json::from_value(envelope.payload).map_err(|_| ReleaseError::InvalidDocument)?;
        feed.validate(now)?;
        if &feed.track() != expected {
            return Err(ReleaseError::WrongPurpose);
        }
        let checkpoint = self.checkpoint();
        if feed.keyset_version != checkpoint.version
            || feed.keyset_digest != checkpoint.document_digest
        {
            return Err(ReleaseError::DigestMismatch);
        }
        self.verify_feed(bytes, expected, now, utc(&feed.expires_at)?)?;
        Ok(VerifiedFeed {
            document_digest,
            feed,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trust::{tests::*, TrustRoot};
    use serde_json::json;

    struct Fixture {
        keyset: VerifiedKeyset,
        signing: SigningKey,
        archive: SigningKey,
        manifest: Vec<u8>,
        attestation: Vec<u8>,
        feed: Feed,
    }
    impl Fixture {
        fn new() -> Self {
            let (root, archive, signing) =
                (SigningKey::new(), SigningKey::new(), SigningKey::new());
            let keyset = TrustRoot::from_embedded("com.inputia", vec![root.public.clone()], 1)
                .unwrap()
                .advance(&signed(
                    DocumentKind::Keyset,
                    serde_json::to_value(payload(&root, &archive, &signing)).unwrap(),
                    &[&root],
                ))
                .unwrap();
            let manifest = signed(
                DocumentKind::Manifest,
                crate::canonical::parse(include_bytes!("../tests/fixtures/manifest.json")).unwrap(),
                &[&archive],
            );
            let attestation = signed(
                DocumentKind::Attestation,
                json!({"schema_version":1,"product_id":"com.inputia","release_id":"inputia-test-current","manifest_digest":digest(&manifest),"acceptance_report_digest":digest(b"acceptance"),"rollback_reports":[{"release_id":"inputia-test-compatible","report_digest":digest(b"rollback")}],"issued_at":"2026-09-29T00:00:00Z"}),
                &[&archive],
            );
            let feed = Feed {
                schema_version: 2,
                product_id: "com.inputia".into(),
                channel: "candidate".into(),
                platform: "macos".into(),
                architecture: "arm64".into(),
                sequence: 5,
                keyset_version: 1,
                keyset_digest: keyset.checkpoint().document_digest,
                archive_policy_id: "release-2026".into(),
                issued_at: "2026-09-30T00:00:00Z".into(),
                expires_at: "2026-10-02T00:00:00Z".into(),
                release_id: "inputia-test-current".into(),
                manifest_digest: digest(&manifest),
                attestation_digest: digest(&attestation),
                release_path: "releases/inputia-test-current".into(),
                rollback: None,
            };
            Self {
                keyset,
                signing,
                archive,
                manifest,
                attestation,
                feed,
            }
        }
        fn raw_feed(&self) -> Vec<u8> {
            signed(
                DocumentKind::Feed,
                serde_json::to_value(&self.feed).unwrap(),
                &[&self.signing],
            )
        }
        fn docs(&self) -> ReleaseDocuments<'_> {
            ReleaseDocuments {
                manifest: &self.manifest,
                attestation: &self.attestation,
                acceptance_report: b"acceptance",
                rollback_reports: BTreeMap::from([(
                    "inputia-test-compatible".into(),
                    b"rollback".as_slice(),
                )]),
            }
        }
        fn host() -> Host<'static> {
            Host {
                platform: "macos",
                architecture: "arm64",
                os_version: "26.0",
                updater_version: "1.0.0",
                transaction_schema: 1,
                installed: None,
            }
        }
        fn authorize(&self) -> Result<AuthorizedReleaseMetadata> {
            let feed = self
                .keyset
                .authorize_feed(&self.raw_feed(), &candidate(), now())?;
            self.keyset
                .authorize_release_metadata(&feed, &self.docs(), &Self::host(), now())
        }
    }

    #[test]
    fn complete_signed_metadata_chain_binds_reports_host_and_immutable_bytes() {
        let mut f = Fixture::new();
        let authorized = f.authorize().unwrap();
        let native = authorized.native_release_policy();
        assert_eq!(native.product_id(), "com.inputia");
        assert_eq!(native.release_id(), "inputia-test-current");
        assert_eq!(native.version(), "1.1.0");
        assert_eq!(native.build(), 84);
        assert_eq!(native.source_commit(), "c".repeat(40));
        assert_eq!(native.architecture(), "arm64");
        assert_eq!(native.components().len(), 5);
        assert_eq!(
            native
                .components()
                .iter()
                .map(|component| component.role())
                .collect::<Vec<_>>(),
            vec![
                crate::native_policy::NativeComponentRole::Control,
                crate::native_policy::NativeComponentRole::Ime,
                crate::native_policy::NativeComponentRole::Settings,
                crate::native_policy::NativeComponentRole::Updater,
                crate::native_policy::NativeComponentRole::Bootstrap,
            ]
        );
        assert!(native.components().iter().all(|component| {
            component.team_id() == "TESTTEAM01"
                && component.archive_sha256() == "a".repeat(64)
                && component.archive_size() == 4
                && component.slices().len() == 1
                && component.slices()[0].architecture() == "arm64"
                && component.slices()[0].cdhash() == "b".repeat(40)
                && component.artifact().starts_with("components/")
                && !component.bundle_id().is_empty()
        }));
        assert_eq!(native.pair_manifest().artifact(), "pair-manifest.json");
        assert_eq!(native.pair_manifest().sha256(), "a".repeat(64));
        assert_eq!(native.pair_manifest().signer_key_id(), "pair-key-1");
        assert_eq!(native.pair_manifest().schema(), 2);
        let feed = f
            .keyset
            .authorize_feed(&f.raw_feed(), &candidate(), now())
            .unwrap();
        let mut docs = f.docs();
        docs.acceptance_report = b"replacement";
        assert!(f
            .keyset
            .authorize_release_metadata(&feed, &docs, &Fixture::host(), now())
            .is_err());
        let mut host = Fixture::host();
        host.os_version = "14.0";
        assert!(f
            .keyset
            .authorize_release_metadata(&feed, &f.docs(), &host, now())
            .is_err());
        host.os_version = "26.0";
        host.updater_version = "0.9.0";
        assert!(f
            .keyset
            .authorize_release_metadata(&feed, &f.docs(), &host, now())
            .is_err());
        f.manifest.push(b' ');
        assert!(f.authorize().is_err());
        f.feed.manifest_digest = digest(&f.manifest);
        assert!(f.authorize().is_err()); // attestation 仍绑定原 manifest。
    }

    #[test]
    fn native_policy_is_frozen_at_authorization_even_if_internal_json_changes() {
        let f = Fixture::new();
        let mut authorized = f.authorize().unwrap();
        let frozen = authorized.native_release_policy().clone();
        authorized.manifest["components"][0]["cdhashes"][0] = json!("c".repeat(40));
        assert_eq!(authorized.native_release_policy(), &frozen);
        assert_eq!(
            authorized.native_release_policy().components()[0].slices()[0].cdhash(),
            "b".repeat(40)
        );
    }

    #[test]
    fn feed_rejects_track_expiry_keyset_mismatch_and_sequence_replay() {
        let mut f = Fixture::new();
        let verified = f
            .keyset
            .authorize_feed(&f.raw_feed(), &candidate(), now())
            .unwrap();
        let checkpoint = verified.checkpoint();
        // 相同已签正文可以有不同 ECDSA 签名/JSON 空白，不冻结合法后续序号。
        let variant = serde_json::to_vec_pretty(&Envelope::parse(&f.raw_feed()).unwrap()).unwrap();
        let variant = f
            .keyset
            .authorize_feed(&variant, &candidate(), now())
            .unwrap();
        variant.check_high_water(Some(&checkpoint)).unwrap();
        f.feed.sequence = 4;
        assert_eq!(
            f.keyset
                .authorize_feed(&f.raw_feed(), &candidate(), now())
                .unwrap()
                .check_high_water(Some(&checkpoint)),
            Err(ReleaseError::Replay)
        );
        f.feed.sequence = 5;
        f.feed.expires_at = "2026-10-03T00:00:00Z".into();
        assert_eq!(
            f.keyset
                .authorize_feed(&f.raw_feed(), &candidate(), now())
                .unwrap()
                .check_high_water(Some(&checkpoint)),
            Err(ReleaseError::ConflictingSequence)
        );
        f.feed.expires_at = "2026-11-03T00:00:00Z".into();
        assert!(f.authorize().is_err());
        f.feed.expires_at = "2026-09-30T00:00:00Z".into();
        assert!(f.authorize().is_err());
        f.feed.expires_at = "2026-10-02T00:00:00Z".into();
        f.feed.keyset_digest = "a".repeat(64);
        assert!(f.authorize().is_err());
        f.feed.keyset_digest = f.keyset.checkpoint().document_digest;
        f.feed.channel = "stable".into();
        assert!(f.authorize().is_err());
    }

    #[test]
    fn downgrade_requires_explicit_source_bound_report_and_old_contract() {
        let mut f = Fixture::new();
        let mut current = Envelope::parse(&f.manifest).unwrap().payload;
        current["release_id"] = json!("inputia-test-newer");
        current["build"] = json!(85);
        current["rollback_targets"][0]["release_id"] = json!("inputia-test-current");
        let source = signed(DocumentKind::Manifest, current.clone(), &[&f.archive]);
        let source_digest = digest(&source);
        let proof = signed(
            DocumentKind::Attestation,
            json!({"schema_version":1,"product_id":"com.inputia","release_id":"inputia-test-newer","manifest_digest":source_digest,"acceptance_report_digest":digest(b"old acceptance"),"rollback_reports":[{"release_id":"inputia-test-current","report_digest":digest(b"new-to-old")}],"issued_at":"2026-09-29T00:00:00Z"}),
            &[&f.archive],
        );
        let mut host = Fixture::host();
        host.installed = Some(InstalledRelease {
            manifest: &current,
            manifest_digest: &source_digest,
            rollback_evidence: None,
        });
        let feed = f
            .keyset
            .authorize_feed(&f.raw_feed(), &candidate(), now())
            .unwrap();
        assert_eq!(
            f.keyset
                .authorize_release_metadata(&feed, &f.docs(), &host, now())
                .unwrap_err(),
            ReleaseError::Replay
        );
        f.feed.rollback = Some(RollbackRecord {
            from_release_id: "inputia-test-newer".into(),
            from_manifest_digest: source_digest.clone(),
            to_release_id: f.feed.release_id.clone(),
            to_manifest_digest: f.feed.manifest_digest.clone(),
            report_digest: digest(b"new-to-old"),
        });
        let feed = f
            .keyset
            .authorize_feed(&f.raw_feed(), &candidate(), now())
            .unwrap();
        assert!(f
            .keyset
            .authorize_release_metadata(&feed, &f.docs(), &host, now())
            .is_err());
        host.installed.as_mut().unwrap().rollback_evidence = Some(InstalledRollbackEvidence {
            archive_policy_id: "release-2026",
            attestation: &proof,
            report: b"new-to-old",
        });
        f.keyset
            .authorize_release_metadata(&feed, &f.docs(), &host, now())
            .unwrap();
        host.installed
            .as_mut()
            .unwrap()
            .rollback_evidence
            .as_mut()
            .unwrap()
            .report = b"changed";
        assert!(f
            .keyset
            .authorize_release_metadata(&feed, &f.docs(), &host, now())
            .is_err());
    }
}

/// 来自本机受保护收据/已验签旧清单的事实，不能由下载的目标 feed 填充。
pub struct InstalledRelease<'a> {
    pub manifest: &'a Value,
    pub manifest_digest: &'a str,
    /// 仅回退时需要；来源是已装版本冻结的签名报告。
    pub rollback_evidence: Option<InstalledRollbackEvidence<'a>>,
}
pub struct InstalledRollbackEvidence<'a> {
    pub archive_policy_id: &'a str,
    pub attestation: &'a [u8],
    pub report: &'a [u8],
}
pub struct Host<'a> {
    pub platform: &'a str,
    pub architecture: &'a str,
    pub os_version: &'a str,
    pub updater_version: &'a str,
    pub transaction_schema: u64,
    pub installed: Option<InstalledRelease<'a>>,
}
pub struct ReleaseDocuments<'a> {
    pub manifest: &'a [u8],
    pub attestation: &'a [u8],
    /// 保留报告原字节。验收执行器/签署环节负责真实证据和 gate 判定。
    pub acceptance_report: &'a [u8],
    pub rollback_reports: BTreeMap<String, &'a [u8]>,
}

/// 不可反序列化或外部构造。不是安装许可，也不表示原生签名/磁盘工况已经通过。
#[derive(Clone, Debug)]
pub struct AuthorizedReleaseMetadata {
    manifest: Value,
    attestation: Value,
    feed: VerifiedFeed,
    native_policy: crate::native_policy::NativeReleasePolicy,
}
impl AuthorizedReleaseMetadata {
    pub fn manifest(&self) -> &Value {
        &self.manifest
    }
    pub fn attestation(&self) -> &Value {
        &self.attestation
    }
    pub fn feed(&self) -> &VerifiedFeed {
        &self.feed
    }
    pub fn native_release_policy(&self) -> &crate::native_policy::NativeReleasePolicy {
        &self.native_policy
    }
}

impl VerifiedKeyset {
    pub(crate) fn authorize_release_metadata(
        &self,
        feed: &VerifiedFeed,
        docs: &ReleaseDocuments<'_>,
        host: &Host<'_>,
        now: i64,
    ) -> Result<AuthorizedReleaseMetadata> {
        feed.feed.validate(now)?;
        let checkpoint = self.checkpoint();
        if checkpoint.version != feed.feed.keyset_version
            || checkpoint.document_digest != feed.feed.keyset_digest
        {
            return Err(ReleaseError::DigestMismatch);
        }
        if digest(docs.manifest) != feed.feed.manifest_digest
            || digest(docs.attestation) != feed.feed.attestation_digest
        {
            return Err(ReleaseError::DigestMismatch);
        }
        let manifest = self.verify_archive_policy(
            docs.manifest,
            DocumentKind::Manifest,
            &feed.feed.archive_policy_id,
            now,
        )?;
        let attestation = self.verify_archive_policy(
            docs.attestation,
            DocumentKind::Attestation,
            &feed.feed.archive_policy_id,
            now,
        )?;
        manifest::validate_manifest(&manifest)?;
        manifest::validate_attestation(&attestation)?;
        if manifest["release_id"] != feed.feed.release_id
            || attestation["release_id"] != feed.feed.release_id
            || attestation["manifest_digest"] != feed.feed.manifest_digest
            || attestation["acceptance_report_digest"] != digest(docs.acceptance_report)
            || utc(attestation["issued_at"]
                .as_str()
                .ok_or(ReleaseError::InvalidDocument)?)?
                > utc(&feed.feed.issued_at)?
        {
            return Err(ReleaseError::DigestMismatch);
        }
        let expected: BTreeMap<_, _> = manifest["rollback_targets"]
            .as_array()
            .ok_or(ReleaseError::InvalidDocument)?
            .iter()
            .map(|v| (v["release_id"].as_str().unwrap_or_default(), v))
            .collect();
        let reports = attestation["rollback_reports"]
            .as_array()
            .ok_or(ReleaseError::InvalidDocument)?;
        if reports.len() != expected.len() || docs.rollback_reports.len() != expected.len() {
            return Err(ReleaseError::DigestMismatch);
        }
        for report in reports {
            let id = report["release_id"]
                .as_str()
                .ok_or(ReleaseError::InvalidDocument)?;
            let bytes = docs
                .rollback_reports
                .get(id)
                .ok_or(ReleaseError::DigestMismatch)?;
            if !expected.contains_key(id) || report["report_digest"] != digest(bytes) {
                return Err(ReleaseError::DigestMismatch);
            }
        }
        if manifest["target"]["platform"] != host.platform
            || manifest["target"]["architecture"] != host.architecture
            || feed.feed.platform != host.platform
            || feed.feed.architecture != host.architecture
            || manifest::os_version(host.os_version)?
                < manifest::os_version(
                    manifest["target"]["min_os"]
                        .as_str()
                        .ok_or(ReleaseError::InvalidDocument)?,
                )?
            || manifest::os_version(host.updater_version)?
                < manifest::os_version(
                    manifest["updater"]["min_version"]
                        .as_str()
                        .ok_or(ReleaseError::InvalidDocument)?,
                )?
            || manifest["updater"]["transaction_schema"] != host.transaction_schema
            || !manifest["updater"]["migration_requirements"]
                .as_array()
                .ok_or(ReleaseError::InvalidDocument)?
                .is_empty()
        {
            return Err(ReleaseError::Incompatible);
        }
        let host_major = manifest::os_version(host.os_version)?[0];
        if !manifest["target"]["tested_os"]
            .as_array()
            .ok_or(ReleaseError::InvalidDocument)?
            .iter()
            .any(|v| {
                v.as_str()
                    .and_then(|v| manifest::os_version(v).ok())
                    .map(|v| v[0])
                    == Some(host_major)
            })
        {
            return Err(ReleaseError::Incompatible);
        }
        if let Some(current) = &host.installed {
            if !valid_digest(current.manifest_digest) {
                return Err(ReleaseError::InvalidDocument);
            }
            manifest::validate_transition(current.manifest, &manifest)?;
            if current.manifest["release_id"] == manifest["release_id"]
                && current.manifest_digest != feed.feed.manifest_digest
            {
                return Err(ReleaseError::DigestMismatch);
            }
            if current.manifest["build"]
                .as_u64()
                .ok_or(ReleaseError::InvalidDocument)?
                >= manifest["build"]
                    .as_u64()
                    .ok_or(ReleaseError::InvalidDocument)?
                && current.manifest_digest != feed.feed.manifest_digest
            {
                let record = feed.feed.rollback.as_ref().ok_or(ReleaseError::Replay)?;
                if record.from_release_id != current.manifest["release_id"]
                    || record.from_manifest_digest != current.manifest_digest
                {
                    return Err(ReleaseError::Incompatible);
                }
                let target = current.manifest["rollback_targets"]
                    .as_array()
                    .ok_or(ReleaseError::InvalidDocument)?
                    .iter()
                    .find(|v| v["release_id"] == feed.feed.release_id)
                    .ok_or(ReleaseError::Incompatible)?;
                let actual: Vec<_> = manifest["distribution_artifacts"]
                    .as_array()
                    .ok_or(ReleaseError::InvalidDocument)?
                    .iter()
                    .map(|v| &v["sha256"])
                    .collect();
                let declared = target["artifact_digests"]
                    .as_array()
                    .ok_or(ReleaseError::InvalidDocument)?;
                if actual.len() != declared.len() || !actual.iter().all(|v| declared.contains(v)) {
                    return Err(ReleaseError::DigestMismatch);
                }
                if target["stores"] != manifest["stores"] {
                    return Err(ReleaseError::Incompatible);
                }
                let evidence = current
                    .rollback_evidence
                    .as_ref()
                    .ok_or(ReleaseError::Incompatible)?;
                let previous = self.verify_archive_policy(
                    evidence.attestation,
                    DocumentKind::Attestation,
                    evidence.archive_policy_id,
                    now,
                )?;
                manifest::validate_attestation(&previous)?;
                if previous["release_id"] != current.manifest["release_id"]
                    || previous["manifest_digest"] != current.manifest_digest
                    || record.report_digest != digest(evidence.report)
                    || !previous["rollback_reports"]
                        .as_array()
                        .ok_or(ReleaseError::InvalidDocument)?
                        .iter()
                        .any(|report| {
                            report["release_id"] == feed.feed.release_id
                                && report["report_digest"] == record.report_digest
                        })
                {
                    return Err(ReleaseError::DigestMismatch);
                }
            } else if feed.feed.rollback.is_some() {
                return Err(ReleaseError::Incompatible);
            }
        } else if feed.feed.rollback.is_some() {
            return Err(ReleaseError::Incompatible);
        }
        let native_policy = crate::native_policy::NativeReleasePolicy::from_manifest(&manifest)?;
        Ok(AuthorizedReleaseMetadata {
            manifest,
            attestation,
            feed: feed.clone(),
            native_policy,
        })
    }
}
