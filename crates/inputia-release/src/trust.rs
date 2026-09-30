use crate::{canonical, digest, utc, valid_id, ReleaseError, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use ring::signature::{UnparsedPublicKey, ECDSA_P256_SHA256_ASN1};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentKind {
    Manifest,
    Attestation,
    Feed,
    Keyset,
    Recovery,
    OfflineInstall,
}
impl DocumentKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Manifest => "manifest",
            Self::Attestation => "attestation",
            Self::Feed => "feed",
            Self::Keyset => "keyset",
            Self::Recovery => "recovery",
            Self::OfflineInstall => "offline_install",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Signature {
    pub key_id: String,
    pub algorithm: String,
    pub signature_der_base64: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub schema_version: u32,
    pub payload_kind: DocumentKind,
    pub payload: Value,
    pub signatures: Vec<Signature>,
}

impl Envelope {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let envelope: Self = serde_json::from_value(canonical::parse(bytes)?)
            .map_err(|_| ReleaseError::InvalidDocument)?;
        if envelope.schema_version != 1
            || !envelope.payload.is_object()
            || !(1..=if envelope.payload_kind == DocumentKind::Keyset {
                32
            } else {
                16
            })
                .contains(&envelope.signatures.len())
        {
            return Err(ReleaseError::InvalidDocument);
        }
        let mut keys = BTreeSet::new();
        for signature in &envelope.signatures {
            if !valid_id(&signature.key_id)
                || !keys.insert(&signature.key_id)
                || signature.algorithm != "ecdsa-p256-sha256"
                || signature.signature_der_base64.len() > 128
            {
                return Err(ReleaseError::InvalidDocument);
            }
            let _ = decode64(&signature.signature_der_base64)?;
        }
        Ok(envelope)
    }
}

/// domain 同时绑定版本与用途；pair v1/v2 的签名不能被此验证器接受。
pub fn signing_bytes(kind: DocumentKind, payload: &Value) -> Result<Vec<u8>> {
    let mut bytes = b"Inputia.Release.v1\0".to_vec();
    bytes.extend(kind.label().as_bytes());
    bytes.push(0);
    bytes.extend(canonical::encode(payload)?);
    Ok(bytes)
}

/// 链与序号使用已签语义身份，避免空白、签名顺序或等价 ECDSA 包装造成永久分叉。
pub fn semantic_digest(kind: DocumentKind, payload: &Value) -> Result<String> {
    Ok(digest(&signing_bytes(kind, payload)?))
}

