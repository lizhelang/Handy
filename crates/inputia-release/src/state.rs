//! 每用户更新信任账本。根链和各频道高水位在返回授权前落盘；任何存储失败使句柄失效。
use crate::{
    canonical,
    feed::{AuthorizedReleaseMetadata, FeedCheckpoint, Host, ReleaseDocuments, VerifiedFeed},
    trust::{Track, TrustRoot, VerifiedKeyset},
    valid_digest, ReleaseError, Result,
};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use std::{
    ffi::CString,
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Component, Path},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    schema_version: u32,
    product_id: String,
    keyset_chain: Vec<String>,
    feeds: Vec<FeedCheckpoint>,
    trusted_time: i64,
}
impl Default for State {
    fn default() -> Self {
        Self {
            schema_version: 1,
            product_id: "com.inputia".into(),
            keyset_chain: vec![],
            feeds: vec![],
            trusted_time: 0,
        }
    }
}

/// 锁覆盖读取、验证和更新；调用者应在网络下载前准备数据，避免长时间持锁。
pub struct TrustStore {
    directory: File,
    _lock: File,
    uid: u32,
    state: State,
    root: TrustRoot,
    current: Option<VerifiedKeyset>,
    poisoned: bool,
}

/// 只有已持久提交的频道才可流向本存储的 metadata 入口；不提供公开构造器。
pub struct CommittedFeed(VerifiedFeed);
impl CommittedFeed {
    pub fn verified(&self) -> &VerifiedFeed {
        &self.0
    }
}

