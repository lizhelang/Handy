//! 唯一更新目录流程：受限下载 → 耐久根链/频道 → 历史安装与目标元数据验证。
//! 此处不授予安装权；可用更新还须经过原生配套事务预检。
use crate::{
    canonical, digest,
    feed::{AuthorizedReleaseMetadata, Feed, Host, InstalledRelease, ReleaseDocuments},
    manifest,
    state::TrustStore,
    trust::{DocumentKind, Envelope, PublicKey, Track, TrustRoot},
    valid_digest, ReleaseError,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, future::Future, path::Path};

pub const DOCUMENT_LIMIT: usize = 4 * 1024 * 1024;
const CHECK_BUDGET: usize = 32 * 1024 * 1024;
const MAX_ROTATIONS: u64 = 32;
const MAX_REPORTS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckError {
    Network,
    BudgetExceeded,
    TrustRefreshRequired,
    InstalledMetadataMissing,
    InvalidInstalledMetadata,
    Release(ReleaseError),
}
impl From<ReleaseError> for CheckError {
    fn from(value: ReleaseError) -> Self {
        Self::Release(value)
    }
}
pub type CheckResult<T> = std::result::Result<T, CheckError>;

/// 只由构建期内置配置创建；不能从下载文档、设置或环境变量赋值。
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmbeddedCatalog {
    pub product_id: String,
    pub base_url: String,
    pub redirect_origins: Vec<String>,
    pub root_keys: Vec<PublicKey>,
    pub root_threshold: usize,
}
impl EmbeddedCatalog {
    pub fn trust_root(&self) -> crate::Result<TrustRoot> {
        TrustRoot::from_embedded(
            &self.product_id,
            self.root_keys.clone(),
            self.root_threshold,
        )
    }
}

/// 源仅收到固定相对路径，不能收到历史、输入内容或本机安装 UUID。
/// 适配器必须限制 HTTPS 来源、重定向、时间与响应体大小。
pub trait DocumentSource {
    fn fetch(
        &self,
        relative: &str,
        limit: usize,
    ) -> impl Future<Output = CheckResult<Vec<u8>>> + Send;
}
pub trait Clock {
    fn now(&self) -> i64;
}

/// 来自固定安装路径的原始签名文件；release_id 必须来自编译身份与安装收据。
pub struct InstalledCatalog<'a> {
    pub release_id: &'a str,
    pub manifest_digest: &'a str,
    pub archive_policy_id: &'a str,
    pub signed_manifest: &'a [u8],
}
pub struct CheckContext<'a> {
    pub track: &'a Track,
    pub os_version: &'a str,
    pub updater_version: &'a str,
    pub transaction_schema: u64,
    pub installed: InstalledCatalog<'a>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CatalogStatus {
    Current,
    Available,
}
/// 仅目录检查结果。调用安装必须重验频道新鲜度、字节及原生条件，不能持久化为许可。
pub struct CatalogCheck {
    pub status: CatalogStatus,
    pub metadata: AuthorizedReleaseMetadata,
}

struct Budget(usize);
impl Budget {
    async fn read(&mut self, source: &impl DocumentSource, path: &str) -> CheckResult<Vec<u8>> {
        crate::safe_relative(path)?;
        let limit = self.0.min(DOCUMENT_LIMIT);
        if limit == 0 {
            return Err(CheckError::BudgetExceeded);
        }
        let bytes = source.fetch(path, limit).await?;
        // 不相信适配器或测试替身会遵守 limit。
        if bytes.is_empty() || bytes.len() > limit {
            return Err(CheckError::BudgetExceeded);
        }
        self.0 -= bytes.len();
        Ok(bytes)
    }
    async fn digest_read(
        &mut self,
        source: &impl DocumentSource,
        path: &str,
        expected: &str,
    ) -> CheckResult<Vec<u8>> {
        if !valid_digest(expected) {
            return Err(ReleaseError::InvalidDocument.into());
        }
        let bytes = self.read(source, path).await?;
        if digest(&bytes) != expected {
            return Err(ReleaseError::DigestMismatch.into());
        }
        Ok(bytes)
    }
}