fn decode64(value: &str) -> Result<Vec<u8>> {
    let bytes = STANDARD
        .decode(value)
        .map_err(|_| ReleaseError::InvalidDocument)?;
    if STANDARD.encode(&bytes) != value {
        return Err(ReleaseError::InvalidDocument);
    }
    Ok(bytes)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicKey {
    pub key_id: String,
    pub public_key_x963_base64: String,
}
impl PublicKey {
    fn bytes(&self) -> Result<Vec<u8>> {
        if !valid_id(&self.key_id) {
            return Err(ReleaseError::InvalidDocument);
        }
        let bytes = decode64(&self.public_key_x963_base64)?;
        if bytes.len() != 65 || bytes[0] != 4 || self.key_id != format!("sha256-{}", digest(&bytes))
        {
            return Err(ReleaseError::InvalidDocument);
        }
        Ok(bytes)
    }
}

fn verify_threshold(envelope: &Envelope, keys: &[PublicKey], threshold: usize) -> Result<()> {
    if threshold == 0 || threshold > keys.len() {
        return Err(ReleaseError::UntrustedKey);
    }
    let mut seen = BTreeSet::new();
    for key in keys {
        key.bytes()?;
        if !seen.insert(&key.key_id) {
            return Err(ReleaseError::InvalidDocument);
        }
    }
    let message = signing_bytes(envelope.payload_kind, &envelope.payload)?;
    let mut verified = 0;
    for signature in &envelope.signatures {
        if let Some(key) = keys.iter().find(|key| key.key_id == signature.key_id) {
            UnparsedPublicKey::new(&ECDSA_P256_SHA256_ASN1, key.bytes()?)
                .verify(&message, &decode64(&signature.signature_der_base64)?)
                .map_err(|_| ReleaseError::InvalidSignature)?;
            verified += 1;
        }
    }
    if verified < threshold {
        return Err(ReleaseError::UntrustedKey);
    }
    Ok(())
}

/// 根仅来自编译期信任锚或连续双阈值轮换，不从下载文件直接反序列化。
#[derive(Clone, Debug)]
pub struct TrustRoot {
    product_id: String,
    keys: Vec<PublicKey>,
    threshold: usize,
    version: u64,
    document_digest: Option<String>,
    revoked_key_ids: BTreeSet<String>,
    archive_policy_digests: BTreeMap<String, String>,
}
impl TrustRoot {
    pub fn from_embedded(product_id: &str, keys: Vec<PublicKey>, threshold: usize) -> Result<Self> {
        if product_id != "com.inputia"
            || keys.is_empty()
            || keys.len() > 16
            || threshold == 0
            || threshold > keys.len()
        {
            return Err(ReleaseError::UntrustedKey);
        }
        let mut ids = BTreeSet::new();
        for key in &keys {
            key.bytes()?;
            if !ids.insert(&key.key_id) {
                return Err(ReleaseError::InvalidDocument);
            }
        }
        Ok(Self {
            product_id: product_id.into(),
            keys,
            threshold,
            version: 0,
            document_digest: None,
            revoked_key_ids: BTreeSet::new(),
            archive_policy_digests: BTreeMap::new(),
        })
    }

    /// 旧根即使过期也只可验证连续的下一根；当前更新授权另有新鲜度检查。
    pub fn advance(&self, bytes: &[u8]) -> Result<VerifiedKeyset> {
        let envelope = Envelope::parse(bytes)?;
        if envelope.payload_kind != DocumentKind::Keyset {
            return Err(ReleaseError::WrongPurpose);
        }
        crate::schema::validate(
            &envelope.payload,
            &canonical::parse(include_bytes!("../../../release/schema/keyset.schema.json"))?,
        )?;
        if envelope.payload.get("previous_keyset_digest").is_none() {
            return Err(ReleaseError::InvalidDocument);
        }
        verify_threshold(&envelope, &self.keys, self.threshold)?;
        let payload: Keyset = serde_json::from_value(envelope.payload.clone())
            .map_err(|_| ReleaseError::InvalidDocument)?;
        payload.validate(&self.product_id)?;
        let document_digest = semantic_digest(DocumentKind::Keyset, &envelope.payload)?;
        let same = payload.version == self.version
            && self.document_digest.as_ref() == Some(&document_digest);
        if !same {
            if payload.version <= self.version {
                return Err(if payload.version == self.version {
                    ReleaseError::ConflictingSequence
                } else {
                    ReleaseError::Replay
                });
            }
            if payload.version != self.version + 1
                || payload.previous_keyset_digest != self.document_digest
            {
                return Err(ReleaseError::Replay);
            }
        }
        let revoked: BTreeSet<_> = payload
            .keys
            .iter()
            .filter(|key| key.status == KeyStatus::Revoked)
            .map(|key| key.key_id.clone())
            .collect();
        if !self.revoked_key_ids.is_subset(&revoked) {
            return Err(ReleaseError::UntrustedKey);
        }
        let archive_policy_digests: BTreeMap<_, _> = payload
            .archive_policies
            .iter()
            .map(|policy| {
                Ok((
                    policy.policy_id.clone(),
                    digest(&canonical::encode(
                        &serde_json::to_value(policy).map_err(|_| ReleaseError::InvalidDocument)?,
                    )?),
                ))
            })
            .collect::<Result<_>>()?;
        if self
            .archive_policy_digests
            .iter()
            .any(|(id, digest)| archive_policy_digests.get(id) != Some(digest))
        {
            return Err(ReleaseError::UntrustedKey);
        }
        let root_keys = payload.role_keys(&payload.roles.root)?;
        // 新根必须同时证明它拥有下一套 root 密钥，禁止只靠旧签名换成任意公钥。
        verify_threshold(&envelope, &root_keys, payload.roles.root.threshold)?;
        let next_root = TrustRoot {
            product_id: self.product_id.clone(),
            keys: root_keys,
            threshold: payload.roles.root.threshold,
            version: payload.version,
            document_digest: Some(document_digest.clone()),
            revoked_key_ids: revoked,
            archive_policy_digests,
        };
        Ok(VerifiedKeyset {
            payload,
            document_digest,
            next_root,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyStatus {
    Active,
    Retired,
    Revoked,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorizedKey {
    pub key_id: String,
    pub algorithm: String,
    pub public_key_x963_base64: String,
    pub status: KeyStatus,
    /// 在线授权时段；不将发行者自填时间当作历史签名时间证明。
    pub not_before: String,
    pub not_after: String,
}
impl AuthorizedKey {
    fn public(&self) -> PublicKey {
        PublicKey {
            key_id: self.key_id.clone(),
            public_key_x963_base64: self.public_key_x963_base64.clone(),
        }
    }
    fn active_at(&self, now: i64) -> Result<bool> {
        Ok(self.status == KeyStatus::Active
            && now >= utc(&self.not_before)?
            && now < utc(&self.not_after)?)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyRole {
    pub threshold: usize,
    pub key_ids: Vec<String>,
}
impl KeyRole {
    fn validate(&self, keys: &BTreeMap<&str, &AuthorizedKey>) -> Result<()> {
        if self.threshold == 0
            || self.threshold > self.key_ids.len()
            || self.key_ids.len() > 16
            || self.key_ids.iter().collect::<BTreeSet<_>>().len() != self.key_ids.len()
            || self
                .key_ids
                .iter()
                .any(|id| !keys.contains_key(id.as_str()))
        {
            return Err(ReleaseError::InvalidDocument);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Track {
    pub channel: String,
    pub platform: String,
    pub architecture: String,
}
impl Track {
    pub fn validate(&self) -> Result<()> {
        if !matches!(self.channel.as_str(), "candidate" | "stable")
            || self.platform != "macos"
            || self.architecture != "arm64"
        {
            return Err(ReleaseError::Incompatible);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedRole {
    pub track: Track,
    pub role: KeyRole,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Roles {
    pub root: KeyRole,
    pub manifest: KeyRole,
    pub attestation: KeyRole,
    pub feeds: Vec<FeedRole>,
}

/// 归档策略不可就地改变或移除。当前频道必须明确选择策略并绑定完整制品摘要。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchivePolicy {
    pub policy_id: String,
    pub manifest: KeyRole,
    pub attestation: KeyRole,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Keyset {
    pub schema_version: u32,
    pub product_id: String,
    pub version: u64,
    pub previous_keyset_digest: Option<String>,
    pub issued_at: String,
    pub expires_at: String,
    pub keys: Vec<AuthorizedKey>,
    pub roles: Roles,
    pub archive_policies: Vec<ArchivePolicy>,
}
impl Keyset {
    fn validate(&self, product: &str) -> Result<()> {
        if self.schema_version != 1
            || self.version == 0
            || self.version > 9_007_199_254_740_991
            || self.keys.is_empty()
            || self.keys.len() > 1024
            || self.roles.feeds.len() > 16
            || self
                .previous_keyset_digest
                .as_ref()
                .is_some_and(|v| !crate::valid_digest(v))
        {
            return Err(ReleaseError::InvalidDocument);
        }
        if self.product_id != product {
            return Err(ReleaseError::WrongProduct);
        }
        if utc(&self.expires_at)? <= utc(&self.issued_at)? {
            return Err(ReleaseError::Expired);
        }
        let mut keys = BTreeMap::new();
        for key in &self.keys {
            key.public().bytes()?;
            if key.algorithm != "ecdsa-p256-sha256"
                || keys.insert(key.key_id.as_str(), key).is_some()
                || utc(&key.not_after)? <= utc(&key.not_before)?
            {
                return Err(ReleaseError::InvalidDocument);
            }
        }
        self.roles.root.validate(&keys)?;
        self.roles.manifest.validate(&keys)?;
        self.roles.attestation.validate(&keys)?;
        if self.archive_policies.is_empty() || self.archive_policies.len() > 1024 {
            return Err(ReleaseError::InvalidDocument);
        }
        let mut policies = BTreeSet::new();
        for policy in &self.archive_policies {
            if !valid_id(&policy.policy_id) || !policies.insert(&policy.policy_id) {
                return Err(ReleaseError::InvalidDocument);
            }
            policy.manifest.validate(&keys)?;
            policy.attestation.validate(&keys)?;
            if self.roles.root.key_ids.iter().any(|id| {
                policy.manifest.key_ids.contains(id) || policy.attestation.key_ids.contains(id)
            }) {
                return Err(ReleaseError::WrongPurpose);
            }
        }
        let mut tracks = BTreeSet::new();
        let mut online_ids = BTreeSet::new();
        online_ids.extend(self.roles.manifest.key_ids.iter());
        online_ids.extend(self.roles.attestation.key_ids.iter());
        for feed in &self.roles.feeds {
            feed.track.validate()?;
            feed.role.validate(&keys)?;
            if !tracks.insert(&feed.track) {
                return Err(ReleaseError::InvalidDocument);
            }
            online_ids.extend(feed.role.key_ids.iter());
        }
        if self
            .roles
            .root
            .key_ids
            .iter()
            .any(|id| online_ids.contains(id) || keys[id.as_str()].status != KeyStatus::Active)
        {
            return Err(ReleaseError::UntrustedKey);
        }
        Ok(())
    }

    fn role_keys(&self, role: &KeyRole) -> Result<Vec<PublicKey>> {
        role.key_ids
            .iter()
            .map(|id| {
                self.keys
                    .iter()
                    .find(|key| key.key_id == *id)
                    .map(AuthorizedKey::public)
                    .ok_or(ReleaseError::UntrustedKey)
            })
            .collect()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeysetCheckpoint {
    pub product_id: String,
    pub version: u64,
    pub document_digest: String,
    pub revoked_key_ids: BTreeSet<String>,
}

/// 只能由已信任根构造；安装授权入口还会检查当前 keyset 的有效期和 feed 轨道。
#[derive(Clone, Debug)]
pub struct VerifiedKeyset {
    payload: Keyset,
    document_digest: String,
    next_root: TrustRoot,
}
impl VerifiedKeyset {
    pub fn checkpoint(&self) -> KeysetCheckpoint {
        KeysetCheckpoint {
            product_id: self.payload.product_id.clone(),
            version: self.payload.version,
            document_digest: self.document_digest.clone(),
            revoked_key_ids: self.next_root.revoked_key_ids.clone(),
        }
    }
    pub fn next_root(&self) -> &TrustRoot {
        &self.next_root
    }
    pub fn require_fresh(&self, now: i64) -> Result<()> {
        if now < utc(&self.payload.issued_at)? || now >= utc(&self.payload.expires_at)? {
            return Err(ReleaseError::Expired);
        }
        Ok(())
    }

    /// 归档正文仅在本模块内部流向完整授权链；不导出可冒充安装许可的通用验签 Bool。
    #[cfg(test)]
    pub(crate) fn verify_archive(
        &self,
        bytes: &[u8],
        kind: DocumentKind,
        now: i64,
    ) -> Result<Value> {
        let role = match kind {
            DocumentKind::Manifest => &self.payload.roles.manifest,
            DocumentKind::Attestation => &self.payload.roles.attestation,
            _ => return Err(ReleaseError::WrongPurpose),
        };
        self.verify_document(bytes, kind, role, now, true, None)
    }

    pub(crate) fn verify_archive_policy(
        &self,
        bytes: &[u8],
        kind: DocumentKind,
        policy_id: &str,
        now: i64,
    ) -> Result<Value> {
        let policy = self
            .payload
            .archive_policies
            .iter()
            .find(|p| p.policy_id == policy_id)
            .ok_or(ReleaseError::UntrustedKey)?;
        let role = match kind {
            DocumentKind::Manifest => &policy.manifest,
            DocumentKind::Attestation => &policy.attestation,
            _ => return Err(ReleaseError::WrongPurpose),
        };
        self.verify_document(bytes, kind, role, now, true, None)
    }

    pub(crate) fn verify_feed(
        &self,
        bytes: &[u8],
        track: &Track,
        now: i64,
        expires: i64,
    ) -> Result<Value> {
        track.validate()?;
        let role = self
            .payload
            .roles
            .feeds
            .iter()
            .find(|entry| &entry.track == track)
            .ok_or(ReleaseError::WrongPurpose)?;
        self.verify_document(
            bytes,
            DocumentKind::Feed,
            &role.role,
            now,
            false,
            Some(expires),
        )
    }

    fn verify_document(
        &self,
        bytes: &[u8],
        kind: DocumentKind,
        role: &KeyRole,
        now: i64,
        archive: bool,
        feed_expires: Option<i64>,
    ) -> Result<Value> {
        self.require_fresh(now)?;
        let envelope = Envelope::parse(bytes)?;
        if envelope.payload_kind != kind {
            return Err(ReleaseError::WrongPurpose);
        }
        if envelope.payload["product_id"] != self.payload.product_id {
            return Err(ReleaseError::WrongProduct);
        }
        let mut authorized = Vec::new();
        for id in &role.key_ids {
            let key = self
                .payload
                .keys
                .iter()
                .find(|key| &key.key_id == id)
                .ok_or(ReleaseError::UntrustedKey)?;
            if envelope
                .signatures
                .iter()
                .any(|signature| signature.key_id == *id)
                && key.status == KeyStatus::Revoked
            {
                return Err(ReleaseError::UntrustedKey);
            }
            let historical = archive && key.status == KeyStatus::Retired;
            let active = key.active_at(now)?
                && feed_expires
                    .map(|expires| utc(&key.not_after).map(|end| expires <= end))
                    .transpose()?
                    .unwrap_or(true);
            if historical || active {
                authorized.push(key.public());
            }
        }
        verify_threshold(&envelope, &authorized, role.threshold)?;
        Ok(envelope.payload)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ring::{
        rand::SystemRandom,
        signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_ASN1_SIGNING},
    };

    pub(crate) struct SigningKey {
        key: EcdsaKeyPair,
        pub(crate) public: PublicKey,
    }
    impl SigningKey {
        pub(crate) fn new() -> Self {
            let rng = SystemRandom::new();
            let pkcs8 =
                EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
            let key =
                EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng)
                    .unwrap();
            let public = PublicKey {
                key_id: format!("sha256-{}", digest(key.public_key().as_ref())),
                public_key_x963_base64: STANDARD.encode(key.public_key().as_ref()),
            };
            Self { key, public }
        }
        pub(crate) fn record(&self, status: KeyStatus) -> AuthorizedKey {
            AuthorizedKey {
                key_id: self.public.key_id.clone(),
                algorithm: "ecdsa-p256-sha256".into(),
                public_key_x963_base64: self.public.public_key_x963_base64.clone(),
                status,
                not_before: "2026-01-01T00:00:00Z".into(),
                not_after: "2027-01-01T00:00:00Z".into(),
            }
        }
    }
    pub(crate) fn signed(kind: DocumentKind, value: Value, keys: &[&SigningKey]) -> Vec<u8> {
        let bytes = signing_bytes(kind, &value).unwrap();
        let signatures = keys
            .iter()
            .map(|key| Signature {
                key_id: key.public.key_id.clone(),
                algorithm: "ecdsa-p256-sha256".into(),
                signature_der_base64: STANDARD
                    .encode(key.key.sign(&SystemRandom::new(), &bytes).unwrap().as_ref()),
            })
            .collect();
        serde_json::to_vec(&Envelope {
            schema_version: 1,
            payload_kind: kind,
            payload: value,
            signatures,
        })
        .unwrap()
    }
    pub(crate) fn role(key: &SigningKey) -> KeyRole {
        KeyRole {
            threshold: 1,
            key_ids: vec![key.public.key_id.clone()],
        }
    }
    pub(crate) fn candidate() -> Track {
        Track {
            channel: "candidate".into(),
            platform: "macos".into(),
            architecture: "arm64".into(),
        }
    }
    pub(crate) fn payload(root: &SigningKey, archive: &SigningKey, feed: &SigningKey) -> Keyset {
        Keyset {
            schema_version: 1,
            product_id: "com.inputia".into(),
            version: 1,
            previous_keyset_digest: None,
            issued_at: "2026-01-01T00:00:00Z".into(),
            expires_at: "2027-01-01T00:00:00Z".into(),
            keys: vec![
                root.record(KeyStatus::Active),
                archive.record(KeyStatus::Active),
                feed.record(KeyStatus::Active),
            ],
            archive_policies: vec![ArchivePolicy {
                policy_id: "release-2026".into(),
                manifest: role(archive),
                attestation: role(archive),
            }],
            roles: Roles {
                root: role(root),
                manifest: role(archive),
                attestation: role(archive),
                feeds: vec![FeedRole {
                    track: candidate(),
                    role: role(feed),
                }],
            },
        }
    }
    pub(crate) fn now() -> i64 {
        utc("2026-09-30T00:00:00Z").unwrap()
    }

    #[test]
    fn envelope_variants_share_chain_identity_and_keep_future_root_rotatable() {
        let (root, archive, online, unknown) = (
            SigningKey::new(),
            SigningKey::new(),
            SigningKey::new(),
            SigningKey::new(),
        );
        let embedded =
            TrustRoot::from_embedded("com.inputia", vec![root.public.clone()], 1).unwrap();
        let original = payload(&root, &archive, &online);
        let bytes = signed(
            DocumentKind::Keyset,
            serde_json::to_value(&original).unwrap(),
            &[&root],
        );
        let with_extra = signed(
            DocumentKind::Keyset,
            serde_json::to_value(&original).unwrap(),
            &[&unknown, &root],
        );
        let pretty = serde_json::to_vec_pretty(&Envelope::parse(&with_extra).unwrap()).unwrap();
        let a = embedded.advance(&bytes).unwrap();
        let b = embedded.advance(&pretty).unwrap();
        assert_eq!(
            a.checkpoint().document_digest,
            b.checkpoint().document_digest
        );
        b.next_root().advance(&bytes).unwrap();
        let mut next = original;
        next.version = 2;
        next.previous_keyset_digest = Some(a.checkpoint().document_digest);
        let bytes = signed(
            DocumentKind::Keyset,
            serde_json::to_value(next).unwrap(),
            &[&root],
        );
        b.next_root().advance(&bytes).unwrap();
    }

    #[test]
    fn archive_policy_survives_online_threshold_change_and_cannot_be_rewritten() {
        let (root, archive, online, new_archive) = (
            SigningKey::new(),
            SigningKey::new(),
            SigningKey::new(),
            SigningKey::new(),
        );
        let embedded =
            TrustRoot::from_embedded("com.inputia", vec![root.public.clone()], 1).unwrap();
        let mut current = payload(&root, &archive, &online);
        let first = embedded
            .advance(&signed(
                DocumentKind::Keyset,
                serde_json::to_value(&current).unwrap(),
                &[&root],
            ))
            .unwrap();
        let old = signed(
            DocumentKind::Manifest,
            serde_json::json!({"product_id":"com.inputia"}),
            &[&archive],
        );
        current.version = 2;
        current.previous_keyset_digest = Some(first.checkpoint().document_digest);
        current.keys[1].status = KeyStatus::Retired;
        current.keys.push(new_archive.record(KeyStatus::Active));
        current.roles.manifest = KeyRole {
            threshold: 2,
            key_ids: vec![
                archive.public.key_id.clone(),
                new_archive.public.key_id.clone(),
            ],
        };
        let next = first
            .next_root()
            .advance(&signed(
                DocumentKind::Keyset,
                serde_json::to_value(&current).unwrap(),
                &[&root],
            ))
            .unwrap();
        next.verify_archive_policy(&old, DocumentKind::Manifest, "release-2026", now())
            .unwrap();
        assert!(next
            .verify_archive(&old, DocumentKind::Manifest, now())
            .is_err());
        current.archive_policies[0].manifest = role(&new_archive);
        assert!(first
            .next_root()
            .advance(&signed(
                DocumentKind::Keyset,
                serde_json::to_value(&current).unwrap(),
                &[&root]
            ))
            .is_err());
    }

    #[test]
    fn disjoint_nine_key_threshold_rotation_accepts_both_sets_and_rejects_missing_vote() {
        let old: Vec<_> = (0..9).map(|_| SigningKey::new()).collect();
        let new: Vec<_> = (0..9).map(|_| SigningKey::new()).collect();
        let (archive, online) = (SigningKey::new(), SigningKey::new());
        let embedded = TrustRoot::from_embedded(
            "com.inputia",
            old.iter().map(|k| k.public.clone()).collect(),
            9,
        )
        .unwrap();
        let mut next = payload(&new[0], &archive, &online);
        next.keys
            .extend(new.iter().skip(1).map(|k| k.record(KeyStatus::Active)));
        next.roles.root = KeyRole {
            threshold: 9,
            key_ids: new.iter().map(|k| k.public.key_id.clone()).collect(),
        };
        let signers: Vec<_> = old.iter().chain(&new).collect();
        let raw = serde_json::to_value(next).unwrap();
        embedded
            .advance(&signed(DocumentKind::Keyset, raw.clone(), &signers))
            .unwrap();
        assert!(embedded
            .advance(&signed(DocumentKind::Keyset, raw, &signers[..17]))
            .is_err());
    }

    #[test]
    fn root_rotation_needs_both_thresholds_contiguous_version_and_previous_digest() {
        let (old, new, archive, feed) = (
            SigningKey::new(),
            SigningKey::new(),
            SigningKey::new(),
            SigningKey::new(),
        );
        let root = TrustRoot::from_embedded("com.inputia", vec![old.public.clone()], 1).unwrap();
        let first = root
            .advance(&signed(
                DocumentKind::Keyset,
                serde_json::to_value(payload(&old, &archive, &feed)).unwrap(),
                &[&old],
            ))
            .unwrap();
        let mut next = payload(&new, &archive, &feed);
        next.version = 2;
        next.previous_keyset_digest = Some(first.document_digest.clone());
        for keys in [vec![&old], vec![&new]] {
            assert!(first
                .next_root()
                .advance(&signed(
                    DocumentKind::Keyset,
                    serde_json::to_value(&next).unwrap(),
                    &keys
                ))
                .is_err());
        }
        let bytes = signed(
            DocumentKind::Keyset,
            serde_json::to_value(&next).unwrap(),
            &[&old, &new],
        );
        let verified = first.next_root().advance(&bytes).unwrap();
        assert_eq!(verified.checkpoint().version, 2);
        assert!(verified.next_root().advance(&bytes).is_ok());
        next.version = 3;
        assert!(first
            .next_root()
            .advance(&signed(
                DocumentKind::Keyset,
                serde_json::to_value(&next).unwrap(),
                &[&old, &new]
            ))
            .is_err());
        next.version = 2;
        next.previous_keyset_digest = Some("a".repeat(64));
        assert!(first
            .next_root()
            .advance(&signed(
                DocumentKind::Keyset,
                serde_json::to_value(&next).unwrap(),
                &[&old, &new]
            ))
            .is_err());
    }

    #[test]
    fn expired_intermediate_root_can_only_advance_to_fresh_keyset() {
        let (rootkey, archive, feed) = (SigningKey::new(), SigningKey::new(), SigningKey::new());
        let root =
            TrustRoot::from_embedded("com.inputia", vec![rootkey.public.clone()], 1).unwrap();
        let mut value = payload(&rootkey, &archive, &feed);
        value.expires_at = "2026-02-01T00:00:00Z".into();
        let expired = root
            .advance(&signed(
                DocumentKind::Keyset,
                serde_json::to_value(&value).unwrap(),
                &[&rootkey],
            ))
            .unwrap();
        assert_eq!(expired.require_fresh(now()), Err(ReleaseError::Expired));
        value.version = 2;
        value.previous_keyset_digest = Some(expired.document_digest.clone());
        value.expires_at = "2027-01-01T00:00:00Z".into();
        let current = expired
            .next_root()
            .advance(&signed(
                DocumentKind::Keyset,
                serde_json::to_value(value).unwrap(),
                &[&rootkey],
            ))
            .unwrap();
        assert!(current.require_fresh(now()).is_ok());
    }

    #[test]
    fn retired_key_only_verifies_archive_not_feed_and_revocation_is_monotonic() {
        let (rootkey, archive, feed) = (SigningKey::new(), SigningKey::new(), SigningKey::new());
        let root =
            TrustRoot::from_embedded("com.inputia", vec![rootkey.public.clone()], 1).unwrap();
        let mut value = payload(&rootkey, &archive, &feed);
        value.keys[1].status = KeyStatus::Retired;
        value.keys[1].not_after = "2026-02-01T00:00:00Z".into();
        value.roles.feeds[0].role = role(&archive);
        let verified = root
            .advance(&signed(
                DocumentKind::Keyset,
                serde_json::to_value(&value).unwrap(),
                &[&rootkey],
            ))
            .unwrap();
        let body = serde_json::json!({"product_id":"com.inputia"});
        assert!(verified
            .verify_archive(
                &signed(DocumentKind::Manifest, body.clone(), &[&archive]),
                DocumentKind::Manifest,
                now()
            )
            .is_ok());
        assert!(verified
            .verify_feed(
                &signed(DocumentKind::Feed, body.clone(), &[&archive]),
                &candidate(),
                now(),
                now() + 60
            )
            .is_err());
        value.version = 2;
        value.previous_keyset_digest = Some(verified.document_digest.clone());
        value.keys[1].status = KeyStatus::Revoked;
        let revoked = verified
            .next_root()
            .advance(&signed(
                DocumentKind::Keyset,
                serde_json::to_value(&value).unwrap(),
                &[&rootkey],
            ))
            .unwrap();
        assert!(revoked
            .verify_archive(
                &signed(DocumentKind::Manifest, body, &[&archive]),
                DocumentKind::Manifest,
                now()
            )
            .is_err());
        value.version = 3;
        value.previous_keyset_digest = Some(revoked.document_digest.clone());
        value.keys[1].status = KeyStatus::Retired;
        assert!(revoked
            .next_root()
            .advance(&signed(
                DocumentKind::Keyset,
                serde_json::to_value(value).unwrap(),
                &[&rootkey]
            ))
            .is_err());
    }

    #[test]
    fn signatures_bind_kind_product_and_distinct_public_key_fingerprint() {
        let (rootkey, archive, feed) = (SigningKey::new(), SigningKey::new(), SigningKey::new());
        let mut alias = rootkey.public.clone();
        alias.key_id = "sha256-alias".into();
        assert!(
            TrustRoot::from_embedded("com.inputia", vec![rootkey.public.clone(), alias], 2)
                .is_err()
        );
        let root =
            TrustRoot::from_embedded("com.inputia", vec![rootkey.public.clone()], 1).unwrap();
        let verified = root
            .advance(&signed(
                DocumentKind::Keyset,
                serde_json::to_value(payload(&rootkey, &archive, &feed)).unwrap(),
                &[&rootkey],
            ))
            .unwrap();
        let mut envelope = Envelope::parse(&signed(
            DocumentKind::Feed,
            serde_json::json!({"product_id":"com.inputia"}),
            &[&archive],
        ))
        .unwrap();
        envelope.payload_kind = DocumentKind::Manifest;
        assert_eq!(
            verified.verify_archive(
                &serde_json::to_vec(&envelope).unwrap(),
                DocumentKind::Manifest,
                now()
            ),
            Err(ReleaseError::InvalidSignature)
        );
        envelope.signatures.push(envelope.signatures[0].clone());
        assert!(Envelope::parse(&serde_json::to_vec(&envelope).unwrap()).is_err());
        assert!(verified
            .verify_archive(
                &signed(
                    DocumentKind::Manifest,
                    serde_json::json!({"product_id":"other"}),
                    &[&archive]
                ),
                DocumentKind::Manifest,
                now()
            )
            .is_err());
        let wrong_track = Track {
            channel: "stable".into(),
            ..candidate()
        };
        assert!(verified
            .verify_feed(
                &signed(
                    DocumentKind::Feed,
                    serde_json::json!({"product_id":"com.inputia"}),
                    &[&feed]
                ),
                &wrong_track,
                now(),
                now() + 60
            )
            .is_err());
    }
}
