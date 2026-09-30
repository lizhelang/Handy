//! 版本化设置的耐久写入核心。版本、值与有限幂等回执在同一文档原子提交。
#[cfg(target_os = "macos")]
pub mod application;
mod external;
mod files;
use crate::{installation::valid_uuid, maintenance, InputiaSettings};
pub use external::{ExternalSnapshot, ImportRequest};
use files::{Boundary, Files};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

const LIMIT: usize = 512 * 1024;
const RECEIPT_LIMIT: usize = 256;
const META: &str = "_inputia_store";
const MARKER: &str = ".inputia-settings-initialized.json";
fn input_settings_domain() -> String {
    "inputia.basic-input".into()
}
fn is_input_settings_domain(domain: &String) -> bool {
    domain == InputSettingsSchema::DOMAIN
}

/// 领域适配器只定义格式；锁、CAS、幂等、原子提交和外部导入均由共享核心负责。
/// 路径名及领域 ID 必须是编译期固定常量，不能从请求或待导入文件读取。
pub trait DocumentSchema {
    const FILE_NAME: &'static str;
    const MARKER_NAME: &'static str;
    const DOMAIN: &'static str;
    /// dirty patch 所在的固定对象路径；主应用可选 ["settings"]，请求不能改变它。
    const PATCH_ROOT: &'static [&'static str] = &[];
    fn defaults(path: &Path) -> Result<Map<String, Value>>;
    /// 保留未知扩展；migrate 仅用于首次转换未版本化的文件。错误不得包含字段值。
    fn validate(
        values: &Map<String, Value>,
        path: &Path,
        migrate: bool,
    ) -> Result<Map<String, Value>>;
}