/// 全流程无长网络锁：每次重新打开都重验持久根链；最后重新核实当前频道高水位。
/// 中途取消只可能保留已验证根/频道，不会产生安装或“已经最新”的结果。
pub async fn check_catalog(
    home: &Path,
    embedded_root: TrustRoot,
    source: &impl DocumentSource,
    context: &CheckContext<'_>,
    clock: &impl Clock,
) -> CheckResult<CatalogCheck> {
    context.track.validate()?;
    if context.installed.signed_manifest.is_empty() {
        return Err(CheckError::InstalledMetadataMissing);
    }
    if context.installed.signed_manifest.len() > DOCUMENT_LIMIT
        || !valid_digest(context.installed.manifest_digest)
        || digest(context.installed.signed_manifest) != context.installed.manifest_digest
    {
        return Err(CheckError::InvalidInstalledMetadata);
    }
    let mut budget = Budget(CHECK_BUDGET);
    let feed_path = format!(
        "channels/{}/{}/{}.json",
        context.track.channel, context.track.platform, context.track.architecture
    );
    let raw_feed = budget.read(source, &feed_path).await?;
    let envelope = Envelope::parse(&raw_feed)?;
    if envelope.payload_kind != DocumentKind::Feed {
        return Err(ReleaseError::WrongPurpose.into());
    }
    // 未验签内容仅决定有界根版本提示；绝不授权路径、频道或安装。
    let hint: Feed =
        serde_json::from_value(envelope.payload).map_err(|_| ReleaseError::InvalidDocument)?;
    if hint.track() != *context.track || !(1..=1024).contains(&hint.keyset_version) {
        return Err(ReleaseError::InvalidDocument.into());
    }
    let existing = {
        let store = TrustStore::open(home, embedded_root.clone())?;
        store
            .current_keyset()
            .map(|keyset| keyset.checkpoint().version)
            .unwrap_or(0)
    };
    if hint.keyset_version < existing {
        return Err(ReleaseError::Replay.into());
    }
    let end = hint.keyset_version.min(existing + MAX_ROTATIONS);
    for version in existing + 1..=end {
        let bytes = budget
            .read(source, &format!("keysets/{version}.json"))
            .await?;
        let mut store = TrustStore::open(home, embedded_root.clone())?;
        let current = store
            .current_keyset()
            .map(|keyset| keyset.checkpoint().version)
            .unwrap_or(0);
        if current < version {
            let next = store.advance_keyset(&bytes)?;
            if next.checkpoint().version != version {
                return Err(ReleaseError::InvalidDocument.into());
            }
        }
    }
    if end < hint.keyset_version {
        return Err(CheckError::TrustRefreshRequired);
    }
    let committed = {
        let mut store = TrustStore::open(home, embedded_root.clone())?;
        store.commit_feed(&raw_feed, context.track, clock.now())?
    };
    let feed = committed.verified().payload();
    let prefix = &feed.release_path;
    let manifest = budget
        .digest_read(
            source,
            &format!("{prefix}/release-manifest.json"),
            &feed.manifest_digest,
        )
        .await?;
    let attestation = budget
        .digest_read(
            source,
            &format!("{prefix}/release-attestation.json"),
            &feed.attestation_digest,
        )
        .await?;
    let proof = Envelope::parse(&attestation)?;
    if proof.payload_kind != DocumentKind::Attestation {
        return Err(ReleaseError::WrongPurpose.into());
    }
    manifest::validate_attestation(&proof.payload)?;
    let report_digest = proof.payload["acceptance_report_digest"]
        .as_str()
        .ok_or(ReleaseError::InvalidDocument)?;
    let acceptance = budget
        .digest_read(
            source,
            &format!("{prefix}/reports/{report_digest}.json"),
            report_digest,
        )
        .await?;
    let reports = proof.payload["rollback_reports"]
        .as_array()
        .ok_or(ReleaseError::InvalidDocument)?;
    if reports.len() > MAX_REPORTS {
        return Err(CheckError::BudgetExceeded);
    }
    let mut rollback = BTreeMap::new();
    for report in reports {
        let id = report["release_id"]
            .as_str()
            .ok_or(ReleaseError::InvalidDocument)?;
        let sha = report["report_digest"]
            .as_str()
            .ok_or(ReleaseError::InvalidDocument)?;
        let bytes = budget
            .digest_read(source, &format!("{prefix}/reports/{sha}.json"), sha)
            .await?;
        if rollback.insert(id.to_owned(), bytes).is_some() {
            return Err(ReleaseError::InvalidDocument.into());
        }
    }
    let mut store = TrustStore::open(home, embedded_root)?;
    let now = store.observe_time(clock.now())?;
    let keyset = store.current_keyset().ok_or(ReleaseError::UntrustedKey)?;
    // 验证旧清单只为确定逐库兼容基线，不赋予旧制品新的安装权。
    let installed = keyset
        .verify_archive_policy(
            context.installed.signed_manifest,
            DocumentKind::Manifest,
            context.installed.archive_policy_id,
            now,
        )
        .map_err(|_| CheckError::InvalidInstalledMetadata)?;
    manifest::validate_manifest(&installed).map_err(|_| CheckError::InvalidInstalledMetadata)?;
    if installed["release_id"] != context.installed.release_id {
        return Err(CheckError::InvalidInstalledMetadata);
    }
    let host = Host {
        platform: &context.track.platform,
        architecture: &context.track.architecture,
        os_version: context.os_version,
        updater_version: context.updater_version,
        transaction_schema: context.transaction_schema,
        installed: Some(InstalledRelease {
            manifest: &installed,
            manifest_digest: context.installed.manifest_digest,
            rollback_evidence: None,
        }),
    };
    // 降级需要旧版本冻结的报告，普通检查入口不给它补造许可。
    let metadata = store.authorize_release_metadata(
        &committed,
        &ReleaseDocuments {
            manifest: &manifest,
            attestation: &attestation,
            acceptance_report: &acceptance,
            rollback_reports: rollback
                .iter()
                .map(|(id, raw)| (id.clone(), raw.as_slice()))
                .collect(),
        },
        &host,
        clock.now(),
    )?;
    let status = if metadata.manifest()["release_id"] == context.installed.release_id {
        CatalogStatus::Current
    } else {
        CatalogStatus::Available
    };
    Ok(CatalogCheck { status, metadata })
}