impl TrustStore {
    pub fn open(home: &Path, embedded: TrustRoot) -> Result<Self> {
        // SAFETY: geteuid 无参数且只读内核当前身份。
        let uid = unsafe { libc::geteuid() };
        let directory = open_directory(
            &home.join("Library/Application Support/Inputia/UpdateTrust"),
            uid,
        )?;
        let lock = open_file(&directory, "state.lock", libc::O_RDWR | libc::O_CREAT, uid)?
            .ok_or(ReleaseError::StateUnavailable)?;
        // SAFETY: 活跃 File 持有 fd；非阻塞排他锁在 File drop 时释放。
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(ReleaseError::StateUnavailable);
        }
        let initialized = read(&directory, "initialized.json", uid)?;
        if initialized
            .as_deref()
            .is_some_and(|bytes| bytes != b"{\"product_id\":\"com.inputia\",\"schema_version\":1}")
        {
            return Err(ReleaseError::StateUnavailable);
        }
        let state = match read(&directory, "state.json", uid)? {
            Some(bytes) => serde_json::from_value::<State>(canonical::parse(&bytes)?)
                .map_err(|_| ReleaseError::InvalidDocument)?,
            None if initialized.is_some() => return Err(ReleaseError::StateUnavailable),
            None => {
                let state = State::default();
                atomic_write(
                    &directory,
                    "state.json",
                    &canonical::encode(
                        &serde_json::to_value(&state).map_err(|_| ReleaseError::InvalidDocument)?,
                    )?,
                    uid,
                )?;
                state
            }
        };
        if state.schema_version != 1
            || state.product_id != "com.inputia"
            || state.trusted_time < 0
            || state.keyset_chain.len() > 1024
            || state.feeds.len() > 16
        {
            return Err(ReleaseError::InvalidDocument);
        }
        let mut root = embedded;
        let mut current = None;
        for expected in &state.keyset_chain {
            if !valid_digest(expected) {
                return Err(ReleaseError::InvalidDocument);
            }
            let bytes = read(&directory, &format!("keyset-{expected}.json"), uid)?
                .ok_or(ReleaseError::StateUnavailable)?;
            let next = root.advance(&bytes)?;
            if next.checkpoint().document_digest != *expected
                || next.checkpoint().version
                    != current
                        .as_ref()
                        .map(|k: &VerifiedKeyset| k.checkpoint().version + 1)
                        .unwrap_or(1)
            {
                return Err(ReleaseError::DigestMismatch);
            }
            root = next.next_root().clone();
            current = Some(next);
        }
        let mut tracks = std::collections::BTreeSet::new();
        for checkpoint in &state.feeds {
            checkpoint.track.validate()?;
            if !tracks.insert(checkpoint.track.clone())
                || checkpoint.sequence == 0
                || checkpoint.sequence > 9_007_199_254_740_991
                || !valid_digest(&checkpoint.document_digest)
                || current.is_none()
            {
                return Err(ReleaseError::InvalidDocument);
            }
        }
        if initialized.is_none() {
            publish_new(
                &directory,
                "initialized.json",
                b"{\"product_id\":\"com.inputia\",\"schema_version\":1}",
                uid,
            )?;
        }
        Ok(Self {
            directory,
            _lock: lock,
            uid,
            state,
            root,
            current,
            poisoned: false,
        })
    }
    pub fn effective_time(&self, wall_clock: i64) -> Result<i64> {
        if self.poisoned || wall_clock < 0 {
            return Err(ReleaseError::StateUnavailable);
        }
        Ok(wall_clock.max(self.state.trusted_time))
    }
    pub fn current_keyset(&self) -> Option<&VerifiedKeyset> {
        if self.poisoned {
            None
        } else {
            self.current.as_ref()
        }
    }
    fn persist(&mut self, next: &State) -> Result<()> {
        if self.poisoned {
            return Err(ReleaseError::StateUnavailable);
        }
        let bytes = canonical::encode(
            &serde_json::to_value(next).map_err(|_| ReleaseError::InvalidDocument)?,
        )?;
        if let Err(error) = atomic_write(&self.directory, "state.json", &bytes, self.uid) {
            self.poisoned = true;
            return Err(error);
        }
        Ok(())
    }
    /// 网络等待结束后先记录观察时间；即使后续验证失败，也不允许回拨恢复已过期授权。
    pub fn observe_time(&mut self, wall_clock: i64) -> Result<i64> {
        let now = self.effective_time(wall_clock)?;
        if now > self.state.trusted_time {
            let mut next = self.state.clone();
            next.trusted_time = now;
            self.persist(&next)?;
            self.state = next;
        }
        Ok(now)
    }
    /// 允许逐个提交已过期中间根；它只能继续轮换，不能签发新的频道授权。
    pub fn advance_keyset(&mut self, bytes: &[u8]) -> Result<&VerifiedKeyset> {
        if self.poisoned {
            return Err(ReleaseError::StateUnavailable);
        }
        let next = self.root.advance(bytes)?;
        let checkpoint = next.checkpoint();
        if self
            .current
            .as_ref()
            .is_some_and(|k| k.checkpoint().document_digest == checkpoint.document_digest)
        {
            return self.current.as_ref().ok_or(ReleaseError::StateUnavailable);
        }
        if self.state.keyset_chain.len() >= 1024 {
            return Err(ReleaseError::StateUnavailable);
        }
        let name = format!("keyset-{}.json", checkpoint.document_digest);
        if let Some(existing) = read(&self.directory, &name, self.uid)? {
            // 中断写入留下的孤立文件只按同一已签语义重用。
            if self.root.advance(&existing)?.checkpoint().document_digest
                != checkpoint.document_digest
            {
                return Err(ReleaseError::DigestMismatch);
            }
        } else if let Err(error) = publish_new(&self.directory, &name, bytes, self.uid) {
            self.poisoned = true;
            return Err(error);
        }
        let mut state = self.state.clone();
        state.keyset_chain.push(checkpoint.document_digest);
        self.persist(&state)?;
        self.state = state;
        self.root = next.next_root().clone();
        self.current = Some(next);
        self.current.as_ref().ok_or(ReleaseError::StateUnavailable)
    }
    pub fn commit_feed(
        &mut self,
        bytes: &[u8],
        track: &Track,
        wall_clock: i64,
    ) -> Result<CommittedFeed> {
        let now = self.observe_time(wall_clock)?;
        let keyset = self.current.as_ref().ok_or(ReleaseError::UntrustedKey)?;
        let verified = keyset.authorize_feed(bytes, track, now)?;
        verified.check_high_water(self.state.feeds.iter().find(|feed| &feed.track == track))?;
        let mut next = self.state.clone();
        next.trusted_time = now;
        next.feeds.retain(|feed| &feed.track != track);
        next.feeds.push(verified.checkpoint());
        next.feeds.sort_by(|a, b| a.track.cmp(&b.track));
        self.persist(&next)?;
        self.state = next;
        Ok(CommittedFeed(verified))
    }
    pub fn authorize_release_metadata(
        &mut self,
        feed: &CommittedFeed,
        documents: &ReleaseDocuments<'_>,
        host: &Host<'_>,
        wall_clock: i64,
    ) -> Result<AuthorizedReleaseMetadata> {
        let now = self.observe_time(wall_clock)?;
        let checkpoint = feed.0.checkpoint();
        if !self.state.feeds.contains(&checkpoint) {
            return Err(ReleaseError::Replay);
        }
        self.current
            .as_ref()
            .ok_or(ReleaseError::UntrustedKey)?
            .authorize_release_metadata(&feed.0, documents, host, now)
    }
}