pub struct InputSettingsSchema;
impl DocumentSchema for InputSettingsSchema {
    const FILE_NAME: &'static str = "settings.json";
    const MARKER_NAME: &'static str = MARKER;
    const DOMAIN: &'static str = "inputia.basic-input";
    fn defaults(path: &Path) -> Result<Map<String, Value>> {
        serde_json::to_value(InputiaSettings::default_for_settings_path(path))
            .map_err(|_| Error::InvalidDocument)?
            .as_object()
            .cloned()
            .ok_or(Error::InvalidDocument)
    }
    fn validate(
        values: &Map<String, Value>,
        path: &Path,
        migrate: bool,
    ) -> Result<Map<String, Value>> {
        validate_values(values, path, migrate)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Error {
    InvalidDocument,
    InvalidRequest,
    UnsafePath,
    StorageUnavailable,
    Busy,
    Maintenance,
    ExternalEdit,
    ExternalChanged,
    RepairRequired,
    CommitUncertain,
    OperationMismatch,
    RevisionExhausted,
}
pub type Result<T> = std::result::Result<T, Error>;
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "设置操作失败：{self:?}")
    }
}
impl std::error::Error for Error {}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    #[serde(
        default = "input_settings_domain",
        skip_serializing_if = "is_input_settings_domain"
    )]
    domain: String,
    pub store_id: String,
    pub revision: String,
    pub values_digest: String,
    pub values: Value,
}
impl Snapshot {
    pub fn domain(&self) -> &str {
        &self.domain
    }
    pub fn operation_id(&self) -> String {
        format!(
            "v1:{}:{}:{}",
            self.store_id,
            self.revision,
            uuid::Uuid::new_v4()
        )
    }
    pub fn settings(&self) -> Result<InputiaSettings> {
        if self.domain != InputSettingsSchema::DOMAIN {
            return Err(Error::InvalidDocument);
        }
        serde_json::from_value(self.values.clone()).map_err(|_| Error::InvalidDocument)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchRequest {
    pub operation_id: String,
    pub expected_store_id: String,
    pub expected_revision: String,
    pub patch: BTreeMap<String, Value>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ApplyResult {
    Saved {
        commit_revision: String,
        replayed: bool,
        current: Snapshot,
    },
    Conflict {
        current: Snapshot,
    },
    OutcomeExpired {
        current: Snapshot,
    },
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    operation_id: String,
    request_digest: String,
    revision: String,
    values_digest: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    schema_version: u32,
    #[serde(
        default = "input_settings_domain",
        skip_serializing_if = "is_input_settings_domain"
    )]
    domain: String,
    store_id: String,
    revision: String,
    values_digest: String,
    receipts: Vec<Receipt>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Marker {
    schema_version: u32,
    #[serde(
        default = "input_settings_domain",
        skip_serializing_if = "is_input_settings_domain"
    )]
    domain: String,
    store_id: String,
}
struct Document {
    source_digest: Option<String>,
    values: Map<String, Value>,
    header: Header,
}
impl Document {
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            domain: self.header.domain.clone(),
            store_id: self.header.store_id.clone(),
            revision: self.header.revision.clone(),
            values_digest: self.header.values_digest.clone(),
            values: Value::Object(self.values.clone()),
        }
    }
    fn bytes(&self) -> Result<Vec<u8>> {
        let mut values = self.values.clone();
        values.insert(
            META.into(),
            serde_json::to_value(&self.header).map_err(|_| Error::InvalidDocument)?,
        );
        let bytes = canonical(&Value::Object(values))?;
        if bytes.len() > LIMIT {
            return Err(Error::InvalidDocument);
        }
        Ok(bytes)
    }
}
fn canonical(value: &Value) -> Result<Vec<u8>> {
    let mut value = value.clone();
    value.sort_all_objects();
    serde_json::to_vec(&value).map_err(|_| Error::InvalidDocument)
}
fn raw_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect()
}
fn digest(value: &Value) -> Result<String> {
    Ok(raw_digest(&canonical(value)?))
}
fn revision(raw: &str) -> Result<u64> {
    if raw.is_empty()
        || (raw.starts_with('0') && raw != "0")
        || !raw.bytes().all(|v| v.is_ascii_digit())
    {
        return Err(Error::InvalidDocument);
    }
    raw.parse().map_err(|_| Error::InvalidDocument)
}
fn is_digest(raw: &str) -> bool {
    raw.len() == 64
        && raw
            .bytes()
            .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
}
fn operation_base(id: &str, store: &str) -> Result<u64> {
    let parts: Vec<_> = id.split(':').collect();
    if parts.len() != 4
        || parts[0] != "v1"
        || parts[1] != store
        || !valid_uuid(store)
        || !valid_uuid(parts[3])
    {
        return Err(Error::InvalidRequest);
    }
    revision(parts[2]).map_err(|_| Error::InvalidRequest)
}
fn validate_values(
    values: &Map<String, Value>,
    path: &Path,
    migrate: bool,
) -> Result<Map<String, Value>> {
    let parsed: InputiaSettings = serde_json::from_value(Value::Object(values.clone()))
        .map_err(|_| Error::InvalidDocument)?;
    let mut normalized = parsed.clone();
    normalized.sanitize_for_settings_path(path);
    if !migrate && normalized != parsed {
        return Err(Error::InvalidRequest);
    }
    if normalized.schema_id.len() > 128
        || normalized.sensitive_bundle_ids.len() > 512
        || normalized
            .sensitive_bundle_ids
            .iter()
            .any(|v| v.is_empty() || v.len() > 256 || v.chars().any(char::is_control))
    {
        return Err(Error::InvalidRequest);
    }
    let mut result = values.clone();
    let known = serde_json::to_value(normalized).map_err(|_| Error::InvalidDocument)?;
    result.extend(known.as_object().ok_or(Error::InvalidDocument)?.clone());
    Ok(result)
}

/// 每次用户动作打开并释放一个短生命周期句柄，不能持锁等待用户输入或网络。
pub struct DocumentStore<S: DocumentSchema> {
    files: Files,
    path: PathBuf,
    home: PathBuf,
    uid: u32,
    schema: std::marker::PhantomData<S>,
}
/// 保留基础输入设置的原公共入口和领域观察接口。
pub type Store = DocumentStore<InputSettingsSchema>;