/// 构建输入也使用严格 JSON；开发构建显式 source=null，不回退到上游地址。
pub fn embedded_config(bytes: &[u8]) -> crate::Result<Option<EmbeddedCatalog>> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Config {
        schema_version: u32,
        source: Option<EmbeddedCatalog>,
    }
    let value = canonical::parse(bytes)?;
    if value.get("source").is_none() {
        return Err(ReleaseError::InvalidDocument);
    }
    let config: Config =
        serde_json::from_value(value).map_err(|_| ReleaseError::InvalidDocument)?;
    if config.schema_version != 1 {
        return Err(ReleaseError::InvalidDocument);
    }
    if let Some(source) = &config.source {
        source.trust_root()?;
    }
    Ok(config.source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trust::tests::{candidate, now, payload, signed, SigningKey};
    use serde_json::json;
    use std::{
        sync::{
            atomic::{AtomicI64, Ordering},
            Arc, Mutex,
        },
        task::{Context, Poll, Waker},
    };
    struct TestClock(Arc<AtomicI64>);
    impl Clock for TestClock {
        fn now(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
    }
    struct Source {
        files: BTreeMap<String, Vec<u8>>,
        calls: Mutex<Vec<String>>,
        expire: Option<(Arc<AtomicI64>, i64)>,
    }
    impl DocumentSource for Source {
        async fn fetch(&self, relative: &str, _limit: usize) -> CheckResult<Vec<u8>> {
            self.calls.lock().unwrap().push(relative.into());
            if relative.contains("/reports/") {
                if let Some(clock) = &self.expire {
                    clock.0.store(clock.1, Ordering::SeqCst);
                }
            }
            self.files.get(relative).cloned().ok_or(CheckError::Network)
        }
    }
    fn run<T>(future: impl Future<Output = T>) -> T {
        let mut context = Context::from_waker(Waker::noop());
        match std::pin::pin!(future).as_mut().poll(&mut context) {
            Poll::Ready(result) => result,
            Poll::Pending => panic!("合成源应当立即返回"),
        }
    }
    struct Fixture {
        root: TrustRoot,
        source: Source,
        installed: Vec<u8>,
        installed_digest: String,
        clock: TestClock,
        home: tempfile::TempDir,
    }
    impl Fixture {
        fn new(newer: bool) -> Self {
            let (root, archive, online) = (SigningKey::new(), SigningKey::new(), SigningKey::new());
            let embedded =
                TrustRoot::from_embedded("com.inputia", vec![root.public.clone()], 1).unwrap();
            let keyset = signed(
                DocumentKind::Keyset,
                serde_json::to_value(payload(&root, &archive, &online)).unwrap(),
                &[&root],
            );
            let verified = embedded.advance(&keyset).unwrap();
            let mut value =
                canonical::parse(include_bytes!("../tests/fixtures/manifest.json")).unwrap();
            let installed = signed(DocumentKind::Manifest, value.clone(), &[&archive]);
            if newer {
                value["build"] = json!(85);
                value["release_id"] = json!("inputia-test-newer");
            }
            let id = value["release_id"].as_str().unwrap().to_owned();
            let manifest = if newer {
                signed(DocumentKind::Manifest, value, &[&archive])
            } else {
                installed.clone()
            };
            let attestation = signed(
                DocumentKind::Attestation,
                json!({"schema_version":1,"product_id":"com.inputia",
                "release_id":id,"manifest_digest":digest(&manifest),"acceptance_report_digest":digest(b"acceptance"),
                "rollback_reports":[{"release_id":"inputia-test-compatible","report_digest":digest(b"rollback")}],
                "issued_at":"2026-09-29T00:00:00Z"}),
                &[&archive],
            );
            let feed = signed(
                DocumentKind::Feed,
                json!({"schema_version":2,"product_id":"com.inputia",
                "channel":"candidate","platform":"macos","architecture":"arm64","sequence":5,
                "keyset_version":1,"keyset_digest":verified.checkpoint().document_digest,"archive_policy_id":"release-2026",
                "issued_at":"2026-09-30T00:00:00Z","expires_at":"2026-10-02T00:00:00Z","release_id":id,
                "manifest_digest":digest(&manifest),"attestation_digest":digest(&attestation),"release_path":format!("releases/{id}"),"rollback":null}),
                &[&online],
            );
            let source = Source {
                files: BTreeMap::from([
                    ("channels/candidate/macos/arm64.json".into(), feed),
                    ("keysets/1.json".into(), keyset),
                    (format!("releases/{id}/release-manifest.json"), manifest),
                    (
                        format!("releases/{id}/release-attestation.json"),
                        attestation,
                    ),
                    (
                        format!("releases/{id}/reports/{}.json", digest(b"acceptance")),
                        b"acceptance".to_vec(),
                    ),
                    (
                        format!("releases/{id}/reports/{}.json", digest(b"rollback")),
                        b"rollback".to_vec(),
                    ),
                ]),
                calls: Mutex::new(vec![]),
                expire: None,
            };
            Self {
                root: embedded,
                installed_digest: digest(&installed),
                installed,
                source,
                clock: TestClock(Arc::new(AtomicI64::new(now()))),
                home: tempfile::tempdir().unwrap(),
            }
        }
        fn check(&self) -> CheckResult<CatalogCheck> {
            run(check_catalog(
                &self.home.path().canonicalize().unwrap(),
                self.root.clone(),
                &self.source,
                &CheckContext {
                    track: &candidate(),
                    os_version: "26.0",
                    updater_version: "1.0.0",
                    transaction_schema: 1,
                    installed: InstalledCatalog {
                        release_id: "inputia-test-current",
                        manifest_digest: &self.installed_digest,
                        archive_policy_id: "release-2026",
                        signed_manifest: &self.installed,
                    },
                },
                &self.clock,
            ))
        }
    }
    #[test]
    fn current_and_available_require_complete_signature_chain_and_installed_baseline() {
        for newer in [false, true] {
            let fixture = Fixture::new(newer);
            assert_eq!(
                fixture.check().unwrap().status,
                if newer {
                    CatalogStatus::Available
                } else {
                    CatalogStatus::Current
                }
            );
            assert_eq!(fixture.source.calls.lock().unwrap().len(), 6);
            // 新服务实例读取耐久根；重复检查无需重新下载旧根。
            assert!(fixture.check().is_ok());
            assert_eq!(
                fixture
                    .source
                    .calls
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|path| *path == "keysets/1.json")
                    .count(),
                1
            );
        }
    }
    #[test]
    fn incomplete_or_wrong_installed_bytes_never_become_first_install() {
        let mut fixture = Fixture::new(true);
        fixture.installed.clear();
        assert!(matches!(
            fixture.check(),
            Err(CheckError::InstalledMetadataMissing)
        ));
        assert!(fixture.source.calls.lock().unwrap().is_empty());
        fixture.installed = b"{}".to_vec();
        assert!(matches!(
            fixture.check(),
            Err(CheckError::InvalidInstalledMetadata)
        ));
        assert!(fixture.source.calls.lock().unwrap().is_empty());
    }
    #[test]
    fn digest_failure_and_offline_never_report_current() {
        let mut fixture = Fixture::new(false);
        let path = "releases/inputia-test-current/release-manifest.json";
        fixture.source.files.get_mut(path).unwrap().push(b' ');
        assert!(matches!(
            fixture.check(),
            Err(CheckError::Release(ReleaseError::DigestMismatch))
        ));
        fixture.source.files.remove(path);
        assert!(matches!(fixture.check(), Err(CheckError::Network)));
    }
    #[test]
    fn expires_during_download_requires_another_fresh_feed() {
        for days in [10, 400] {
            let mut fixture = Fixture::new(true);
            fixture.source.expire = Some((fixture.clock.0.clone(), now() + days * 86400));
            assert!(matches!(
                fixture.check(),
                Err(CheckError::Release(ReleaseError::Expired))
                    | Err(CheckError::InvalidInstalledMetadata)
            ));
            fixture.clock.0.store(now(), Ordering::SeqCst);
            fixture.source.expire = None;
            assert!(matches!(
                fixture.check(),
                Err(CheckError::Release(ReleaseError::Expired))
            ));
        }
    }

    #[test]
    fn untrusted_hint_and_oversize_body_have_bounded_effects() {
        let mut fixture = Fixture::new(true);
        let path = "channels/candidate/macos/arm64.json";
        let mut raw = canonical::parse(fixture.source.files.get(path).unwrap()).unwrap();
        raw["payload"]["keyset_version"] = json!(1025);
        fixture
            .source
            .files
            .insert(path.into(), serde_json::to_vec(&raw).unwrap());
        assert!(matches!(
            fixture.check(),
            Err(CheckError::Release(ReleaseError::InvalidDocument))
        ));
        assert_eq!(fixture.source.calls.lock().unwrap().len(), 1);
        fixture
            .source
            .files
            .insert(path.into(), vec![b'x'; DOCUMENT_LIMIT + 1]);
        assert!(matches!(fixture.check(), Err(CheckError::BudgetExceeded)));
        assert_eq!(fixture.source.calls.lock().unwrap().len(), 2);
    }
    #[test]
    fn missing_source_is_explicit_and_duplicate_keys_are_rejected() {
        assert!(embedded_config(br#"{"schema_version":1,"source":null}"#)
            .unwrap()
            .is_none());
        assert!(embedded_config(br#"{"schema_version":1}"#).is_err());
        assert!(embedded_config(br#"{"schema_version":1,"source":null,"source":null}"#).is_err());
    }
}