fn unavailable<T>(_: T) -> ReleaseError {
    ReleaseError::StateUnavailable
}
fn name(value: &str) -> Result<CString> {
    if value.contains('/') {
        return Err(ReleaseError::UnsafePath);
    }
    CString::new(value).map_err(|_| ReleaseError::UnsafePath)
}
fn open_directory(path: &Path, uid: u32) -> Result<File> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
    {
        return Err(ReleaseError::UnsafePath);
    }
    let mut directory = File::open("/").map_err(unavailable)?;
    for part in path.components().filter_map(|part| {
        if let Component::Normal(p) = part {
            Some(p)
        } else {
            None
        }
    }) {
        let child = CString::new(part.as_bytes()).map_err(|_| ReleaseError::UnsafePath)?;
        // SAFETY: 固定父目录 fd，名称单段；NOFOLLOW 拒绝祖先符号链接。
        let mut fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                child.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
            if unsafe { libc::mkdirat(directory.as_raw_fd(), child.as_ptr(), 0o700) } != 0
                && std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST)
            {
                return Err(ReleaseError::StateUnavailable);
            }
            directory.sync_all().map_err(unavailable)?;
            fd = unsafe {
                libc::openat(
                    directory.as_raw_fd(),
                    child.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
        }
        if fd < 0 {
            return Err(ReleaseError::UnsafePath);
        }
        let next = unsafe { File::from_raw_fd(fd) };
        let meta = next.metadata().map_err(unavailable)?;
        if !meta.is_dir()
            || (meta.uid() != 0 && meta.uid() != uid)
            || (meta.mode() & 0o022 != 0 && !(meta.uid() == 0 && meta.mode() & 0o1000 != 0))
        {
            return Err(ReleaseError::UnsafePath);
        }
        directory = next;
    }
    let meta = directory.metadata().map_err(unavailable)?;
    if meta.uid() != uid || meta.mode() & 0o077 != 0 {
        return Err(ReleaseError::UnsafePath);
    }
    Ok(directory)
}
fn open_file(directory: &File, leaf: &str, flags: i32, uid: u32) -> Result<Option<File>> {
    let leaf = name(leaf)?;
    // SAFETY: 单段名称，固定目录 fd。NONBLOCK 防止被 FIFO 阻塞。
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            leaf.as_ptr(),
            flags | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
            0o600,
        )
    };
    if fd < 0 {
        return if std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
            Ok(None)
        } else {
            Err(ReleaseError::StateUnavailable)
        };
    }
    let file = unsafe { File::from_raw_fd(fd) };
    let meta = file.metadata().map_err(unavailable)?;
    if !meta.is_file()
        || meta.uid() != uid
        || meta.nlink() != 1
        || meta.mode() & 0o077 != 0
        || meta.len() > canonical::MAX_DOCUMENT_BYTES as u64
    {
        return Err(ReleaseError::UnsafePath);
    }
    Ok(Some(file))
}
fn read(directory: &File, leaf: &str, uid: u32) -> Result<Option<Vec<u8>>> {
    let Some(file) = open_file(directory, leaf, libc::O_RDONLY, uid)? else {
        return Ok(None);
    };
    let mut bytes = Vec::new();
    file.take(canonical::MAX_DOCUMENT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(unavailable)?;
    if bytes.len() > canonical::MAX_DOCUMENT_BYTES {
        return Err(ReleaseError::InvalidDocument);
    }
    Ok(Some(bytes))
}
fn write_new(directory: &File, leaf: &str, bytes: &[u8], uid: u32) -> Result<()> {
    let mut file = open_file(
        directory,
        leaf,
        libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
        uid,
    )?
    .ok_or(ReleaseError::StateUnavailable)?;
    file.write_all(bytes).map_err(unavailable)?;
    file.sync_all().map_err(unavailable)?;
    directory.sync_all().map_err(unavailable)
}
fn atomic_write(directory: &File, leaf: &str, bytes: &[u8], uid: u32) -> Result<()> {
    // 旧文件必须先通过身份校验；损坏/软硬链接不能被悄悄覆盖。
    let _ = open_file(directory, leaf, libc::O_RDONLY, uid)?;
    let mut nonce = [0u8; 16];
    SystemRandom::new().fill(&mut nonce).map_err(unavailable)?;
    let temp = format!("pending-{}", crate::digest(&nonce));
    write_new(directory, &temp, bytes, uid)?;
    let old = name(&temp)?;
    let new = name(leaf)?;
    // SAFETY: 两端均为固定私有目录内单段名称，原子替换仅用于本产品信任账本。
    if unsafe {
        libc::renameat(
            directory.as_raw_fd(),
            old.as_ptr(),
            directory.as_raw_fd(),
            new.as_ptr(),
        )
    } != 0
    {
        return Err(ReleaseError::StateUnavailable);
    }
    directory.sync_all().map_err(unavailable)
}

fn publish_new(directory: &File, leaf: &str, bytes: &[u8], uid: u32) -> Result<()> {
    let mut nonce = [0u8; 16];
    SystemRandom::new().fill(&mut nonce).map_err(unavailable)?;
    let temp = format!("pending-{}", crate::digest(&nonce));
    write_new(directory, &temp, bytes, uid)?;
    let old = name(&temp)?;
    let new = name(leaf)?;
    // SAFETY: 固定私有目录内原子不覆盖发布；崩溃只留下未引用的临时文件或完整 keyset。
    #[cfg(target_os = "macos")]
    let result = unsafe {
        libc::renameatx_np(
            directory.as_raw_fd(),
            old.as_ptr(),
            directory.as_raw_fd(),
            new.as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    #[cfg(target_os = "linux")]
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            directory.as_raw_fd(),
            old.as_ptr(),
            directory.as_raw_fd(),
            new.as_ptr(),
            libc::RENAME_NOREPLACE,
        ) as i32
    };
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let result = -1;
    if result != 0 {
        return Err(ReleaseError::StateUnavailable);
    }
    directory.sync_all().map_err(unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        feed::Feed,
        trust::{tests::*, DocumentKind},
    };
    use std::os::unix::fs::{symlink, PermissionsExt};
    fn fixture() -> (TrustRoot, Vec<u8>, SigningKey, SigningKey, SigningKey) {
        let (root, archive, online) = (SigningKey::new(), SigningKey::new(), SigningKey::new());
        let embedded =
            TrustRoot::from_embedded("com.inputia", vec![root.public.clone()], 1).unwrap();
        let bytes = signed(
            DocumentKind::Keyset,
            serde_json::to_value(payload(&root, &archive, &online)).unwrap(),
            &[&root],
        );
        (embedded, bytes, root, archive, online)
    }
    fn feed(store: &TrustStore, online: &SigningKey, sequence: u64, channel: &str) -> Vec<u8> {
        let checkpoint = store.current_keyset().unwrap().checkpoint();
        signed(
            DocumentKind::Feed,
            serde_json::to_value(Feed {
                schema_version: 2,
                product_id: "com.inputia".into(),
                channel: channel.into(),
                platform: "macos".into(),
                architecture: "arm64".into(),
                sequence,
                keyset_version: checkpoint.version,
                keyset_digest: checkpoint.document_digest,
                archive_policy_id: "release-2026".into(),
                issued_at: "2026-09-30T00:00:00Z".into(),
                expires_at: "2026-10-02T00:00:00Z".into(),
                release_id: "inputia-test-current".into(),
                manifest_digest: "a".repeat(64),
                attestation_digest: "b".repeat(64),
                release_path: "releases/inputia-test-current".into(),
                rollback: None,
            })
            .unwrap(),
            &[online],
        )
    }
    #[test]
    fn restart_preserves_per_track_sequence_time_and_process_lock() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().canonicalize().unwrap();
        let (embedded, _bytes, root, archive, online) = fixture();
        let mut value = payload(&root, &archive, &online);
        value.roles.feeds.push(crate::trust::FeedRole {
            track: Track {
                channel: "stable".into(),
                ..candidate()
            },
            role: role(&online),
        });
        let bytes = signed(
            DocumentKind::Keyset,
            serde_json::to_value(value).unwrap(),
            &[&root],
        );
        let mut store = TrustStore::open(&home, embedded.clone()).unwrap();
        assert!(TrustStore::open(&home, embedded.clone()).is_err());
        store.advance_keyset(&bytes).unwrap();
        let candidate_bytes = feed(&store, &online, 5, "candidate");
        store
            .commit_feed(&candidate_bytes, &candidate(), now() + 60)
            .unwrap();
        let stable = Track {
            channel: "stable".into(),
            ..candidate()
        };
        let stable_bytes = feed(&store, &online, 2, "stable");
        store.commit_feed(&stable_bytes, &stable, now()).unwrap();
        drop(store);
        let mut store = TrustStore::open(&home, embedded).unwrap();
        assert_eq!(store.effective_time(now() - 3600).unwrap(), now() + 60);
        store
            .commit_feed(&candidate_bytes, &candidate(), now())
            .unwrap();
        let lower = feed(&store, &online, 4, "candidate");
        assert!(store.commit_feed(&lower, &candidate(), now()).is_err());
        let lower = feed(&store, &online, 1, "stable");
        assert!(store.commit_feed(&lower, &stable, now()).is_err());
        assert!(store
            .commit_feed(&candidate_bytes, &candidate(), now() + 3 * 86400)
            .is_err());
        assert!(store
            .commit_feed(&candidate_bytes, &candidate(), now())
            .is_err());
    }
    #[test]
    fn orphan_keyset_is_reusable_but_partial_temporary_file_never_becomes_state() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().canonicalize().unwrap();
        let (embedded, bytes, _, _, online) = fixture();
        let mut store = TrustStore::open(&home, embedded.clone()).unwrap();
        let verified = embedded.advance(&bytes).unwrap();
        publish_new(
            &store.directory,
            &format!("keyset-{}.json", verified.checkpoint().document_digest),
            &bytes,
            store.uid,
        )
        .unwrap();
        write_new(&store.directory, "pending-interrupted", b"{", store.uid).unwrap();
        drop(store);
        store = TrustStore::open(&home, embedded).unwrap();
        assert!(store.current_keyset().is_none());
        store.advance_keyset(&bytes).unwrap();
        let raw = feed(&store, &online, 1, "candidate");
        store.commit_feed(&raw, &candidate(), now()).unwrap();
    }
    #[test]
    fn corrupt_state_or_linked_storage_is_rejected_without_resetting_history() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().canonicalize().unwrap();
        let (embedded, bytes, _, _, online) = fixture();
        let mut store = TrustStore::open(&home, embedded.clone()).unwrap();
        store.advance_keyset(&bytes).unwrap();
        let raw = feed(&store, &online, 3, "candidate");
        store.commit_feed(&raw, &candidate(), now()).unwrap();
        drop(store);
        let state = home.join("Library/Application Support/Inputia/UpdateTrust/state.json");
        let valid = std::fs::read(&state).unwrap();
        std::fs::remove_file(&state).unwrap();
        assert!(TrustStore::open(&home, embedded.clone()).is_err());
        std::fs::write(&state, &valid).unwrap();
        std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::fs::write(&state, b"{}").unwrap();
        assert!(TrustStore::open(&home, embedded.clone()).is_err());
        std::fs::write(&state, &valid).unwrap();
        std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(TrustStore::open(&home, embedded.clone()).is_err());
        std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o600)).unwrap();
        let backup = home.join("state-backup");
        std::fs::rename(&state, &backup).unwrap();
        symlink(&backup, &state).unwrap();
        assert!(TrustStore::open(&home, embedded).is_err());
        assert_eq!(std::fs::read(&backup).unwrap(), valid);
    }
}