fn safe_name(name: &str) -> bool {
    (1..=128).contains(&name.len())
        && ![
            ".",
            "..",
            ".inputia-settings.lock",
            ".inputia-settings-applications.json",
        ]
        .contains(&name)
        && name
            .bytes()
            .all(|v| v.is_ascii_alphanumeric() || b"._-".contains(&v))
}
impl<S: DocumentSchema> DocumentStore<S> {
    pub fn open(path: &Path, home: &Path, uid: u32) -> Result<Self> {
        if uid != unsafe { libc::geteuid() }
            || !path.is_absolute()
            || !home.is_absolute()
            || !path.starts_with(home)
            || path.file_name().and_then(|v| v.to_str()) != Some(S::FILE_NAME)
            || !safe_name(S::FILE_NAME)
            || !safe_name(S::MARKER_NAME)
            || S::FILE_NAME == S::MARKER_NAME
            || !safe_name(S::DOMAIN)
            || S::PATCH_ROOT.len() > 4
            || S::PATCH_ROOT.iter().any(|key| {
                key.is_empty()
                    || key.len() > 128
                    || *key == META
                    || key.chars().any(char::is_control)
            })
        {
            return Err(Error::UnsafePath);
        }
        maintenance::ensure_normal_start(home, uid).map_err(|_| Error::Maintenance)?;
        let files = Files::open(path.parent().ok_or(Error::UnsafePath)?, uid)?;
        maintenance::ensure_normal_start(home, uid).map_err(|_| Error::Maintenance)?;
        Ok(Self {
            files,
            path: path.into(),
            home: home.into(),
            uid,
            schema: std::marker::PhantomData,
        })
    }
    fn load(&self) -> Result<Document> {
        self.load_internal(false)
    }
    fn load_internal(&self, external: bool) -> Result<Document> {
        maintenance::ensure_normal_start(&self.home, self.uid).map_err(|_| Error::Maintenance)?;
        let marker = self
            .files
            .read(S::MARKER_NAME, 4096, true)?
            .map(|raw| {
                strict_json(&raw).and_then(|value| {
                    serde_json::from_value::<Marker>(value).map_err(|_| Error::RepairRequired)
                })
            })
            .transpose()?;
        if marker.as_ref().is_some_and(|v| {
            v.schema_version != 1 || v.domain != S::DOMAIN || !valid_uuid(&v.store_id)
        }) {
            return Err(Error::RepairRequired);
        }
        let raw = self.files.read(S::FILE_NAME, LIMIT, false)?;
        if external && (raw.is_none() || marker.is_none()) {
            return Err(Error::RepairRequired);
        }
        let source_digest = raw.as_ref().map(|bytes| raw_digest(bytes));
        let mut values = match raw {
            Some(raw) => strict_json(&raw)?
                .as_object()
                .cloned()
                .ok_or(Error::InvalidDocument)?,
            None if marker.is_some() => return Err(Error::RepairRequired),
            None => S::defaults(&self.path)?,
        };
        let mut migrated = false;
        let header = match values.remove(META) {
            Some(value) => {
                serde_json::from_value::<Header>(value).map_err(|_| Error::RepairRequired)?
            }
            None if marker.is_some() => return Err(Error::RepairRequired),
            None => {
                values = S::validate(&values, &self.path, true)?;
                migrated = true;
                Header {
                    schema_version: 1,
                    domain: S::DOMAIN.into(),
                    store_id: uuid::Uuid::new_v4().to_string(),
                    revision: "0".into(),
                    values_digest: digest(&Value::Object(values.clone()))?,
                    receipts: vec![],
                }
            }
        };
        let current = revision(&header.revision)?;
        if header.schema_version != 1
            || header.domain != S::DOMAIN
            || !valid_uuid(&header.store_id)
            || header.receipts.len() > RECEIPT_LIMIT
            || marker
                .as_ref()
                .is_some_and(|v| v.store_id != header.store_id)
        {
            return Err(Error::RepairRequired);
        }
        if !external && digest(&Value::Object(values.clone()))? != header.values_digest {
            return Err(Error::ExternalEdit);
        }
        S::validate(&values, &self.path, false)?;
        let mut ids = BTreeSet::new();
        let mut previous = None;
        for receipt in &header.receipts {
            let base = operation_base(&receipt.operation_id, &header.store_id)
                .map_err(|_| Error::RepairRequired)?;
            let applied = revision(&receipt.revision)?;
            if base.checked_add(1) != Some(applied)
                || applied > current
                || previous.is_some_and(|v| applied != v + 1)
                || !ids.insert(&receipt.operation_id)
                || !is_digest(&receipt.request_digest)
                || !is_digest(&receipt.values_digest)
            {
                return Err(Error::RepairRequired);
            }
            previous = Some(applied);
        }
        if (current == 0 && !header.receipts.is_empty())
            || (current > 0 && previous != Some(current))
            || header
                .receipts
                .last()
                .is_some_and(|r| r.values_digest != header.values_digest)
        {
            return Err(Error::RepairRequired);
        }
        let document = Document {
            values,
            header,
            source_digest,
        };
        if migrated || marker.is_none() {
            maintenance::ensure_normal_start(&self.home, self.uid)
                .map_err(|_| Error::Maintenance)?;
        }
        if migrated {
            self.files
                .replace(S::FILE_NAME, &document.bytes()?, &mut |_| Ok(()))?;
        }
        if marker.is_none() {
            let marker = Marker {
                schema_version: 1,
                domain: S::DOMAIN.into(),
                store_id: document.header.store_id.clone(),
            };
            self.files.replace(
                S::MARKER_NAME,
                &serde_json::to_vec(&marker).map_err(|_| Error::InvalidDocument)?,
                &mut |_| Ok(()),
            )?;
        }
        Ok(document)
    }
    pub fn read(&self) -> Result<Snapshot> {
        Ok(self.load()?.snapshot())
    }
    pub fn apply(&self, request: &PatchRequest) -> Result<ApplyResult> {
        self.apply_with_hook(request, &mut |_| Ok(()))
    }
    fn apply_with_hook(
        &self,
        request: &PatchRequest,
        hook: &mut impl FnMut(Boundary) -> Result<()>,
    ) -> Result<ApplyResult> {
        maintenance::ensure_normal_start(&self.home, self.uid).map_err(|_| Error::Maintenance)?;
        let expected = revision(&request.expected_revision).map_err(|_| Error::InvalidRequest)?;
        if operation_base(&request.operation_id, &request.expected_store_id)? != expected
            || request.patch.is_empty()
            || request.patch.len() > 32
        {
            return Err(Error::InvalidRequest);
        }
        let known = S::defaults(&self.path)?;
        let mut known = &known;
        for key in S::PATCH_ROOT {
            known = known
                .get(*key)
                .and_then(Value::as_object)
                .ok_or(Error::InvalidDocument)?;
        }
        if request
            .patch
            .keys()
            .any(|key| key == META || known.get(key).is_none())
        {
            return Err(Error::InvalidRequest);
        }
        let request_digest =
            digest(&serde_json::to_value(request).map_err(|_| Error::InvalidRequest)?)?;
        let mut document = self.load()?;
        if request.expected_store_id != document.header.store_id {
            return Ok(ApplyResult::Conflict {
                current: document.snapshot(),
            });
        }
        if let Some(receipt) = document
            .header
            .receipts
            .iter()
            .find(|v| v.operation_id == request.operation_id)
        {
            if receipt.request_digest != request_digest {
                return Err(Error::OperationMismatch);
            }
            self.files
                .confirm_durable(S::FILE_NAME, S::MARKER_NAME, hook)?;
            return Ok(ApplyResult::Saved {
                commit_revision: receipt.revision.clone(),
                replayed: true,
                current: document.snapshot(),
            });
        }
        let current = revision(&document.header.revision)?;
        if expected != current {
            return if document.header.receipts.first().is_some_and(|v| {
                revision(&v.revision).is_ok_and(|oldest| expected < oldest.saturating_sub(1))
            }) {
                Ok(ApplyResult::OutcomeExpired {
                    current: document.snapshot(),
                })
            } else {
                Ok(ApplyResult::Conflict {
                    current: document.snapshot(),
                })
            };
        }
        let mut target = &mut document.values;
        for key in S::PATCH_ROOT {
            target = target
                .get_mut(*key)
                .and_then(Value::as_object_mut)
                .ok_or(Error::InvalidDocument)?;
        }
        target.extend(request.patch.clone());
        document.values = S::validate(&document.values, &self.path, false)?;
        document.header.revision = current
            .checked_add(1)
            .ok_or(Error::RevisionExhausted)?
            .to_string();
        document.header.values_digest = digest(&Value::Object(document.values.clone()))?;
        document.header.receipts.push(Receipt {
            operation_id: request.operation_id.clone(),
            request_digest,
            revision: document.header.revision.clone(),
            values_digest: document.header.values_digest.clone(),
        });
        if document.header.receipts.len() > RECEIPT_LIMIT {
            document.header.receipts.remove(0);
        }
        maintenance::ensure_normal_start(&self.home, self.uid).map_err(|_| Error::Maintenance)?;
        self.files.replace(S::FILE_NAME, &document.bytes()?, hook)?;
        Ok(ApplyResult::Saved {
            commit_revision: document.header.revision.clone(),
            replayed: false,
            current: document.snapshot(),
        })
    }
}

