//! 把已验签发布策略中的归档摘要与根名绑定到受限解包证明。
//!
//! 本层只处理安装事务所需的三个应用组件；updater/bootstrap 属于独立恢复环境。
use crate::{
    archive::{extract_zip, ArchiveDigest, ArchiveLimits, ExtractedArchive},
    Artifact, Error, Fingerprint, Result, Role,
};
use inputia_release::{
    feed::AuthorizedReleaseMetadata,
    native_policy::{NativeComponentPolicy, NativeComponentRole},
};
use std::{fs::File, path::Path, sync::atomic::AtomicBool};

/// 只能由 `AuthorizedReleaseMetadata` 选择摘要与根名后创建。
/// 它同时持有解包外层目录和精确 `.app` 子树的 fd 身份。
#[derive(Debug)]
pub struct AuthorizedExtractedComponent {
    release_id: String,
    role: Role,
    archive: ExtractedArchive,
}

impl AuthorizedExtractedComponent {
    pub fn release_id(&self) -> &str {
        &self.release_id
    }
    pub fn role(&self) -> Role {
        self.role
    }
    pub fn bundle_path(&self) -> std::path::PathBuf {
        self.archive.bundle_path()
    }
    pub fn bundle_tree(&self) -> &Fingerprint {
        self.archive.bundle_tree()
    }
    /// 生成可直接交给 `Updater::prepare` 的制品描述。
    ///
    /// 描述中的来源只指向本次受限解包持有的精确 `.app` 根，摘要也来自
    /// 同一个 held fd；调用方仍须持有本对象直到事务预检完成。
    pub fn artifact(&self) -> Artifact {
        Artifact {
            role: self.role,
            source: self.bundle_path(),
            expected: self.bundle_tree().clone(),
        }
    }
    pub fn archive(&self) -> &ExtractedArchive {
        &self.archive
    }
    pub fn verify(&self, uid: u32, cancelled: &AtomicBool) -> Result<()> {
        self.archive.verify_bound_bundle(uid, &|| {
            if cancelled.load(std::sync::atomic::Ordering::Acquire) {
                Err(Error::Invalid("archive cancelled"))
            } else {
                Ok(())
            }
        })
    }
}

fn policy_role(role: Role) -> Result<NativeComponentRole> {
    match role {
        Role::Control => Ok(NativeComponentRole::Control),
        Role::Ime => Ok(NativeComponentRole::Ime),
        Role::Settings => Ok(NativeComponentRole::Settings),
        Role::PairManifest | Role::Receipt => {
            Err(Error::Invalid("component archive role required"))
        }
    }
}

fn component(metadata: &AuthorizedReleaseMetadata, role: Role) -> Result<&NativeComponentPolicy> {
    let role = policy_role(role)?;
    let mut matches = metadata
        .native_release_policy()
        .components()
        .iter()
        .filter(|component| component.role() == role);
    let component = matches
        .next()
        .ok_or(Error::Invalid("authorized component missing"))?;
    if matches.next().is_some() {
        return Err(Error::Invalid("authorized component ambiguous"));
    }
    Ok(component)
}

/// 从已验签元数据自行取得摘要和唯一根名；调用方不能注入替代值。
#[allow(clippy::too_many_arguments)]
pub fn extract_authorized_component(
    metadata: &AuthorizedReleaseMetadata,
    role: Role,
    source: File,
    destination: &Path,
    uid: u32,
    limits: &ArchiveLimits,
    cancelled: &AtomicBool,
) -> Result<AuthorizedExtractedComponent> {
    let policy = component(metadata, role)?;
    let expected = ArchiveDigest {
        sha256: policy.archive_sha256().into(),
        size: policy.archive_size(),
    };
    let archive = extract_zip(
        source,
        &expected,
        policy.bundle_root(),
        destination,
        uid,
        limits,
        cancelled,
    )?;
    archive.verify_bound_bundle(uid, &|| {
        if cancelled.load(std::sync::atomic::Ordering::Acquire) {
            Err(Error::Invalid("archive cancelled"))
        } else {
            Ok(())
        }
    })?;
    Ok(AuthorizedExtractedComponent {
        release_id: metadata.native_release_policy().release_id().into(),
        role,
        archive,
    })
}
