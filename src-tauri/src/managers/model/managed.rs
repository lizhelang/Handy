//! 新 HF 下载的私有命名空间。稳定条目不含 revision，已校验代际先落盘，最后切换索引。
//! 所有写者持模型目录的共同租约；读取不创建、迁移或清理任何文件。

use super::storage::ModelWriteLease;
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const DIRECTORY: &str = "managed-hf-v1";
const RECEIPT_LIMIT: u64 = 16 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Receipt {
    schema: u32,
    pub repo_id: String,
    pub filename: String,
    pub revision: String,
    pub sha256: String,
    pub size_bytes: u64,
}

fn safe_components(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 1024
        && value.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
        })
}

fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

impl Receipt {
    pub fn new(
        repo_id: &str,
        filename: &str,
        revision: &str,
        sha256: &str,
        size_bytes: u64,
    ) -> Result<Self> {
        let value = Self {
            schema: 1,
            repo_id: repo_id.into(),
            filename: filename.into(),
            revision: revision.into(),
            sha256: sha256.into(),
            size_bytes,
        };
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<()> {
        ensure!(
            self.schema == 1
                && safe_components(&self.repo_id)
                && self.repo_id.split('/').count() == 2
                && safe_components(&self.filename)
                && self.filename.ends_with(".gguf")
                && hex(&self.revision, 40)
                && hex(&self.sha256, 64)
                && self.size_bytes > 0
                && self.size_bytes <= (1_u64 << 50),
            "model_storage_invalid_receipt"
        );
        Ok(())
    }

    fn stem(&self) -> String {
        format!("{}-{}-{}", self.revision, self.sha256, self.size_bytes)
    }

    pub fn partial_path(&self, models: &Path) -> PathBuf {
        entry(models, &self.repo_id, &self.filename).join(format!("{}.partial", self.stem()))
    }

    fn payload_path(&self, models: &Path) -> PathBuf {
        entry(models, &self.repo_id, &self.filename).join(format!("{}.gguf", self.stem()))
    }

    pub fn prepare(&self, lease: &ModelWriteLease) -> Result<PathBuf> {
        self.validate()?;
        let models = lease.root();
        create_private_dir(&models.join(DIRECTORY))?;
        let directory = entry(models, &self.repo_id, &self.filename);
        create_private_dir(&directory)?;
        // 原 active 损坏时保留证据，不用下载覆盖来隐式修复。
        read_receipt(models, &self.repo_id, &self.filename)?;
        let partial = self.partial_path(models);
        recover_link_pair(&partial, &self.payload_path(models))?;
        if fs::symlink_metadata(&partial).is_ok() {
            regular_file(&partial)?;
        }
        Ok(partial)
    }

    pub fn publish_existing(&self, lease: &ModelWriteLease) -> Result<bool> {
        match fs::symlink_metadata(self.payload_path(lease.root())) {
            Ok(_) => {
                self.publish(lease)?;
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    /// 同一固定摘要的 HF 和镜像共用续传字节；revision 不同使用另一文件。
    pub fn url(&self, endpoint: &str) -> Result<String> {
        self.validate()?;
        let mut url = reqwest::Url::parse(endpoint)?;
        ensure!(
            matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none(),
            "model_download_invalid_endpoint"
        );
        url.path_segments_mut()
            .map_err(|_| anyhow::anyhow!("model_download_invalid_endpoint"))?
            .pop_if_empty()
            .extend(self.repo_id.split('/'))
            .push("resolve")
            .push(&self.revision)
            .extend(self.filename.split('/'));
        Ok(url.into())
    }

    /// 发布前从真实文件校验摘要，之后同步 payload、目录、索引及目录。
    /// 任一失败都保留上一份 active；最后目录同步失败时索引可能已切换，重试只对账。
    pub fn publish(&self, lease: &ModelWriteLease) -> Result<PathBuf> {
        self.validate()?;
        let models = lease.root();
        let directory = entry(models, &self.repo_id, &self.filename);
        validate_dir(&models.join(DIRECTORY))?;
        validate_dir(&directory)?;
        read_receipt(models, &self.repo_id, &self.filename)?;
        let payload = self.payload_path(models);
        let partial = self.partial_path(models);
        recover_link_pair(&partial, &payload)?;
        if fs::symlink_metadata(&payload).is_ok() {
            verify_payload(&payload, self)?;
        } else {
            verify_payload(&partial, self)?;
            // 合作写者由同一租约串行；不替换已存在的代际。
            fs::hard_link(&partial, &payload)?;
            sync_dir(&directory)?;
            fs::remove_file(&partial)?;
        }
        open_read(&payload)?.sync_all()?;
        sync_dir(&directory)?;
        let temporary = directory.join(format!(".active-{}.tmp", uuid::Uuid::new_v4()));
        let mut file = private_new(&temporary)?;
        let bytes = serde_json::to_vec(self)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, directory.join("active.json"))?;
        sync_dir(&directory)?;
        Ok(payload)
    }
}

// hard_link 已生效、partial 尚未移除时可安全重入。只接纳这两个准确名字指向同一 inode。
fn recover_link_pair(partial: &Path, payload: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(left), Ok(right)) =
            (fs::symlink_metadata(partial), fs::symlink_metadata(payload))
        {
            if left.is_file()
                && right.is_file()
                && left.nlink() == 2
                && right.nlink() == 2
                && left.dev() == right.dev()
                && left.ino() == right.ino()
                && left.uid() == unsafe { libc::geteuid() }
            {
                File::open(payload)?.sync_all()?;
                sync_dir(payload.parent().context("missing generation parent")?)?;
                fs::remove_file(partial)?;
            }
        }
    }
    #[cfg(not(unix))]
    let _ = (partial, payload);
    Ok(())
}

fn key(repo_id: &str, filename: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(repo_id.as_bytes());
    digest.update([0]);
    digest.update(filename.as_bytes());
    format!("{:x}", digest.finalize())
}

fn entry(models: &Path, repo_id: &str, filename: &str) -> PathBuf {
    models.join(DIRECTORY).join(key(repo_id, filename))
}

fn validate_dir(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(metadata.is_dir(), "model_storage_unsafe_directory");
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o777 == 0o700,
            "model_storage_unsafe_directory"
        );
    }
    Ok(())
}

