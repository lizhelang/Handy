use crate::{ArchiveEntry, ArchiveKind, Error, Fingerprint, Result};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    ffi::{CStr, CString, OsString},
    fs::{File, Metadata},
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            ffi::{OsStrExt, OsStringExt},
            fs::{MetadataExt, PermissionsExt},
        },
    },
    path::{Component, Path, PathBuf},
};

pub(crate) const MAX_TREE_BYTES: u64 = 32 * 1024 * 1024 * 1024;
pub(crate) const MAX_TREE_ENTRIES: u64 = 300_000;
pub(crate) fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
pub(crate) fn valid_sha(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(crate) fn canonical(path: &Path) -> Result<()> {
    let text = path.to_str().ok_or(Error::UnsafePath)?;
    if !path.is_absolute()
        || text == "/"
        || text.ends_with('/')
        || text.contains("//")
        || text.chars().any(char::is_control)
        || text.split('/').any(|s| s == "." || s == "..")
    {
        return Err(Error::UnsafePath);
    }
    Ok(())
}
fn name(value: &std::ffi::OsStr) -> Result<CString> {
    CString::new(value.as_bytes()).map_err(|_| Error::UnsafePath)
}
fn last_error() -> Error {
    std::io::Error::last_os_error().into()
}
fn from_fd(fd: i32) -> Result<File> {
    if fd < 0 {
        Err(last_error())
    } else {
        // SAFETY: fd 为本函数调用者新创建的唯一描述符。
        Ok(unsafe { File::from_raw_fd(fd) })
    }
}
pub(crate) fn open_child(parent: &File, child: &CStr, directory: bool) -> Result<File> {
    // SAFETY: 描述符与 NUL 名称在调用期间有效；拒绝符号链接和特殊阻塞文件。
    from_fd(unsafe {
        libc::openat(
            parent.as_raw_fd(),
            child.as_ptr(),
            libc::O_RDONLY
                | libc::O_CLOEXEC
                | libc::O_NOFOLLOW
                | libc::O_NONBLOCK
                | if directory { libc::O_DIRECTORY } else { 0 },
        )
    })
}
fn safe_parent(meta: &Metadata, uid: u32) -> Result<()> {
    if !meta.is_dir() || (meta.uid() != uid && meta.uid() != 0) {
        return Err(Error::UnsafePath);
    }
    if meta.mode() & 0o022 != 0 && !(meta.uid() == 0 && meta.mode() & 0o1000 != 0) {
        return Err(Error::UnsafePath);
    }
    Ok(())
}
fn safe_artifact(meta: &Metadata, uid: u32) -> Result<()> {
    if meta.uid() != uid {
        return Err(Error::PermissionRequired);
    }
    if !(meta.is_file() || meta.is_dir())
        || (meta.is_file() && meta.nlink() != 1)
        || meta.mode() & 0o7022 != 0
    {
        return Err(Error::UnsafePath);
    }
    Ok(())
}

/// 每次从 / 逐层打开并固定父目录 fd，后续创建/rename 都相对此 fd，避免再次解释路径链。
pub(crate) fn parent(path: &Path, uid: u32, create: bool) -> Result<(File, CString)> {
    canonical(path)?;
    let parts: Vec<_> = path
        .components()
        .filter_map(|c| {
            if let Component::Normal(p) = c {
                Some(p)
            } else {
                None
            }
        })
        .collect();
    let leaf = name(parts.last().ok_or(Error::UnsafePath)?)?;
    let mut cursor = File::open("/")?;
    for part in &parts[..parts.len() - 1] {
        let child = name(part)?;
        let next = match open_child(&cursor, &child, true) {
            Ok(next) => next,
            Err(Error::MissingArtifact) if create => {
                // SAFETY: 父句柄固定且名称为单段；EEXIST 仍需重新安全打开校验。
                let status = unsafe { libc::mkdirat(cursor.as_raw_fd(), child.as_ptr(), 0o700) };
                if status != 0
                    && std::io::Error::last_os_error().raw_os_error() != Some(libc::EEXIST)
                {
                    return Err(last_error());
                }
                cursor.sync_all()?;
                open_child(&cursor, &child, true)?
            }
            Err(e) => return Err(e),
        };
        safe_parent(&next.metadata()?, uid)?;
        cursor = next;
    }
    safe_parent(&cursor.metadata()?, uid)?;
    Ok((cursor, leaf))
}
pub(crate) fn validate_prefix(path: &Path, uid: u32) -> Result<()> {
    canonical(path)?;
    let mut cursor = File::open("/")?;
    let parts: Vec<_> = path
        .components()
        .filter_map(|c| {
            if let Component::Normal(p) = c {
                Some(p)
            } else {
                None
            }
        })
        .collect();
    for part in &parts[..parts.len() - 1] {
        match open_child(&cursor, &name(part)?, true) {
            Ok(next) => {
                safe_parent(&next.metadata()?, uid)?;
                cursor = next;
            }
            Err(Error::MissingArtifact) => return Ok(()),
            Err(e) => return Err(e),
        }
    }
    Ok(())
}
pub(crate) fn mkdir_new(path: &Path, uid: u32) -> Result<()> {
    let (parent, leaf) = parent(path, uid, true)?;
    // SAFETY: 同上；不得复用已存在的事务目录。
    if unsafe { libc::mkdirat(parent.as_raw_fd(), leaf.as_ptr(), 0o700) } != 0 {
        return if std::io::Error::last_os_error().raw_os_error() == Some(libc::EEXIST) {
            Err(Error::TransactionReused)
        } else {
            Err(last_error())
        };
    }
    parent.sync_all()?;
    Ok(())
}
pub(crate) fn ensure_dir(path: &Path, uid: u32) -> Result<()> {
    let _ = parent(&path.join(".probe"), uid, true)?;
    Ok(())
}
fn children(directory: &File) -> Result<Vec<OsString>> {
    // fdopendir 取得复制 fd 的所有权；closedir 恰好释放一次。
    let fd = unsafe { libc::dup(directory.as_raw_fd()) };
    if fd < 0 {
        return Err(last_error());
    }
    let ptr = unsafe { libc::fdopendir(fd) };
    if ptr.is_null() {
        unsafe { libc::close(fd) };
        return Err(last_error());
    }
    struct Directory(*mut libc::DIR);
    impl Drop for Directory {
        fn drop(&mut self) {
            unsafe {
                libc::closedir(self.0);
            }
        }
    }
    let guard = Directory(ptr);
    // `dup` 共享目录的 open-file description；每次摘要都必须从头枚举，
    // 否则对同一个 held fd 的第二次复核会错误地看到空目录。
    unsafe { libc::rewinddir(guard.0) };
    let mut names = Vec::new();
    loop {
        // errno 只在 readdir 返回空指针时读取；区分目录结束与读取失败。
        #[cfg(target_os = "macos")]
        unsafe {
            *libc::__error() = 0;
        }
        #[cfg(target_os = "linux")]
        unsafe {
            *libc::__errno_location() = 0;
        }
        let entry = unsafe { libc::readdir(guard.0) };
        if entry.is_null() {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(0) {
                return Err(error.into());
            }
            break;
        }
        let bytes = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
        if bytes != b"." && bytes != b".." {
            names.push(OsString::from_vec(bytes.to_vec()));
        }
        if names.len() > MAX_TREE_ENTRIES as usize {
            return Err(Error::Invalid("tree entry budget"));
        }
    }
    names.sort();
    Ok(names)
}
fn feed(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}
#[allow(clippy::too_many_arguments)] // 保持既有树摘要字节合同，仅附加取消检查。
fn hash_node(
    mut file: File,
    relative: &Path,
    uid: u32,
    hasher: &mut Sha256,
    bytes: &mut u64,
    entries: &mut u64,
    nodes: &mut BTreeMap<PathBuf, NodeKind>,
    check: &dyn Fn() -> Result<()>,
) -> Result<()> {
    check()?;
    if relative.components().count() > 128 {
        return Err(Error::Invalid("tree depth budget"));
    }
    let before = file.metadata()?;
    safe_artifact(&before, uid)?;
    *entries += 1;
    if *entries > MAX_TREE_ENTRIES {
        return Err(Error::Invalid("tree entry budget"));
    }
    feed(hasher, relative.as_os_str().as_bytes());
    hasher.update((before.mode() & 0o777).to_le_bytes());
    if before.is_dir() {
        hasher.update(b"D");
        nodes.insert(relative.to_owned(), NodeKind::Directory);
        for child in children(&file)? {
            let child_name = name(&child)?;
            let path = relative.join(&child);
            if let Some(target) = read_link(&file, &child_name, uid)? {
                *entries += 1;
                *bytes = bytes
                    .checked_add(target.as_os_str().len() as u64)
                    .ok_or(Error::Invalid("link size overflow"))?;
                if *entries > MAX_TREE_ENTRIES || *bytes > MAX_TREE_BYTES {
                    return Err(Error::Invalid("tree link budget"));
                }
                feed(hasher, path.as_os_str().as_bytes());
                hasher.update(0o777u32.to_le_bytes());
                hasher.update(b"L");
                feed(hasher, target.as_os_str().as_bytes());
                nodes.insert(path, NodeKind::Link(target));
            } else {
                hash_node(
                    open_child(&file, &child_name, false)?,
                    &path,
                    uid,
                    hasher,
                    bytes,
                    entries,
                    nodes,
                    check,
                )?;
            }
        }
    } else {
        hasher.update(b"F");
        nodes.insert(relative.to_owned(), NodeKind::File);
        hasher.update(before.len().to_le_bytes());
        *bytes = bytes
            .checked_add(before.len())
            .ok_or(Error::Invalid("tree size overflow"))?;
        if *bytes > MAX_TREE_BYTES {
            return Err(Error::Invalid("tree byte budget"));
        }
        let mut input = (&mut file).take(before.len().saturating_add(1));
        let mut buffer = [0u8; 65536];
        let mut read = 0;
        loop {
            check()?;
            let n = input.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
            read += n as u64;
        }
        if read != before.len() {
            return Err(Error::ArtifactMismatch);
        }
    }
    let after = file.metadata()?;
    if before.ino() != after.ino()
        || before.len() != after.len()
        || before.mtime() != after.mtime()
        || before.mtime_nsec() != after.mtime_nsec()
        || before.ctime() != after.ctime()
        || before.ctime_nsec() != after.ctime_nsec()
    {
        return Err(Error::ArtifactMismatch);
    }
    Ok(())
}
pub fn fingerprint(path: &Path, uid: u32) -> Result<Fingerprint> {
    let (parent, leaf) = parent(path, uid, false)?;
    let file = open_child(&parent, &leaf, false)?;
    fingerprint_file(file, uid, &|| Ok(()))
}

/// 固定已打开的根 fd，供可取消的归档解包验证复用。
pub(crate) fn fingerprint_file(
    file: File,
    uid: u32,
    check: &dyn Fn() -> Result<()>,
) -> Result<Fingerprint> {
    let mut hasher = Sha256::new();
    hasher.update(b"Inputia-Artifact-Tree-v1\0");
    let (mut bytes, mut entries) = (0, 0);
    let mut nodes = BTreeMap::new();
    hash_node(
        file,
        Path::new(""),
        uid,
        &mut hasher,
        &mut bytes,
        &mut entries,
        &mut nodes,
        check,
    )?;
    validate_links(&nodes)?;
    Ok(Fingerprint {
        sha256: hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
        bytes,
        entries,
    })
}
pub(crate) fn maybe_fingerprint(path: &Path, uid: u32) -> Result<Option<Fingerprint>> {
    match fingerprint(path, uid) {
        Ok(value) => Ok(Some(value)),
        Err(Error::MissingArtifact) => Ok(None),
        Err(e) => Err(e),
    }
}
pub(crate) fn read(path: &Path, uid: u32, limit: usize) -> Result<Vec<u8>> {
    let (parent, leaf) = parent(path, uid, false)?;
    let file = open_child(&parent, &leaf, false)?;
    let meta = file.metadata()?;
    safe_artifact(&meta, uid)?;
    if !meta.is_file() || meta.len() > limit as u64 || meta.mode() & 0o077 != 0 {
        return Err(Error::UnsafePath);
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(Error::Invalid("file byte budget"));
    }
    Ok(bytes)
}
pub(crate) fn write_new(path: &Path, uid: u32, bytes: &[u8]) -> Result<()> {
    let (parent, leaf) = parent(path, uid, true)?;
    let mut file = from_fd(unsafe {
        libc::openat(
            parent.as_raw_fd(),
            leaf.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0o600,
        )
    })?;
    file.write_all(bytes)?;
    file.sync_all()?;
    parent.sync_all()?;
    Ok(())
}
pub(crate) fn sync_file(path: &Path, uid: u32) -> Result<()> {
    let (parent, leaf) = parent(path, uid, false)?;
    let file = open_child(&parent, &leaf, false)?;
    safe_artifact(&file.metadata()?, uid)?;
    file.sync_all()?;
    parent.sync_all()?;
    Ok(())
}
pub(crate) fn rename(source: &Path, destination: &Path, uid: u32, replace: bool) -> Result<()> {
    let (from, from_name) = parent(source, uid, false)?;
    let (to, to_name) = parent(destination, uid, false)?;
    let status = if replace {
        unsafe {
            libc::renameat(
                from.as_raw_fd(),
                from_name.as_ptr(),
                to.as_raw_fd(),
                to_name.as_ptr(),
            )
        }
    } else {
        #[cfg(target_os = "macos")]
        {
            unsafe {
                libc::renameatx_np(
                    from.as_raw_fd(),
                    from_name.as_ptr(),
                    to.as_raw_fd(),
                    to_name.as_ptr(),
                    libc::RENAME_EXCL,
                )
            }
        }
        #[cfg(target_os = "linux")]
        {
            unsafe {
                libc::syscall(
                    libc::SYS_renameat2,
                    from.as_raw_fd(),
                    from_name.as_ptr(),
                    to.as_raw_fd(),
                    to_name.as_ptr(),
                    1u32,
                ) as i32
            }
        }
    };
    if status != 0 {
        return Err(last_error());
    }
    from.sync_all()?;
    to.sync_all()?;
    Ok(())
}
pub(crate) fn unlink(path: &Path, uid: u32) -> Result<()> {
    let (parent, leaf) = parent(path, uid, false)?;
    if unsafe { libc::unlinkat(parent.as_raw_fd(), leaf.as_ptr(), 0) } != 0 {
        return Err(last_error());
    }
    parent.sync_all()?;
    Ok(())
}
pub(crate) fn atomic_write(path: &Path, uid: u32, bytes: &[u8]) -> Result<()> {
    let temp = path.with_file_name(format!(".journal-{}.tmp", uuid::Uuid::new_v4()));
    write_new(&temp, uid, bytes)?;
    rename(&temp, path, uid, true)
}
fn copy_node(
    source: &File,
    destination_parent: &File,
    leaf: &CStr,
    uid: u32,
    used: &mut (u64, u64),
    depth: usize,
) -> Result<()> {
    if depth > 128 {
        return Err(Error::Invalid("copy depth budget"));
    }
    let meta = source.metadata()?;
    safe_artifact(&meta, uid)?;
    used.0 = used
        .0
        .checked_add(if meta.is_file() { meta.len() } else { 0 })
        .ok_or(Error::Invalid("copy size overflow"))?;
    used.1 += 1;
    if used.0 > MAX_TREE_BYTES || used.1 > MAX_TREE_ENTRIES {
        return Err(Error::Invalid("copy budget"));
    }
    if meta.is_dir() {
        if unsafe { libc::mkdirat(destination_parent.as_raw_fd(), leaf.as_ptr(), 0o700) } != 0 {
            return Err(last_error());
        }
        let destination = open_child(destination_parent, leaf, true)?;
        for child in children(source)? {
            let child = name(&child)?;
            if let Some(target) = read_link(source, &child, uid)? {
                used.0 += target.as_os_str().len() as u64;
                used.1 += 1;
                if used.0 > MAX_TREE_BYTES || used.1 > MAX_TREE_ENTRIES {
                    return Err(Error::Invalid("copy link budget"));
                }
                let target = name(target.as_os_str())?;
                if unsafe {
                    libc::symlinkat(target.as_ptr(), destination.as_raw_fd(), child.as_ptr())
                } != 0
                {
                    return Err(last_error());
                }
            } else {
                copy_node(
                    &open_child(source, &child, false)?,
                    &destination,
                    &child,
                    uid,
                    used,
                    depth + 1,
                )?;
            }
        }
        destination.set_permissions(std::fs::Permissions::from_mode(meta.mode() & 0o777))?;
        destination.sync_all()?;
    } else {
        let mut destination = from_fd(unsafe {
            libc::openat(
                destination_parent.as_raw_fd(),
                leaf.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        })?;
        let mut input = source.try_clone()?.take(meta.len().saturating_add(1));
        if std::io::copy(&mut input, &mut destination)? != meta.len() {
            return Err(Error::ArtifactMismatch);
        }
        destination.set_permissions(std::fs::Permissions::from_mode(meta.mode() & 0o777))?;
        destination.sync_all()?;
    }
    destination_parent.sync_all()?;
    Ok(())
}
pub(crate) fn copy(source: &Path, destination: &Path, uid: u32) -> Result<()> {
    let (from, leaf) = parent(source, uid, false)?;
    let source = open_child(&from, &leaf, false)?;
    let (to, leaf) = parent(destination, uid, true)?;
    copy_node(&source, &to, &leaf, uid, &mut (0, 0), 0)
}
pub(crate) fn lock(path: &Path, uid: u32) -> Result<File> {
    let (parent, leaf) = parent(path, uid, true)?;
    let file = from_fd(unsafe {
        libc::openat(
            parent.as_raw_fd(),
            leaf.as_ptr(),
            libc::O_RDWR | libc::O_CREAT | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0o600,
        )
    })?;
    let metadata = file.metadata()?;
    safe_artifact(&metadata, uid)?;
    if !metadata.is_file() || metadata.mode() & 0o077 != 0 {
        return Err(Error::UnsafePath);
    }
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return if std::io::Error::last_os_error().raw_os_error() == Some(libc::EWOULDBLOCK) {
            Err(Error::Busy)
        } else {
            Err(last_error())
        };
    }
    file.sync_all()?;
    parent.sync_all()?;
    Ok(file)
}
pub(crate) fn free_bytes(path: &Path, uid: u32) -> Result<u64> {
    let mut probe = path.to_path_buf();
    loop {
        match parent(&probe, uid, false) {
            Ok((file, _)) => {
                let dot = CString::new(".").map_err(|_| Error::UnsafePath)?;
                if unsafe {
                    libc::faccessat(
                        file.as_raw_fd(),
                        dot.as_ptr(),
                        libc::W_OK | libc::X_OK,
                        libc::AT_EACCESS,
                    )
                } != 0
                {
                    return Err(last_error());
                }
                let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
                if unsafe { libc::fstatvfs(file.as_raw_fd(), &mut stat) } != 0 {
                    return Err(last_error());
                }
                return Ok((stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64));
            }
            Err(Error::MissingArtifact) => {
                probe = probe.parent().ok_or(Error::UnsafePath)?.to_path_buf();
            }
            Err(e) => return Err(e),
        }
    }
}
pub(crate) fn list(path: &Path, uid: u32) -> Result<Vec<OsString>> {
    let (parent, leaf) = parent(path, uid, false)?;
    children(&open_child(&parent, &leaf, true)?)
}

#[derive(Clone)]
enum NodeKind {
    File,
    Directory,
    Link(PathBuf),
}
fn read_link(parent: &File, child: &CStr, uid: u32) -> Result<Option<PathBuf>> {
    let mut metadata: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe {
        libc::fstatat(
            parent.as_raw_fd(),
            child.as_ptr(),
            &mut metadata,
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } != 0
    {
        return Err(last_error());
    }
    if metadata.st_mode & libc::S_IFMT != libc::S_IFLNK {
        return Ok(None);
    }
    if metadata.st_uid != uid || metadata.st_nlink != 1 {
        return Err(Error::UnsafePath);
    }
    let mut bytes = [0u8; 4096];
    let count = unsafe {
        libc::readlinkat(
            parent.as_raw_fd(),
            child.as_ptr(),
            bytes.as_mut_ptr().cast(),
            bytes.len(),
        )
    };
    if count <= 0 || count as usize == bytes.len() {
        return Err(Error::UnsafePath);
    }
    let target = PathBuf::from(OsString::from_vec(bytes[..count as usize].to_vec()));
    validate_link_text(&target)?;
    Ok(Some(target))
}
fn validate_link_text(target: &Path) -> Result<()> {
    let text = target.to_str().ok_or(Error::UnsafePath)?;
    if target.is_absolute()
        || text.is_empty()
        || text.contains('\\')
        || text.chars().any(char::is_control)
        || text.len() >= 4096
    {
        return Err(Error::UnsafePath);
    }
    Ok(())
}
fn validate_links(nodes: &BTreeMap<PathBuf, NodeKind>) -> Result<()> {
    for (path, node) in nodes {
        if let NodeKind::Link(target) = node {
            validate_link_text(target)?;
            let mut current = path.parent().unwrap_or(Path::new("")).to_path_buf();
            let mut pending: VecDeque<OsString> = target
                .components()
                .map(|part| part.as_os_str().to_owned())
                .collect();
            let mut expanded = 0;
            while let Some(part) = pending.pop_front() {
                if part == "." {
                    continue;
                }
                if part == ".." {
                    if !current.pop() {
                        return Err(Error::UnsafePath);
                    }
                    continue;
                }
                current.push(part);
                match nodes.get(&current) {
                    Some(NodeKind::Link(target)) => {
                        expanded += 1;
                        if expanded > 40 {
                            return Err(Error::UnsafePath);
                        }
                        current.pop();
                        for part in target.components().rev() {
                            pending.push_front(part.as_os_str().to_owned());
                        }
                    }
                    Some(NodeKind::File) if !pending.is_empty() => return Err(Error::UnsafePath),
                    Some(_) => {}
                    None => return Err(Error::UnsafePath),
                }
            }
            if !nodes.contains_key(&current) {
                return Err(Error::UnsafePath);
            }
        }
    }
    Ok(())
}

/// 解包器必须先校验完整目录表；仅保留根内可解析的相对符号链接，拒绝硬链接/设备节点。
pub fn validate_archive_entries(entries: &[ArchiveEntry], max_bytes: u64) -> Result<()> {
    if entries.is_empty() || entries.len() as u64 > MAX_TREE_ENTRIES {
        return Err(Error::Invalid("archive entry budget"));
    }
    let mut paths = BTreeSet::new();
    let mut nodes = BTreeMap::new();
    let mut size = 0u64;
    nodes.insert(PathBuf::new(), NodeKind::Directory);
    for entry in entries {
        let text = entry.path.to_str().ok_or(Error::UnsafePath)?;
        if entry.path.components().count() > 128
            || entry.path.is_absolute()
            || text.is_empty()
            || text.contains('\\')
            || text.contains("//")
            || text.ends_with('/')
            || text.chars().any(char::is_control)
            || text.split('/').any(|p| p == "." || p == "..")
            || !paths.insert(entry.path.clone())
        {
            return Err(Error::UnsafePath);
        }
        let kind = match (&entry.kind, &entry.link_target) {
            (ArchiveKind::File, None) => NodeKind::File,
            (ArchiveKind::Directory, None) if entry.unpacked_bytes == 0 => NodeKind::Directory,
            (ArchiveKind::Symlink, Some(target)) if entry.unpacked_bytes == 0 => {
                validate_link_text(target)?;
                NodeKind::Link(target.clone())
            }
            _ => return Err(Error::UnsafePath),
        };
        nodes.insert(entry.path.clone(), kind);
        size = size
            .checked_add(entry.unpacked_bytes)
            .and_then(|size| {
                size.checked_add(
                    entry
                        .link_target
                        .as_ref()
                        .map_or(0, |p| p.as_os_str().len() as u64),
                )
            })
            .ok_or(Error::Invalid("archive size overflow"))?;
        if size > max_bytes {
            return Err(Error::Invalid("archive byte budget"));
        }
    }
    for path in &paths {
        for ancestor in path.ancestors().skip(1) {
            if let Some(node) = nodes.get(ancestor) {
                if !matches!(node, NodeKind::Directory) {
                    return Err(Error::UnsafePath);
                }
            } else {
                nodes.insert(ancestor.to_path_buf(), NodeKind::Directory);
            }
        }
    }
    validate_links(&nodes)
}
pub(crate) fn contents_fingerprint(bytes: &[u8], mode: u32) -> Fingerprint {
    let mut hasher = Sha256::new();
    hasher.update(b"Inputia-Artifact-Tree-v1\0");
    feed(&mut hasher, b"");
    hasher.update(mode.to_le_bytes());
    hasher.update(b"F");
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    Fingerprint {
        sha256: hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
        bytes: bytes.len() as u64,
        entries: 1,
    }
}
pub(crate) fn private_mode(path: &Path, uid: u32) -> Result<()> {
    let (parent, leaf) = parent(path, uid, false)?;
    let file = open_child(&parent, &leaf, false)?;
    safe_artifact(&file.metadata()?, uid)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.sync_all()?;
    Ok(())
}
pub(crate) fn read_public(path: &Path, uid: u32, limit: usize) -> Result<Vec<u8>> {
    let (parent, leaf) = parent(path, uid, false)?;
    let file = open_child(&parent, &leaf, false)?;
    let meta = file.metadata()?;
    safe_artifact(&meta, uid)?;
    if !meta.is_file() || meta.len() > limit as u64 {
        return Err(Error::UnsafePath);
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(Error::Invalid("file byte budget"));
    }
    Ok(bytes)
}
