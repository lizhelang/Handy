//! 从已验签发布元数据导出原生代码核验策略；不接受独立 JSON 或待验 App 自报身份。
use crate::{manifest, ReleaseError, Result};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum NativeComponentRole {
    Control,
    Ime,
    Settings,
    Updater,
    Bootstrap,
}
impl NativeComponentRole {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "control" => Ok(Self::Control),
            "ime" => Ok(Self::Ime),
            "settings" => Ok(Self::Settings),
            "updater" => Ok(Self::Updater),
            "bootstrap" => Ok(Self::Bootstrap),
            _ => Err(ReleaseError::InvalidDocument),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeSlicePolicy {
    architecture: String,
    cdhash: String,
}
impl NativeSlicePolicy {
    pub fn architecture(&self) -> &str {
        &self.architecture
    }
    pub fn cdhash(&self) -> &str {
        &self.cdhash
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeComponentPolicy {
    role: NativeComponentRole,
    bundle_id: String,
    team_id: String,
    artifact: String,
    bundle_root: String,
    archive_sha256: String,
    archive_size: u64,
    slices: Vec<NativeSlicePolicy>,
}
impl NativeComponentPolicy {
    pub fn role(&self) -> NativeComponentRole {
        self.role
    }
    pub fn bundle_id(&self) -> &str {
        &self.bundle_id
    }
    pub fn team_id(&self) -> &str {
        &self.team_id
    }
    pub fn artifact(&self) -> &str {
        &self.artifact
    }
    pub fn bundle_root(&self) -> &str {
        &self.bundle_root
    }
    pub fn archive_sha256(&self) -> &str {
        &self.archive_sha256
    }
    pub fn archive_size(&self) -> u64 {
        self.archive_size
    }
    pub fn slices(&self) -> &[NativeSlicePolicy] {
        &self.slices
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PairManifestPolicy {
    artifact: String,
    sha256: String,
    signer_key_id: String,
    schema: u64,
}
impl PairManifestPolicy {
    pub fn artifact(&self) -> &str {
        &self.artifact
    }
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
    pub fn signer_key_id(&self) -> &str {
        &self.signer_key_id
    }
    pub fn schema(&self) -> u64 {
        self.schema
    }
}

/// 只能从 `AuthorizedReleaseMetadata` 导出；本类型没有反序列化或公开字段构造入口。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeReleasePolicy {
    product_id: String,
    release_id: String,
    version: String,
    build: u64,
    source_commit: String,
    architecture: String,
    components: Vec<NativeComponentPolicy>,
    pair_manifest: PairManifestPolicy,
}
impl NativeReleasePolicy {
    pub fn product_id(&self) -> &str {
        &self.product_id
    }
    pub fn release_id(&self) -> &str {
        &self.release_id
    }
    pub fn version(&self) -> &str {
        &self.version
    }
    pub fn build(&self) -> u64 {
        self.build
    }
    pub fn source_commit(&self) -> &str {
        &self.source_commit
    }
    pub fn architecture(&self) -> &str {
        &self.architecture
    }
    pub fn components(&self) -> &[NativeComponentPolicy] {
        &self.components
    }
    pub fn pair_manifest(&self) -> &PairManifestPolicy {
        &self.pair_manifest
    }
}

fn text(value: &serde_json::Value) -> Result<&str> {
    value.as_str().ok_or(ReleaseError::InvalidDocument)
}
fn integer(value: &serde_json::Value) -> Result<u64> {
    value.as_u64().ok_or(ReleaseError::InvalidDocument)
}

impl NativeReleasePolicy {
    /// 仅由发布授权函数在验签完成后立即调用并冻结；不得暴露为裸 JSON 转换入口。
    pub(crate) fn from_manifest(value: &serde_json::Value) -> Result<Self> {
        manifest::validate_manifest(value)?;
        let architecture = text(&value["target"]["architecture"])?;
        let mut components = value["components"]
            .as_array()
            .ok_or(ReleaseError::InvalidDocument)?
            .iter()
            .map(|component| {
                let role = NativeComponentRole::parse(text(&component["role"])?)?;
                let cdhashes = component["cdhashes"]
                    .as_array()
                    .ok_or(ReleaseError::InvalidDocument)?
                    .iter()
                    .map(|value| text(value).map(str::to_owned))
                    .collect::<Result<Vec<_>>>()?;
                Ok(NativeComponentPolicy {
                    role,
                    bundle_id: text(&component["bundle_id"])?.into(),
                    team_id: text(&component["signing_requirement"]["team_id"])?.into(),
                    artifact: text(&component["artifact"])?.into(),
                    bundle_root: text(&component["bundle_root"])?.into(),
                    archive_sha256: text(&component["sha256"])?.into(),
                    archive_size: integer(&component["size"])?,
                    slices: cdhashes
                        .into_iter()
                        .map(|cdhash| NativeSlicePolicy {
                            architecture: architecture.into(),
                            cdhash,
                        })
                        .collect(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        components.sort_by_key(|component| component.role);
        let pair = &value["pair_manifest"];
        Ok(NativeReleasePolicy {
            product_id: text(&value["product_id"])?.into(),
            release_id: text(&value["release_id"])?.into(),
            version: text(&value["version"])?.into(),
            build: integer(&value["build"])?,
            source_commit: text(&value["source_commit"])?.into(),
            architecture: architecture.into(),
            components,
            pair_manifest: PairManifestPolicy {
                artifact: text(&pair["artifact"])?.into(),
                sha256: text(&pair["sha256"])?.into(),
                signer_key_id: text(&pair["signer_key_id"])?.into(),
                schema: integer(&pair["schema"])?,
            },
        })
    }
}
