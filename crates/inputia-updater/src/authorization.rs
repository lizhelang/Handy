//! 安装入口的授权信封。
//!
//! 该模块只做本地、只读的身份绑定：它不能替代发布目录的密码验签或
//! Apple 代码签名检查，但会阻止一个脱离发布清单的裸安装请求进入
//! `prepare` 输出。正式 bootstrap 应在写入信封前完成上游签名验证。

use crate::{artifact_set_digest, Artifact, Error, PreparedPlan, Result, Role};
use inputia_settings::installation::{valid_release_id, valid_uuid};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallAuthorization {
    pub schema_version: u32,
    pub product_id: String,
    pub installation_id: String,
    pub release_id: String,
    pub pair_manifest_sha256: String,
    pub artifact_set_sha256: String,
    /// 上游发布信封的完整文件摘要；空值明确表示尚未接入签名发布目录。
    pub release_envelope_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct AuthorizationEvidence {
    pub schema_version: u32,
    pub product_id: String,
    pub installation_id: String,
    pub release_id: String,
    pub pair_manifest_sha256: String,
    pub artifact_set_sha256: String,
    pub release_envelope_sha256: String,
}

fn digest<T: Serialize>(value: &T) -> Result<String> {
    let bytes = serde_json::to_vec(value).map_err(|_| Error::Invalid("授权摘要编码失败"))?;
    Ok(Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

fn valid_sha(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// 以安装请求中的角色和预期摘要计算确定性制品集合摘要。
pub fn request_artifact_set_digest(artifacts: &[Artifact]) -> Result<String> {
    let mut roles = BTreeSet::new();
    let mut values = Vec::with_capacity(artifacts.len());
    for artifact in artifacts {
        if !roles.insert(artifact.role) || artifact.role == Role::Receipt {
            return Err(Error::Invalid("授权请求包含重复或非法角色"));
        }
        values.push((artifact.role, &artifact.expected));
    }
    values.sort_by_key(|value| value.0);
    digest(&values)
}

impl InstallAuthorization {
    /// 从尚未预检的请求生成候选信封，供外部 bootstrap 在调用
    /// `prepare_authorized` 前使用；真实计划仍会再次核对所有文件摘要。
    pub fn for_request(
        request: &crate::InstallRequest,
        pair_manifest_sha256: String,
        release_envelope_sha256: String,
    ) -> Result<Self> {
        if !valid_sha(&pair_manifest_sha256) || !valid_sha(&release_envelope_sha256) {
            return Err(Error::Invalid("授权信封摘要无效"));
        }
        let pair = request
            .artifacts
            .iter()
            .find(|artifact| artifact.role == Role::PairManifest)
            .ok_or(Error::MissingArtifact)?;
        if pair.expected.sha256 != pair_manifest_sha256 {
            return Err(Error::ArtifactMismatch);
        }
        Ok(Self {
            schema_version: 1,
            product_id: request.new_receipt.product_id.clone(),
            installation_id: request.new_receipt.installation_id.clone(),
            release_id: request.new_receipt.release_id.clone(),
            pair_manifest_sha256,
            artifact_set_sha256: request_artifact_set_digest(&request.artifacts)?,
            release_envelope_sha256,
        })
    }

    /// 从已完成只读计划生成未签名的本地绑定信封。`release_envelope_sha256`
    /// 必须来自上游发布验证器；此函数不会把它当作已验签。
    pub fn for_plan(
        plan: &PreparedPlan,
        pair_manifest_sha256: String,
        release_envelope_sha256: String,
    ) -> Result<Self> {
        if !valid_sha(&pair_manifest_sha256) || !valid_sha(&release_envelope_sha256) {
            return Err(Error::Invalid("授权信封摘要无效"));
        }
        let pair = plan
            .entries
            .iter()
            .find(|entry| entry.role == Role::PairManifest)
            .ok_or(Error::MissingArtifact)?;
        if pair.new.sha256 != pair_manifest_sha256 {
            return Err(Error::ArtifactMismatch);
        }
        Ok(Self {
            schema_version: 1,
            product_id: plan.request.new_receipt.product_id.clone(),
            installation_id: plan.request.new_receipt.installation_id.clone(),
            release_id: plan.request.new_receipt.release_id.clone(),
            pair_manifest_sha256,
            artifact_set_sha256: artifact_set_digest(&plan.entries)?,
            release_envelope_sha256,
        })
    }

    pub fn validate_request(&self, plan: &PreparedPlan) -> Result<AuthorizationEvidence> {
        if self.schema_version != 1
            || self.product_id != "com.inputia"
            || !valid_uuid(&self.installation_id)
            || !valid_release_id(&self.release_id)
            || !valid_sha(&self.pair_manifest_sha256)
            || !valid_sha(&self.artifact_set_sha256)
            || !valid_sha(&self.release_envelope_sha256)
        {
            return Err(Error::Invalid("安装授权信封身份或摘要无效"));
        }
        if self.installation_id != plan.request.new_receipt.installation_id
            || self.release_id != plan.request.new_receipt.release_id
        {
            return Err(Error::Invalid("安装授权与新收据身份不一致"));
        }
        let artifact_digest = artifact_set_digest(&plan.entries)?;
        if artifact_digest != self.artifact_set_sha256 {
            return Err(Error::ArtifactMismatch);
        }
        let pair = plan
            .entries
            .iter()
            .find(|entry| entry.role == Role::PairManifest)
            .ok_or(Error::MissingArtifact)?;
        if pair.new.sha256 != self.pair_manifest_sha256 {
            return Err(Error::ArtifactMismatch);
        }
        Ok(AuthorizationEvidence {
            schema_version: self.schema_version,
            product_id: self.product_id.clone(),
            installation_id: self.installation_id.clone(),
            release_id: self.release_id.clone(),
            pair_manifest_sha256: self.pair_manifest_sha256.clone(),
            artifact_set_sha256: self.artifact_set_sha256.clone(),
            release_envelope_sha256: self.release_envelope_sha256.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Artifact, Entry, Fingerprint};
    use inputia_settings::installation::{
        ComponentPaths, DataLocation, InstallationReceipt, InstallationScope, UpdateChannel,
    };
    use std::path::PathBuf;

    fn receipt() -> InstallationReceipt {
        InstallationReceipt {
            schema_version: 1,
            product_id: "com.inputia".into(),
            installation_id: "123e4567-e89b-12d3-a456-426614174000".into(),
            profile_id: "123e4567-e89b-12d3-a456-426614174001".into(),
            uid: unsafe { libc::geteuid() },
            scope: InstallationScope::User,
            data: DataLocation::Managed,
            components: ComponentPaths {
                control: PathBuf::from("/tmp/control.app"),
                ime: PathBuf::from("/tmp/ime.app"),
                settings: PathBuf::from("/tmp/settings.app"),
            },
            release_id: "inputia-1-test".into(),
            channel: UpdateChannel::Candidate,
        }
    }

    fn plan() -> PreparedPlan {
        let fingerprint = |sha: char| Fingerprint {
            sha256: sha.to_string().repeat(64),
            bytes: 1,
            entries: 1,
        };
        let entries = [
            (Role::Control, 'a'),
            (Role::Ime, 'b'),
            (Role::Settings, 'c'),
            (Role::PairManifest, 'd'),
            (Role::Receipt, 'e'),
        ]
        .into_iter()
        .map(|(role, sha)| Entry {
            role,
            source: None,
            destination: PathBuf::from(format!("/tmp/{}.app", role.label())),
            stage: PathBuf::from(format!("/tmp/{}.stage", role.label())),
            backup: PathBuf::from(format!("/tmp/{}.backup", role.label())),
            failed: PathBuf::from(format!("/tmp/{}.failed", role.label())),
            old: None,
            new: fingerprint(sha),
        })
        .collect();
        let artifacts = [
            (Role::Control, 'a'),
            (Role::Ime, 'b'),
            (Role::Settings, 'c'),
            (Role::PairManifest, 'd'),
        ]
        .into_iter()
        .map(|(role, sha)| Artifact {
            role,
            source: PathBuf::from(format!("/tmp/{}.app", role.label())),
            expected: fingerprint(sha),
        })
        .collect();
        PreparedPlan {
            schema_version: 1,
            home: PathBuf::from("/tmp"),
            uid: unsafe { libc::geteuid() },
            request: crate::InstallRequest {
                transaction_id: "123e4567-e89b-12d3-a456-426614174002".into(),
                new_receipt: receipt(),
                artifacts,
            },
            old_receipt: None,
            old_pair_manifest: None,
            entries,
            required_free_bytes: 1,
        }
    }

    #[test]
    fn constructor_round_trips_plan_binding() {
        let plan = plan();
        let pair = "d".repeat(64);
        let authorization = InstallAuthorization::for_plan(&plan, pair, "f".repeat(64)).unwrap();
        let evidence = authorization.validate_request(&plan).unwrap();
        assert_eq!(
            evidence.installation_id,
            plan.request.new_receipt.installation_id
        );
        assert_eq!(evidence.release_id, plan.request.new_receipt.release_id);
        assert_eq!(evidence.pair_manifest_sha256, "d".repeat(64));
        assert_eq!(
            evidence.artifact_set_sha256,
            authorization.artifact_set_sha256
        );
    }

    #[test]
    fn request_constructor_can_seed_prepare_authorization() {
        let plan = plan();
        let authorization =
            InstallAuthorization::for_request(&plan.request, "d".repeat(64), "f".repeat(64))
                .unwrap();
        let evidence = authorization.validate_request(&plan).unwrap();
        assert_eq!(
            evidence.artifact_set_sha256,
            authorization.artifact_set_sha256
        );
    }

    #[test]
    fn rejects_empty_release_envelope_digest() {
        let authorization = InstallAuthorization {
            schema_version: 1,
            product_id: "com.inputia".into(),
            installation_id: receipt().installation_id,
            release_id: receipt().release_id,
            pair_manifest_sha256: "0".repeat(64),
            artifact_set_sha256: "0".repeat(64),
            release_envelope_sha256: String::new(),
        };
        assert!(matches!(
            authorization.validate_request(&PreparedPlan {
                schema_version: 1,
                home: PathBuf::from("/tmp"),
                uid: unsafe { libc::geteuid() },
                request: crate::InstallRequest {
                    transaction_id: "123e4567-e89b-12d3-a456-426614174002".into(),
                    new_receipt: receipt(),
                    artifacts: vec![]
                },
                old_receipt: None,
                old_pair_manifest: None,
                entries: vec![],
                required_free_bytes: 0
            }),
            Err(Error::Invalid(_))
        ));
    }
}