fn create_private_dir(path: &Path) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.into()),
    }
    validate_dir(path)?;
    sync_dir(path)?;
    sync_dir(path.parent().context("model directory has no parent")?)
}

fn open_read(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    ensure!(metadata.is_file(), "model_storage_unsafe_file");
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            metadata.uid() == unsafe { libc::geteuid() } && metadata.nlink() == 1,
            "model_storage_unsafe_file"
        );
    }
    Ok(file)
}

fn regular_file(path: &Path) -> Result<u64> {
    Ok(open_read(path)?.metadata()?.len())
}

fn private_new(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    Ok(options.open(path)?)
}

fn sync_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn verify_payload(path: &Path, receipt: &Receipt) -> Result<()> {
    let mut file = open_read(path)?;
    ensure!(
        file.metadata()?.len() == receipt.size_bytes,
        "model_download_size_mismatch"
    );
    let mut digest = Sha256::new();
    let mut bytes = [0; 64 * 1024];
    loop {
        let count = file.read(&mut bytes)?;
        if count == 0 {
            break;
        }
        digest.update(&bytes[..count]);
    }
    ensure!(
        format!("{:x}", digest.finalize()) == receipt.sha256,
        "model_download_digest_mismatch"
    );
    file.sync_all()?;
    Ok(())
}

fn read_receipt(models: &Path, repo_id: &str, filename: &str) -> Result<Option<Receipt>> {
    let directory = entry(models, repo_id, filename);
    for path in [models.join(DIRECTORY), directory.clone()] {
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
            Ok(_) => validate_dir(&path)?,
        }
    }
    let file = match open_read(&directory.join("active.json")) {
        Ok(file) => file,
        Err(e)
            if e.downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            return Ok(None)
        }
        Err(e) => return Err(e),
    };
    let receipt = decode_receipt(file)?;
    ensure!(
        receipt.repo_id == repo_id && receipt.filename == filename,
        "model_storage_wrong_receipt"
    );
    Ok(Some(receipt))
}