/// serde Value 默认接受重复键；设置合同要求在转换成 Map 前拒绝重复。
pub fn strict_json(raw: &[u8]) -> Result<Value> {
    if raw.is_empty() || raw.len() > LIMIT {
        return Err(Error::InvalidDocument);
    }
    struct Strict(Value);
    impl<'de> Deserialize<'de> for Strict {
        fn deserialize<D: serde::Deserializer<'de>>(
            deserializer: D,
        ) -> std::result::Result<Self, D::Error> {
            struct Visitor;
            impl<'de> serde::de::Visitor<'de> for Visitor {
                type Value = Strict;
                fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    f.write_str("JSON without duplicate keys")
                }
                fn visit_bool<E: serde::de::Error>(
                    self,
                    v: bool,
                ) -> std::result::Result<Strict, E> {
                    Ok(Strict(Value::Bool(v)))
                }
                fn visit_str<E: serde::de::Error>(self, v: &str) -> std::result::Result<Strict, E> {
                    Ok(Strict(Value::String(v.into())))
                }
                fn visit_string<E: serde::de::Error>(
                    self,
                    v: String,
                ) -> std::result::Result<Strict, E> {
                    Ok(Strict(Value::String(v)))
                }
                fn visit_i64<E: serde::de::Error>(self, v: i64) -> std::result::Result<Strict, E> {
                    Ok(Strict(v.into()))
                }
                fn visit_u64<E: serde::de::Error>(self, v: u64) -> std::result::Result<Strict, E> {
                    Ok(Strict(v.into()))
                }
                fn visit_f64<E: serde::de::Error>(self, v: f64) -> std::result::Result<Strict, E> {
                    serde_json::Number::from_f64(v)
                        .map(|number| Strict(Value::Number(number)))
                        .ok_or_else(|| E::custom("non-finite number"))
                }
                fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Strict, E> {
                    Ok(Strict(Value::Null))
                }
                fn visit_none<E: serde::de::Error>(self) -> std::result::Result<Strict, E> {
                    Ok(Strict(Value::Null))
                }
                fn visit_seq<A: serde::de::SeqAccess<'de>>(
                    self,
                    mut access: A,
                ) -> std::result::Result<Strict, A::Error> {
                    let mut values = vec![];
                    while let Some(Strict(value)) = access.next_element()? {
                        if values.len() >= 1024 {
                            return Err(serde::de::Error::custom("array limit"));
                        }
                        values.push(value);
                    }
                    Ok(Strict(Value::Array(values)))
                }
                fn visit_map<A: serde::de::MapAccess<'de>>(
                    self,
                    mut access: A,
                ) -> std::result::Result<Strict, A::Error> {
                    let mut values = Map::new();
                    while let Some((key, Strict(value))) = access.next_entry::<String, Strict>()? {
                        if values.len() >= 1024 || values.insert(key, value).is_some() {
                            return Err(serde::de::Error::custom("duplicate key or map limit"));
                        }
                    }
                    Ok(Strict(Value::Object(values)))
                }
            }
            deserializer.deserialize_any(Visitor)
        }
    }
    serde_json::from_slice::<Strict>(raw)
        .map(|v| v.0)
        .map_err(|_| Error::InvalidDocument)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{
        fs,
        os::unix::fs::{symlink, PermissionsExt},
        process::Command,
    };
    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf, u32) {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().canonicalize().unwrap();
        let path = home.join("profile/settings.json");
        let uid = unsafe { libc::geteuid() };
        (temp, home, path, uid)
    }
    fn request(snapshot: &Snapshot, value: bool) -> PatchRequest {
        PatchRequest {
            operation_id: snapshot.operation_id(),
            expected_store_id: snapshot.store_id.clone(),
            expected_revision: snapshot.revision.clone(),
            patch: BTreeMap::from([("memory_enabled".into(), json!(value))]),
        }
    }
    struct NestedFixture;
    impl DocumentSchema for NestedFixture {
        const FILE_NAME: &'static str = "control-fixture.json";
        const MARKER_NAME: &'static str = ".control-fixture-initialized.json";
        const DOMAIN: &'static str = "fixture.control";
        const PATCH_ROOT: &'static [&'static str] = &["settings"];
        fn defaults(_: &Path) -> Result<Map<String, Value>> {
            Ok(json!({"settings":{"theme":"light","sound":true}})
                .as_object()
                .unwrap()
                .clone())
        }
        fn validate(values: &Map<String, Value>, _: &Path, _: bool) -> Result<Map<String, Value>> {
            let settings = values
                .get("settings")
                .and_then(Value::as_object)
                .ok_or(Error::InvalidDocument)?;
            if settings.get("theme").and_then(Value::as_str).is_none()
                || settings.get("sound").and_then(Value::as_bool).is_none()
            {
                return Err(Error::InvalidDocument);
            }
            Ok(values.clone())
        }
    }
    #[test]
    fn nested_schema_preserves_both_unknown_layers_and_only_syncs_its_own_files() {
        let (_temp, home, path, uid) = fixture();
        let input = Store::open(&path, &home, uid).unwrap();
        input.read().unwrap();
        drop(input);
        let input_bytes = fs::read(&path).unwrap();
        assert!(
            strict_json(&input_bytes).unwrap()[META]
                .get("domain")
                .is_none(),
            "原Input序列化格式不增加未知字段"
        );
        let other_path = path.with_file_name(NestedFixture::FILE_NAME);
        fs::write(&other_path,br#"{"settings":{"theme":"light","sound":true,"futureNested":{"v":2}},"futureRoot":[1,2]}"#).unwrap();
        let other = DocumentStore::<NestedFixture>::open(&other_path, &home, uid).unwrap();
        let snapshot = other.read().unwrap();
        assert_eq!(snapshot.domain(), NestedFixture::DOMAIN);
        assert!(snapshot.settings().is_err());
        let request = PatchRequest {
            operation_id: snapshot.operation_id(),
            expected_store_id: snapshot.store_id,
            expected_revision: snapshot.revision,
            patch: BTreeMap::from([("theme".into(), json!("dark"))]),
        };
        let result = other.apply_with_hook(&request, &mut |point| {
            if point == Boundary::Renamed {
                Err(Error::StorageUnavailable)
            } else {
                Ok(())
            }
        });
        assert!(matches!(result, Err(Error::CommitUncertain)));
        drop(other);
        // 第一领域暂时无法读取；第二领域重放不得去同步它或它的 marker。
        fs::rename(&path, path.with_extension("held")).unwrap();
        let other = DocumentStore::<NestedFixture>::open(&other_path, &home, uid).unwrap();
        let ApplyResult::Saved {
            current,
            replayed: true,
            ..
        } = other.apply(&request).unwrap()
        else {
            panic!("原请求应耐久重放");
        };
        assert_eq!(current.values["settings"]["theme"], "dark");
        assert_eq!(current.values["settings"]["sound"], true);
        assert_eq!(current.values["settings"]["futureNested"]["v"], 2);
        assert_eq!(current.values["futureRoot"], json!([1, 2]));
        let mut raw = strict_json(&fs::read(&other_path).unwrap()).unwrap();
        raw["settings"]["theme"] = json!("external");
        fs::write(&other_path, serde_json::to_vec(&raw).unwrap()).unwrap();
        let preview = other.inspect_external().unwrap();
        let imported = ImportRequest {
            operation_id: current.operation_id(),
            expected_store_id: preview.store_id,
            expected_revision: preview.revision,
            observed_file_digest: preview.observed_file_digest,
        };
        assert!(matches!(
            other.import_external(&imported).unwrap(),
            ApplyResult::Saved {
                replayed: false,
                ..
            }
        ));
        assert!(matches!(
            other.import_external(&imported).unwrap(),
            ApplyResult::Saved { replayed: true, .. }
        ));
        drop(other);
        fs::rename(path.with_extension("held"), &path).unwrap();
        assert_eq!(fs::read(&path).unwrap(), input_bytes);
        assert_eq!(
            Store::open(&path, &home, uid)
                .unwrap()
                .read()
                .unwrap()
                .revision,
            "0"
        );
    }
    #[test]
    fn schema_identity_and_patch_root_cannot_be_redirected_by_a_document() {
        let (_temp, home, path, uid) = fixture();
        let input = Store::open(&path, &home, uid).unwrap();
        input.read().unwrap();
        drop(input);
        let other_path = path.with_file_name(NestedFixture::FILE_NAME);
        let other = DocumentStore::<NestedFixture>::open(&other_path, &home, uid).unwrap();
        let snapshot = other.read().unwrap();
        let invalid = PatchRequest {
            operation_id: snapshot.operation_id(),
            expected_store_id: snapshot.store_id,
            expected_revision: snapshot.revision,
            patch: BTreeMap::from([("settings".into(), json!({"theme":"replace"}))]),
        };
        assert!(matches!(other.apply(&invalid), Err(Error::InvalidRequest)));
        drop(other);
        fs::copy(&other_path, &path).unwrap();
        fs::copy(
            path.with_file_name(NestedFixture::MARKER_NAME),
            path.with_file_name(MARKER),
        )
        .unwrap();
        assert!(matches!(
            Store::open(&path, &home, uid).unwrap().read(),
            Err(Error::RepairRequired)
        ));
        assert!(!safe_name("../escape"));
        assert!(!safe_name("/escape"));
        assert!(!safe_name(".."));
        assert!(!safe_name(".inputia-settings.lock"));
        assert!(!safe_name(".inputia-settings-applications.json"));
    }
    #[test]
    fn legacy_migration_and_cas_keep_unknown_fields_and_report_current_values() {
        let (_temp, home, path, uid) = fixture();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            br#"{"candidate_page_size":99,"future_extension":"preserved"}"#,
        )
        .unwrap();
        let store = Store::open(&path, &home, uid).unwrap();
        let first = store.read().unwrap();
        assert_eq!(first.revision, "0");
        assert_eq!(first.values["candidate_page_size"], 9);
        assert_eq!(first.values["future_extension"], "preserved");
        let one = request(&first, false);
        let two = request(&first, true);
        assert!(matches!(
            store.apply(&one),
            Ok(ApplyResult::Saved {
                replayed: false,
                ..
            })
        ));
        assert!(
            matches!(store.apply(&two),Ok(ApplyResult::Conflict {current}) if current.revision=="1" && current.values["memory_enabled"]==false)
        );
        drop(store);
        let reopened = Store::open(&path, &home, uid).unwrap();
        assert!(
            matches!(reopened.apply(&one),Ok(ApplyResult::Saved {replayed:true,commit_revision,..}) if commit_revision=="1")
        );
        let mut changed = one;
        changed.patch.insert("memory_enabled".into(), json!(true));
        assert!(matches!(
            reopened.apply(&changed),
            Err(Error::OperationMismatch)
        ));
        assert_eq!(
            reopened.read().unwrap().values["future_extension"],
            "preserved"
        );
    }
    #[test]
    fn rolling_receipts_do_not_exhaust_writes_or_replay_evicted_operations() {
        let (_temp, home, path, uid) = fixture();
        let store = Store::open(&path, &home, uid).unwrap();
        let mut snapshot = store.read().unwrap();
        let old = request(&snapshot, false);
        for index in 0..260 {
            let next = request(&snapshot, index % 2 == 0);
            store.apply(&next).unwrap();
            snapshot = store.read().unwrap();
        }
        assert_eq!(snapshot.revision, "260");
        assert!(matches!(
            store.apply(&old),
            Ok(ApplyResult::OutcomeExpired { .. })
        ));
        let raw = strict_json(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(raw[META]["receipts"].as_array().unwrap().len(), 256);
        let mut forged = old;
        forged.expected_revision = snapshot.revision;
        assert!(matches!(store.apply(&forged), Err(Error::InvalidRequest)));
        assert_eq!(store.read().unwrap().revision, "260");
    }
    #[test]
    fn atomic_commit_boundaries_keep_receipt_and_value_together() {
        for boundary in [
            Boundary::TempWritten,
            Boundary::FileSynced,
            Boundary::Renamed,
            Boundary::DirectorySynced,
        ] {
            let (_temp, home, path, uid) = fixture();
            let store = Store::open(&path, &home, uid).unwrap();
            let snapshot = store.read().unwrap();
            let change = request(&snapshot, false);
            let error = store.apply_with_hook(&change, &mut |event| {
                if event == boundary {
                    Err(Error::StorageUnavailable)
                } else {
                    Ok(())
                }
            });
            let after_rename = matches!(boundary, Boundary::Renamed | Boundary::DirectorySynced);
            assert!(matches!(error, Err(Error::CommitUncertain)) == after_rename);
            drop(store);
            let store = Store::open(&path, &home, uid).unwrap();
            let applied = store.apply(&change).unwrap();
            assert!(
                matches!(applied,ApplyResult::Saved {commit_revision, replayed,..} if commit_revision=="1" && replayed==after_rename)
            );
            assert_eq!(store.read().unwrap().values["memory_enabled"], false);
        }
    }
    #[test]
    fn uncertain_commit_is_not_saved_until_retry_confirms_durability() {
        let (_temp, home, path, uid) = fixture();
        let store = Store::open(&path, &home, uid).unwrap();
        let operation = request(&store.read().unwrap(), false);
        assert_eq!(
            store
                .apply_with_hook(&operation, &mut |boundary| {
                    if boundary == Boundary::Renamed {
                        Err(Error::StorageUnavailable)
                    } else {
                        Ok(())
                    }
                })
                .unwrap_err(),
            Error::CommitUncertain
        );
        drop(store);
        let store = Store::open(&path, &home, uid).unwrap();
        for fault in [Boundary::ReplayFileSynced, Boundary::ReplayDirectorySynced] {
            assert_eq!(
                store
                    .apply_with_hook(&operation, &mut |boundary| {
                        if boundary == fault {
                            Err(Error::StorageUnavailable)
                        } else {
                            Ok(())
                        }
                    })
                    .unwrap_err(),
                Error::CommitUncertain
            );
        }
        assert!(
            matches!(store.apply(&operation).unwrap(), ApplyResult::Saved { replayed:true, commit_revision,.. } if commit_revision=="1")
        );
    }
    #[test]
    fn external_changes_missing_metadata_and_missing_file_are_preserved_for_repair() {
        for mutation in ["edit", "metadata", "missing", "duplicate"] {
            let (_temp, home, path, uid) = fixture();
            let store = Store::open(&path, &home, uid).unwrap();
            store.read().unwrap();
            let mut value = strict_json(&fs::read(&path).unwrap()).unwrap();
            match mutation {
                "edit" => value["memory_enabled"] = json!(false),
                "metadata" => {
                    value.as_object_mut().unwrap().remove(META);
                }
                "missing" => {
                    fs::remove_file(&path).unwrap();
                }
                _ => {}
            }
            if mutation != "missing" {
                let mut bytes = serde_json::to_vec(&value).unwrap();
                if mutation == "duplicate" {
                    bytes = br#"{"memory_enabled":true,"memory_enabled":false}"#.to_vec();
                }
                fs::write(&path, &bytes).unwrap();
            }
            let before = fs::read(&path).ok();
            assert!(store.read().is_err());
            assert_eq!(fs::read(&path).ok(), before);
        }
    }
    #[test]
    fn invalid_patch_and_late_maintenance_never_overwrite_values() {
        let (_temp, home, path, uid) = fixture();
        let store = Store::open(&path, &home, uid).unwrap();
        let snapshot = store.read().unwrap();
        for (key, value) in [
            ("candidate_page_size", json!(99)),
            ("new_unrecognized", json!(true)),
            ("chinese_script", json!("typo")),
        ] {
            let mut change = request(&snapshot, false);
            change.patch = BTreeMap::from([(key.into(), value)]);
            assert!(store.apply(&change).is_err());
            assert_eq!(store.read().unwrap().revision, "0");
        }
        let marker = maintenance::marker_path(&home);
        fs::create_dir_all(marker.parent().unwrap()).unwrap();
        fs::write(&marker, b"invalid").unwrap();
        fs::set_permissions(&marker, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(matches!(
            store.apply(&request(&snapshot, false)),
            Err(Error::Maintenance)
        ));
        assert!(matches!(store.read(), Err(Error::Maintenance)));
    }
    #[test]
    fn links_and_fixed_lock_are_rejected_and_same_process_handles_are_exclusive() {
        let (_temp, home, path, uid) = fixture();
        let store = Store::open(&path, &home, uid).unwrap();
        store.read().unwrap();
        assert!(matches!(Store::open(&path, &home, uid), Err(Error::Busy)));
        drop(store);
        let other = home.join("copy.json");
        fs::rename(&path, &other).unwrap();
        symlink(&other, &path).unwrap();
        assert!(Store::open(&path, &home, uid).unwrap().read().is_err());
        fs::remove_file(&path).unwrap();
        fs::rename(&other, &path).unwrap();
        let lock = path.parent().unwrap().join(".inputia-settings.lock");
        fs::set_permissions(&lock, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(
            Store::open(&path, &home, uid),
            Err(Error::UnsafePath)
        ));
    }
    #[test]
    fn child_writer() {
        let Ok(home) = std::env::var("INPUTIA_SETTINGS_TEST_HOME") else {
            return;
        };
        let home = PathBuf::from(home);
        let path = home.join("profile/settings.json");
        let request: PatchRequest =
            serde_json::from_str(&std::env::var("INPUTIA_SETTINGS_TEST_REQUEST").unwrap()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match Store::open(&path, &home, unsafe { libc::geteuid() }) {
                Ok(store) => {
                    let code = match store.apply(&request).unwrap() {
                        ApplyResult::Saved { .. } => 10,
                        ApplyResult::Conflict { .. } => 11,
                        _ => 12,
                    };
                    std::process::exit(code);
                }
                Err(Error::Busy) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(5))
                }
                _ => std::process::exit(13),
            }
        }
    }
    #[test]
    fn independent_processes_same_revision_commit_exactly_once() {
        let (_temp, home, path, uid) = fixture();
        let store = Store::open(&path, &home, uid).unwrap();
        let snapshot = store.read().unwrap();
        drop(store);
        let mut children = vec![];
        for value in [true, false] {
            children.push(
                Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "store::tests::child_writer"])
                    .env("INPUTIA_SETTINGS_TEST_HOME", &home)
                    .env(
                        "INPUTIA_SETTINGS_TEST_REQUEST",
                        serde_json::to_string(&request(&snapshot, value)).unwrap(),
                    )
                    .stdout(std::process::Stdio::null())
                    .spawn()
                    .unwrap(),
            );
        }
        let mut codes: Vec<_> = children
            .iter_mut()
            .map(|child| child.wait().unwrap().code().unwrap())
            .collect();
        codes.sort();
        assert_eq!(codes, [10, 11]);
        assert_eq!(
            Store::open(&path, &home, uid)
                .unwrap()
                .read()
                .unwrap()
                .revision,
            "1"
        );
    }
}