fn decode_receipt(file: File) -> Result<Receipt> {
    ensure!(
        file.metadata()?.len() <= RECEIPT_LIMIT,
        "model_storage_invalid_receipt"
    );
    let mut bytes = Vec::new();
    file.take(RECEIPT_LIMIT + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= RECEIPT_LIMIT as usize,
        "model_storage_invalid_receipt"
    );
    let receipt: Receipt = serde_json::from_slice(&bytes)?;
    receipt.validate()?;
    Ok(receipt)
}

pub(super) fn discover(models: &Path) -> Vec<Receipt> {
    let root = models.join(DIRECTORY);
    if validate_dir(&root).is_err() {
        return Vec::new();
    }
    let Ok(entries) = fs::read_dir(&root) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().to_str()?.to_owned();
            if !hex(&name, 64) || validate_dir(&entry.path()).is_err() {
                return None;
            }
            let receipt =
                decode_receipt(open_read(&entry.path().join("active.json")).ok()?).ok()?;
            if key(&receipt.repo_id, &receipt.filename) != name {
                return None;
            }
            resolve(models, &receipt.repo_id, &receipt.filename)
                .ok()
                .flatten()?;
            Some(receipt)
        })
        .collect()
}

pub(super) fn resolve(models: &Path, repo_id: &str, filename: &str) -> Result<Option<PathBuf>> {
    let Some(receipt) = read_receipt(models, repo_id, filename)? else {
        return Ok(None);
    };
    let path = receipt.payload_path(models);
    ensure!(
        regular_file(&path)? == receipt.size_bytes,
        "model_storage_payload_changed"
    );
    Ok(Some(path))
}

pub(super) fn has_entry(models: &Path, repo_id: &str, filename: &str) -> Result<bool> {
    match fs::symlink_metadata(entry(models, repo_id, filename)) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}

pub(super) fn partial_size(models: &Path, repo_id: &str, filename: &str, revision: &str) -> u64 {
    let Some((_, file)) = crate::catalog::file_in_catalog(filename, Some(repo_id)) else {
        return 0;
    };
    let Some(sha256) = file.sha256.as_deref() else {
        return 0;
    };
    Receipt::new(repo_id, filename, revision, sha256, file.size_bytes)
        .ok()
        .and_then(|receipt| regular_file(&receipt.partial_path(models)).ok())
        .unwrap_or(0)
}

/// 精确删除此 repo/file 的受管文件。未知文件或子目录先拒绝，绝不递归到外部目标。
pub(super) fn remove(lease: &ModelWriteLease, repo_id: &str, filename: &str) -> Result<bool> {
    let models = lease.root();
    let directory = entry(models, repo_id, filename);
    if !directory.try_exists()? {
        return Ok(false);
    }
    validate_dir(&models.join(DIRECTORY))?;
    validate_dir(&directory)?;
    read_receipt(models, repo_id, filename)?;
    let mut paths = Vec::new();
    for item in fs::read_dir(&directory)? {
        let item = item?;
        let name = item.file_name();
        let name = name.to_str().context("model_storage_unknown_file")?;
        if name != "active.json" && !generation_name(name) && !temporary_name(name) {
            bail!("model_storage_unknown_file");
        }
        regular_file(&item.path())?;
        paths.push(item.path());
    }
    // 先撤销索引。中断后不会把只剩一半的代际暴露给加载器。
    paths.sort_by_key(|path| path.file_name().is_none_or(|name| name != "active.json"));
    for path in paths {
        fs::remove_file(path)?;
    }
    sync_dir(&directory)?;
    fs::remove_dir(&directory)?;
    sync_dir(&models.join(DIRECTORY))?;
    Ok(true)
}

fn generation_name(name: &str) -> bool {
    let Some(stem) = name
        .strip_suffix(".gguf")
        .or_else(|| name.strip_suffix(".partial"))
    else {
        return false;
    };
    let parts: Vec<_> = stem.split('-').collect();
    parts.len() == 3
        && hex(parts[0], 40)
        && hex(parts[1], 64)
        && parts[2]
            .parse::<u64>()
            .is_ok_and(|size| size > 0 && size <= (1_u64 << 50))
}

fn temporary_name(name: &str) -> bool {
    name.strip_prefix(".active-")
        .and_then(|name| name.strip_suffix(".tmp"))
        .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
}

#[cfg(test)]
mod tests;
