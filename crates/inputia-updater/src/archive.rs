//! Inputia ZIP profile v1：只解包，不授予签名信任或安装权限。
//!
//! 调用方必须从已授权发布元数据取得摘要，并提供已固定的普通文件 fd。
//! 不支持 resource fork / xattr / AppleDouble；存在这些元数据时拒绝整个归档。
use crate::{filesystem, ArchiveEntry, ArchiveKind, Error, Fingerprint, Result};
use flate2::{Decompress, FlushDecompress, Status};
use rawzip::{
    extra_fields::ExtraFields, CompressionMethod, ReaderAt, ZipArchive, ZipArchiveEntryWayfinder,
    ZipLocator,
};
use sha2::{Digest, Sha256};
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    ffi::CString,
    fs::{File, Metadata, Permissions},
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{FileExt, MetadataExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;

/// 来自调用方已验证的发布描述。此类型本身不证明签名或当前安装授权。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveDigest {
    pub sha256: String,
    pub size: u64,
}

/// 限制 ZIP 输入、输出与工作量；默认值是安全上限，不是发布大小承诺。
#[derive(Debug, Clone)]
pub struct ArchiveLimits {
    pub max_source_bytes: u64,
    pub max_entries: usize,
    pub max_depth: usize,
    /// 所有唯一原始路径前缀与 Unicode 比较键的总字节预算。
    pub max_path_bytes: usize,
    pub max_file_bytes: u64,
    pub max_unpacked_bytes: u64,
    pub max_compression_ratio: u64,
    /// 包括前后两次摘要、目录和数据读取；重复读取也计费。
    pub max_source_read_bytes: u64,
}
impl Default for ArchiveLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: 4 * 1024 * 1024 * 1024 - 1,
            max_entries: 100_000,
            max_depth: 64,
            max_path_bytes: 16 * 1024 * 1024,
            max_file_bytes: 2 * 1024 * 1024 * 1024,
            max_unpacked_bytes: 8 * 1024 * 1024 * 1024,
            max_compression_ratio: 200,
            max_source_read_bytes: 16 * 1024 * 1024 * 1024,
        }
    }
}

/// 只有成功解包、fsync、源复核和树摘要后才能创建。不是 Security 或安装回执。
/// 目录仍须置于受控 staging，使用前/恢复时重新验树与原生签名。
#[derive(Debug)]
pub struct ExtractedArchive {
    destination: PathBuf,
    required_root: String,
    source: ArchiveDigest,
    outer_tree: Fingerprint,
    bundle_tree: Fingerprint,
    root: File,
    bundle: File,
    source_read_bytes: u64,
}
impl ExtractedArchive {
    pub fn destination(&self) -> &Path {
        &self.destination
    }
    pub fn required_root(&self) -> &str {
        &self.required_root
    }
    pub fn source(&self) -> &ArchiveDigest {
        &self.source
    }
    pub fn tree(&self) -> &Fingerprint {
        &self.outer_tree
    }
    /// 安装事务使用的精确 `.app` 子树摘要；`tree()` 仍表示解包容器整体。
    pub fn bundle_tree(&self) -> &Fingerprint {
        &self.bundle_tree
    }
    pub fn bundle_path(&self) -> PathBuf {
        self.destination.join(&self.required_root)
    }
    pub fn root_identity(&self) -> Result<(u64, u64)> {
        let meta = self.root.metadata()?;
        Ok((meta.dev(), meta.ino()))
    }
    pub fn bundle_identity(&self) -> Result<(u64, u64)> {
        let meta = self.bundle.metadata()?;
        Ok((meta.dev(), meta.ino()))
    }
    pub fn source_read_bytes(&self) -> u64 {
        self.source_read_bytes
    }

    /// 复核公开路径仍指向本次解包固定的外层与 bundle fd，并重算两棵树。
    pub fn verify_bound_bundle(&self, uid: u32, check: &dyn Fn() -> Result<()>) -> Result<()> {
        check()?;
        if filesystem::fingerprint_file(self.root.try_clone()?, uid, check)? != self.outer_tree
            || filesystem::fingerprint_file(self.bundle.try_clone()?, uid, check)?
                != self.bundle_tree
        {
            return Err(Error::ArtifactMismatch);
        }
        let outer = self.root.metadata()?;
        let bundle = self.bundle.metadata()?;
        let (parent, leaf) = filesystem::parent(&self.destination, uid, false)?;
        let named_outer = filesystem::open_child(&parent, &leaf, true)?;
        let named_outer_meta = named_outer.metadata()?;
        if named_outer_meta.dev() != outer.dev() || named_outer_meta.ino() != outer.ino() {
            return Err(Error::ArtifactMismatch);
        }
        let bundle_name =
            CString::new(self.required_root.as_bytes()).map_err(|_| Error::UnsafePath)?;
        let named_bundle = filesystem::open_child(&named_outer, &bundle_name, true)?;
        let named_bundle_meta = named_bundle.metadata()?;
        if named_bundle_meta.dev() != bundle.dev() || named_bundle_meta.ino() != bundle.ino() {
            return Err(Error::ArtifactMismatch);
        }
        check()
    }
}

struct Source<'a> {
    file: File,
    before: Metadata,
    read_bytes: Cell<u64>,
    limits: &'a ArchiveLimits,
    cancelled: &'a AtomicBool,
}
impl Source<'_> {
    fn check(&self) -> Result<()> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err(Error::Invalid("archive cancelled"));
        }
        Ok(())
    }
    fn unchanged(&self) -> Result<()> {
        let now = self.file.metadata()?;
        if !same_file(&self.before, &now) {
            return Err(Error::ArtifactMismatch);
        }
        Ok(())
    }
    fn digest(&self, expected: &ArchiveDigest) -> Result<()> {
        let mut sha = Sha256::new();
        let mut buf = [0u8; 65536];
        let mut offset = 0;
        while offset < expected.size {
            let length = (expected.size - offset).min(buf.len() as u64) as usize;
            let n = self.read_at(&mut buf[..length], offset)?;
            if n == 0 {
                return Err(Error::ArtifactMismatch);
            }
            sha.update(&buf[..n]);
            offset += n as u64;
        }
        let actual: String = sha.finalize().iter().map(|b| format!("{b:02x}")).collect();
        if actual != expected.sha256 {
            return Err(Error::ArtifactMismatch);
        }
        self.unchanged()
    }
}
impl ReaderAt for Source<'_> {
    fn read_at(&self, buf: &mut [u8], offset: u64) -> io::Result<usize> {
        self.check().map_err(io::Error::other)?;
        let count = self
            .read_bytes
            .get()
            .checked_add(buf.len() as u64)
            .filter(|n| *n <= self.limits.max_source_read_bytes)
            .ok_or_else(|| io::Error::other("archive source read budget"))?;
        self.read_bytes.set(count);
        let remaining = self
            .before
            .len()
            .saturating_sub(offset)
            .min(buf.len() as u64) as usize;
        self.file.read_at(&mut buf[..remaining], offset)
    }
}
fn same_file(a: &Metadata, b: &Metadata) -> bool {
    a.dev() == b.dev()
        && a.ino() == b.ino()
        && a.len() == b.len()
        && a.mtime() == b.mtime()
        && a.mtime_nsec() == b.mtime_nsec()
        && a.ctime() == b.ctime()
        && a.ctime_nsec() == b.ctime_nsec()
        && a.mode() == b.mode()
        && a.uid() == b.uid()
        && a.nlink() == b.nlink()
}
fn zip_error(_: rawzip::Error) -> Error {
    Error::Invalid("ZIP structure or CRC")
}
fn invalid(why: &'static str) -> Error {
    Error::Invalid(why)
}

struct Entry {
    path: String,
    raw_path: Vec<u8>,
    kind: ArchiveKind,
    mode: u32,
    size: u64,
    compressed: u64,
    method: CompressionMethod,
    flags: u16,
    crc: u32,
    offset: u64,
    wayfinder: ZipArchiveEntryWayfinder,
    link: Option<PathBuf>,
}

/// 完整审查后才创建专用目标目录。目标必须不存在；失败保留不可用的部分目录，
/// 不删除未知文件、不返回 proof。源 fd 从头到尾不按路径重开。
#[allow(clippy::too_many_arguments)] // 显式区分源、受信根名和执行预算。
pub fn extract_zip(
    source: File,
    expected: &ArchiveDigest,
    required_root: &str,
    destination: &Path,
    uid: u32,
    limits: &ArchiveLimits,
    cancelled: &AtomicBool,
) -> Result<ExtractedArchive> {
    if !filesystem::valid_sha(&expected.sha256)
        || expected.size < 22
        || expected.size > limits.max_source_bytes
        || expected.size >= u32::MAX as u64
        || limits.max_entries == 0
        || limits.max_entries > 300_000
        || limits.max_depth == 0
        || limits.max_depth > 128
        || limits.max_unpacked_bytes > filesystem::MAX_TREE_BYTES
        || limits.max_compression_ratio == 0
    {
        return Err(invalid("archive limits or expected digest"));
    }
    let mut root_spelling = BTreeMap::new();
    validate_name(required_root, limits, &mut root_spelling, &mut 0)?;
    if required_root.contains('/') {
        return Err(invalid("ZIP expected root must be one component"));
    }
    let before = source.metadata()?;
    if !before.is_file()
        || before.uid() != uid
        || before.nlink() != 1
        || before.mode() & 0o7022 != 0
        || before.len() != expected.size
    {
        return Err(Error::UnsafePath);
    }
    let source = Source {
        file: source,
        before,
        read_bytes: Cell::new(0),
        limits,
        cancelled,
    };
    source.check()?;
    source.digest(expected)?;
    let mut buf = vec![0u8; 128 * 1024];
    let archive = ZipLocator::new()
        .locate_in_reader(&source, &mut buf, expected.size)
        .map_err(|(_, e)| zip_error(e))?;
    if archive.directory_offset() > archive.eocd_offset()
        || archive.end_offset() != expected.size
        || archive.comment().remaining() != 0
        || archive.entries_hint() == 0
        || archive.entries_hint() > limits.max_entries as u64
    {
        return Err(invalid("ZIP ending, comment or entry count"));
    }
    // rawzip 不暴露这些字段；仅对其已定位的固定记录补充 profile 政策检查。
    // 不自行定位/解析可变 ZIP 结构或处理压缩。
    let mut end_record = [0u8; 22];
    source.read_exact_at(&mut end_record, archive.eocd_offset())?;
    if end_record[4..8] != [0; 4]
        || end_record[8..10] != end_record[10..12]
        || u32::from_le_bytes(end_record[12..16].try_into().map_err(|_| invalid("EOCD"))?) as u64
            != archive.eocd_offset() - archive.directory_offset()
        || u32::from_le_bytes(end_record[16..20].try_into().map_err(|_| invalid("EOCD"))?) as u64
            != archive.directory_offset()
    {
        return Err(invalid("ZIP multi-volume or inconsistent directory"));
    }
    let entries = inspect(&archive, limits, required_root)?;
    source.unchanged()?;
    source.check()?;
    let (parent, leaf) = filesystem::parent(destination, uid, false)?;
    // SAFETY: 目标名与父 fd 已通过路径检查。EEXIST 永不转为覆盖。
    if unsafe { libc::mkdirat(parent.as_raw_fd(), leaf.as_ptr(), 0o700) } != 0 {
        return Err(io::Error::last_os_error().into());
    }
    parent.sync_all()?;
    let root = filesystem::open_child(&parent, &leaf, true)?;
    root.set_permissions(Permissions::from_mode(0o700))?;
    reject_inherited_acl(&root)?;
    let root_identity = root.metadata()?;
    let directories = directories(&entries);
    for path in directories.keys() {
        source.check()?;
        let (dir, name) = relative_parent(&root, path)?;
        if unsafe { libc::mkdirat(dir.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
            return Err(io::Error::last_os_error().into());
        }
        dir.sync_all()?;
    }
    for entry in &entries {
        source.check()?;
        if !matches!(entry.kind, ArchiveKind::File) {
            continue;
        }
        let (dir, name) = relative_parent(&root, &entry.path)?;
        // SAFETY: 每个父目录均相对 held root fd、NOFOLLOW 打开，文件独占创建。
        let fd = unsafe {
            libc::openat(
                dir.as_raw_fd(),
                name.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error().into());
        }
        let mut file = unsafe { File::from_raw_fd(fd) };
        read_entry(&archive, entry, &mut file)?;
        file.set_permissions(Permissions::from_mode(entry.mode & 0o777))?;
        file.sync_all()?;
        dir.sync_all()?;
    }
    // 链接最后创建，且目录表已经验证每条链最终指向根内现存节点。
    for entry in &entries {
        if let Some(target) = &entry.link {
            source.check()?;
            let (dir, name) = relative_parent(&root, &entry.path)?;
            let text = CString::new(target.to_str().ok_or(Error::UnsafePath)?)
                .map_err(|_| Error::UnsafePath)?;
            if unsafe { libc::symlinkat(text.as_ptr(), dir.as_raw_fd(), name.as_ptr()) } != 0 {
                return Err(io::Error::last_os_error().into());
            }
            dir.sync_all()?;
        }
    }
    for (path, mode) in directories.iter().rev() {
        source.check()?;
        let (dir, name) = relative_parent(&root, path)?;
        let child = filesystem::open_child(&dir, &name, true)?;
        child.set_permissions(Permissions::from_mode(*mode))?;
        child.sync_all()?;
    }
    root.sync_all()?;
    parent.sync_all()?;
    source.digest(expected)?;
    let outer_tree = filesystem::fingerprint_file(root.try_clone()?, uid, &|| source.check())?;
    let bundle_name = CString::new(required_root.as_bytes()).map_err(|_| Error::UnsafePath)?;
    let bundle = filesystem::open_child(&root, &bundle_name, true)?;
    let bundle_identity = bundle.metadata()?;
    let bundle_tree = filesystem::fingerprint_file(bundle.try_clone()?, uid, &|| source.check())?;
    source.unchanged()?;
    // proof 的公开路径也必须仍解析到 held root，不能只复查旧父 fd 下的名称。
    let (current_parent, current_leaf) = filesystem::parent(destination, uid, false)?;
    let named_root = filesystem::open_child(&current_parent, &current_leaf, true)?.metadata()?;
    if named_root.dev() != root_identity.dev() || named_root.ino() != root_identity.ino() {
        return Err(Error::ArtifactMismatch);
    }
    let named_bundle = filesystem::open_child(
        &filesystem::open_child(&current_parent, &current_leaf, true)?,
        &bundle_name,
        true,
    )?
    .metadata()?;
    if named_bundle.dev() != bundle_identity.dev() || named_bundle.ino() != bundle_identity.ino() {
        return Err(Error::ArtifactMismatch);
    }
    source.check()?;
    Ok(ExtractedArchive {
        destination: destination.to_owned(),
        required_root: required_root.to_owned(),
        source: expected.clone(),
        outer_tree,
        bundle_tree,
        root,
        bundle,
        source_read_bytes: source.read_bytes.get(),
    })
}

fn inspect(
    archive: &ZipArchive<&Source<'_>>,
    limits: &ArchiveLimits,
    required_root: &str,
) -> Result<Vec<Entry>> {
    let mut buffer = vec![0u8; 128 * 1024];
    let mut iterator = archive.entries(&mut buffer);
    let mut entries = Vec::new();
    let mut spellings = BTreeMap::new();
    let mut path_bytes = 0;
    let mut seen = BTreeSet::new();
    let mut total = 0u64;
    let mut cd_offset = archive.directory_offset();
    while let Some(header) = iterator.next_entry().map_err(zip_error)? {
        archive.get_ref().check()?;
        if entries.len() >= limits.max_entries {
            return Err(invalid("archive entry budget"));
        }
        let mut policy = [0u8; 46];
        archive
            .get_ref()
            .read_exact_at(&mut policy, header.central_directory_offset())?;
        if policy[34..36] != [0; 2] || u16::from_le_bytes([policy[6], policy[7]]) > 20 {
            return Err(invalid("ZIP multi-volume or unsupported version"));
        }
        let raw_path = header.file_path().as_ref().to_vec();
        let text = std::str::from_utf8(&raw_path).map_err(|_| Error::UnsafePath)?;
        let mode = header.external_attributes() >> 16;
        if !matches!(header.version_made_by().creator_system().as_u8(), 3 | 19) {
            return Err(invalid("ZIP requires Unix mode"));
        }
        let kind = match mode & 0o170000 {
            0o100000 => ArchiveKind::File,
            0o040000 => ArchiveKind::Directory,
            0o120000 => ArchiveKind::Symlink,
            _ => return Err(invalid("ZIP special or unspecified file type")),
        };
        let is_dir = matches!(kind, ArchiveKind::Directory);
        if text.ends_with('/') != is_dir {
            return Err(invalid("ZIP name/type mismatch"));
        }
        let path = if is_dir {
            &text[..text.len() - 1]
        } else {
            text
        };
        validate_name(path, limits, &mut spellings, &mut path_bytes)?;
        if path.split('/').next() != Some(required_root) || (path == required_root && !is_dir) {
            return Err(invalid("ZIP does not match required component root"));
        }
        if !seen.insert(path.to_string()) {
            return Err(invalid("ZIP duplicate entry"));
        }
        if mode & 0o7000 != 0
            || (!matches!(kind, ArchiveKind::Symlink)
                && (mode & 0o022 != 0 || mode & 0o400 == 0 || (is_dir && mode & 0o100 == 0)))
        {
            return Err(invalid("ZIP unsafe permissions"));
        }
        let flags = header.flags().bits();
        if flags & !(0x0800 | 0x0008 | 0x0006) != 0 {
            return Err(invalid("ZIP encrypted or unsupported flags"));
        }
        let method = header.compression_method();
        if method != CompressionMethod::STORE && method != CompressionMethod::DEFLATE {
            return Err(invalid("ZIP unsupported compression"));
        }
        if method == CompressionMethod::STORE && flags & 6 != 0 {
            return Err(invalid("ZIP flags/method"));
        }
        let size = header.uncompressed_size_hint();
        let compressed = header.compressed_size_hint();
        if size > limits.max_file_bytes
            || size >= u32::MAX as u64
            || compressed >= u32::MAX as u64
            || (is_dir && (size != 0 || compressed != 0 || flags & 8 != 0))
            || (matches!(kind, ArchiveKind::Symlink) && (size == 0 || size >= 4096))
            || (method == CompressionMethod::STORE && size != compressed)
            || (size > compressed.saturating_mul(limits.max_compression_ratio))
        {
            return Err(invalid("ZIP file size or compression ratio"));
        }
        total = total
            .checked_add(size)
            .filter(|n| *n <= limits.max_unpacked_bytes)
            .ok_or(invalid("ZIP unpacked budget"))?;
        check_extras(header.extra_fields(), false)?;
        if !header.comment().as_bytes().is_empty() {
            return Err(invalid("ZIP entry comments unsupported"));
        }
        if header.central_directory_offset() != cd_offset {
            return Err(invalid("ZIP central directory gap"));
        }
        cd_offset +=
            46 + raw_path.len() as u64 + header.extra_fields().remaining_bytes().len() as u64;
        entries.push(Entry {
            path: path.to_string(),
            raw_path,
            kind,
            mode,
            size,
            compressed,
            method,
            flags,
            crc: header.crc32(),
            offset: header.local_header_offset(),
            wayfinder: header.wayfinder(),
            link: None,
        });
    }
    if entries.len() as u64 != archive.entries_hint() || cd_offset != archive.eocd_offset() {
        return Err(invalid("ZIP directory count, ZIP64 or trailing metadata"));
    }
    let mut ranges = Vec::new();
    for entry in &mut entries {
        let item = archive.get_entry(entry.wayfinder).map_err(zip_error)?;
        let mut local_buffer = vec![0u8; 128 * 1024];
        let local = item.local_header(&mut local_buffer).map_err(zip_error)?;
        let mut version = [0u8; 2];
        archive
            .get_ref()
            .read_exact_at(&mut version, entry.offset + 4)?;
        if u16::from_le_bytes(version) > 20 {
            return Err(invalid("ZIP local unsupported version"));
        }
        if local.file_path().as_ref() != entry.raw_path
            || local.flags().bits() != entry.flags
            || local.compression_method() != entry.method
        {
            return Err(invalid("ZIP local/central identity mismatch"));
        }
        check_extras(local.extra_fields(), true)?;
        let (data_start, mut end) = item.compressed_data_range();
        if data_start <= entry.offset || end > archive.directory_offset() {
            return Err(invalid("ZIP entry range"));
        }
        if entry.flags & 8 == 0 {
            if local.crc32() != entry.crc
                || local.compressed_size_hint() != entry.compressed
                || local.uncompressed_size_hint() != entry.size
            {
                return Err(invalid("ZIP local/central sizes or CRC"));
            }
        } else {
            if (local.crc32() != 0 && local.crc32() != entry.crc)
                || (local.compressed_size_hint() != 0
                    && local.compressed_size_hint() != entry.compressed)
                || (local.uncompressed_size_hint() != 0
                    && local.uncompressed_size_hint() != entry.size)
            {
                return Err(invalid("ZIP local descriptor placeholders"));
            }
            let descriptor = item
                .reader()
                .data_descriptor()
                .map_err(zip_error)?
                .ok_or(invalid("ZIP missing descriptor"))?;
            if descriptor.crc32() != entry.crc
                || descriptor.compressed_size() != entry.compressed
                || descriptor.uncompressed_size() != entry.size
            {
                return Err(invalid("ZIP descriptor mismatch"));
            }
            // 这里只识别标准 descriptor 的可选签名；结构/数值由 rawzip 解析。
            let mut signature = [0u8; 4];
            archive.get_ref().read_exact_at(&mut signature, end)?;
            end += if signature == [0x50, 0x4b, 0x07, 0x08] {
                16
            } else {
                12
            };
        }
        ranges.push((entry.offset, end));
        if matches!(entry.kind, ArchiveKind::Symlink) {
            let mut text = Vec::with_capacity(entry.size as usize);
            read_entry(archive, entry, &mut text)?;
            let target = String::from_utf8(text).map_err(|_| Error::UnsafePath)?;
            entry.link = Some(PathBuf::from(target));
        } else if matches!(entry.kind, ArchiveKind::Directory) {
            read_entry(archive, entry, &mut io::sink())?;
        }
    }
    ranges.sort_unstable();
    let mut next = 0;
    for (start, end) in ranges {
        if start != next || end < start || end > archive.directory_offset() {
            return Err(invalid("ZIP overlapping, prefixed or hidden entries"));
        }
        next = end;
    }
    if next != archive.directory_offset() {
        return Err(invalid("ZIP local directory gap"));
    }
    if !entries
        .iter()
        .any(|entry| entry.path == required_root && matches!(entry.kind, ArchiveKind::Directory))
    {
        return Err(invalid("ZIP explicit component directory missing"));
    }
    let root_prefix = format!("{required_root}/");
    let table: Vec<_> = entries
        .iter()
        .filter(|e| e.path != required_root)
        .map(|e| ArchiveEntry {
            path: PathBuf::from(e.path.strip_prefix(&root_prefix).unwrap_or(&e.path)),
            kind: e.kind.clone(),
            unpacked_bytes: if matches!(e.kind, ArchiveKind::File) {
                e.size
            } else {
                0
            },
            link_target: e.link.clone(),
        })
        .collect();
    filesystem::validate_archive_entries(&table, limits.max_unpacked_bytes)?;
    if directories(&entries).len().saturating_add(
        entries
            .iter()
            .filter(|e| !matches!(e.kind, ArchiveKind::Directory))
            .count(),
    ) > limits.max_entries
    {
        return Err(invalid("ZIP implicit directory budget"));
    }
    Ok(entries)
}

fn check_extras(mut extras: ExtraFields<'_>, local: bool) -> Result<()> {
    let mut seen = BTreeSet::new();
    for (id, data) in extras.by_ref() {
        if !seen.insert(id.as_u16()) {
            return Err(invalid("ZIP duplicate extra field"));
        }
        let valid = match id.as_u16() {
            // 时间与 owner 信息不恢复；只允许格式完整的这三种说明性元数据。
            0x5855 => data.len() == if local { 12 } else { 8 },
            0x5455 => {
                !data.is_empty()
                    && data[0] & !7 == 0
                    && data.len()
                        == if local {
                            1 + 4 * data[0].count_ones() as usize
                        } else {
                            1 + 4 * usize::from(data[0] & 1 != 0)
                        }
            }
            0x7875 => {
                data.len() >= 5
                    && data[0] == 1
                    && matches!(data[1], 1 | 2 | 4 | 8)
                    && (2 + data[1] as usize) < data.len()
                    && matches!(data[2 + data[1] as usize], 1 | 2 | 4 | 8)
                    && data.len() == 3 + data[1] as usize + data[2 + data[1] as usize] as usize
            }
            _ => false,
        };
        if !valid {
            return Err(invalid("ZIP unsupported or malformed metadata"));
        }
    }
    if !extras.remaining_bytes().is_empty() {
        return Err(invalid("ZIP truncated extra field"));
    }
    Ok(())
}

fn validate_name(
    path: &str,
    limits: &ArchiveLimits,
    spellings: &mut BTreeMap<String, String>,
    path_bytes: &mut usize,
) -> Result<()> {
    if path.is_empty()
        || path.len() >= 4096
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains(':')
        || path.chars().any(char::is_control)
    {
        return Err(Error::UnsafePath);
    }
    let mut prefix = String::new();
    let parts: Vec<_> = path.split('/').collect();
    if parts.len() > limits.max_depth {
        return Err(invalid("ZIP depth budget"));
    }
    for part in parts {
        if part.is_empty() || part == "." || part == ".." || part.len() > 255
            || part.starts_with("._") || part == "__MACOSX"
            || part.chars().any(|c| matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}')) {
            return Err(Error::UnsafePath);
        }
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(part);
        // 保留原文件名；比较键取更保守的兼容分解+完整折叠，另补当前 Rust Unicode 小写。
        let key: String = prefix
            .nfkd()
            .case_fold()
            .flat_map(char::to_lowercase)
            .nfkd()
            .collect();
        if let Some(prior) = spellings.get(&key) {
            if prior != &prefix {
                return Err(invalid("ZIP case or Unicode alias"));
            }
        } else {
            if spellings.len() >= limits.max_entries {
                return Err(invalid("ZIP implicit directory budget"));
            }
            *path_bytes = path_bytes
                .checked_add(key.len())
                .and_then(|n| n.checked_add(prefix.len()))
                .filter(|n| *n <= limits.max_path_bytes)
                .ok_or(invalid("ZIP path memory budget"))?;
            spellings.insert(key, prefix.clone());
        }
    }
    Ok(())
}

fn read_entry(
    archive: &ZipArchive<&Source<'_>>,
    entry: &Entry,
    output: &mut dyn Write,
) -> Result<()> {
    let item = archive.get_entry(entry.wayfinder).map_err(zip_error)?;
    if entry.method == CompressionMethod::STORE {
        let mut verified = item.verifying_reader(item.reader());
        copy_bounded(&mut verified, output, entry.size, archive.get_ref())?;
    } else {
        let decoder = StrictDeflate::new(item.reader(), entry.compressed);
        let mut verified = item.verifying_reader(decoder);
        copy_bounded(&mut verified, output, entry.size, archive.get_ref())?;
        if verified.into_inner().decoder.total_in() != entry.compressed {
            return Err(invalid("ZIP trailing compressed data"));
        }
    }
    Ok(())
}
// flate2 的高层 Read 可以在底层 EOF + BufError 时返回 0；不能把这种截断当作 StreamEnd。
// 此薄适配只调用成熟解压器，并要求真实 StreamEnd 与压缩长度完全相等。
struct StrictDeflate<R> {
    reader: R,
    decoder: Decompress,
    input: [u8; 65536],
    start: usize,
    end: usize,
    finished: bool,
    compressed: u64,
}
impl<R: Read> StrictDeflate<R> {
    fn new(reader: R, compressed: u64) -> Self {
        Self {
            reader,
            decoder: Decompress::new(false),
            input: [0; 65536],
            start: 0,
            end: 0,
            finished: false,
            compressed,
        }
    }
}
impl<R: Read> Read for StrictDeflate<R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() || self.finished {
            return Ok(0);
        }
        loop {
            if self.start == self.end {
                self.end = self.reader.read(&mut self.input)?;
                self.start = 0;
            }
            let eof = self.start == self.end;
            let before_in = self.decoder.total_in();
            let before_out = self.decoder.total_out();
            let status = self
                .decoder
                .decompress(
                    &self.input[self.start..self.end],
                    output,
                    if eof {
                        FlushDecompress::Finish
                    } else {
                        FlushDecompress::None
                    },
                )
                .map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "invalid Deflate stream")
                })?;
            let consumed = (self.decoder.total_in() - before_in) as usize;
            let produced = (self.decoder.total_out() - before_out) as usize;
            self.start += consumed;
            if status == Status::StreamEnd {
                if self.decoder.total_in() != self.compressed {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "trailing Deflate data",
                    ));
                }
                self.finished = true;
                return Ok(produced);
            }
            if produced > 0 {
                return Ok(produced);
            }
            if eof {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "missing Deflate StreamEnd",
                ));
            }
            if consumed == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "stalled Deflate stream",
                ));
            }
        }
    }
}
fn copy_bounded(
    input: &mut dyn Read,
    output: &mut dyn Write,
    size: u64,
    source: &Source<'_>,
) -> Result<()> {
    let mut count = 0;
    let mut buffer = [0u8; 65536];
    loop {
        source.check()?;
        let allowed =
            (size.saturating_sub(count).saturating_add(1)).min(buffer.len() as u64) as usize;
        let n = input.read(&mut buffer[..allowed])?;
        if n == 0 {
            break;
        }
        count += n as u64;
        if count > size {
            return Err(invalid("ZIP actual size exceeds declaration"));
        }
        output.write_all(&buffer[..n])?;
    }
    if count != size {
        return Err(invalid("ZIP actual size below declaration"));
    }
    Ok(())
}
fn directories(entries: &[Entry]) -> BTreeMap<String, u32> {
    let mut dirs = BTreeMap::new();
    for entry in entries {
        let mut parts: Vec<_> = entry.path.split('/').collect();
        if !matches!(entry.kind, ArchiveKind::Directory) {
            parts.pop();
        }
        while !parts.is_empty() {
            dirs.entry(parts.join("/")).or_insert(0o755);
            parts.pop();
        }
    }
    for entry in entries {
        if matches!(entry.kind, ArchiveKind::Directory) {
            dirs.insert(entry.path.clone(), entry.mode & 0o777);
        }
    }
    dirs
}
// macOS 扩展 ACL 可越过 0700 mode；新根若继承 ACL，拒绝而不是静默清掉策略。
#[cfg(target_os = "macos")]
fn reject_inherited_acl(root: &File) -> Result<()> {
    use std::ffi::c_void;
    unsafe extern "C" {
        fn acl_get_fd_np(fd: i32, kind: u32) -> *mut c_void;
        fn acl_get_entry(acl: *mut c_void, index: i32, entry: *mut *mut c_void) -> i32;
        fn acl_free(acl: *mut c_void) -> i32;
    }
    let acl = unsafe { acl_get_fd_np(root.as_raw_fd(), 0x100) }; // SDK ACL_TYPE_EXTENDED
    if acl.is_null() {
        let error = io::Error::last_os_error();
        // Apple Libc acl_file.c：fd 有效但 FILESEC_ACL 属性不存在时也返回 ENOENT。
        // 必须同时确认固定目录仍存在；其他读取错误不能被当作无 ACL。
        if error.raw_os_error() == Some(libc::ENOENT) && root.metadata()?.nlink() > 0 {
            return Ok(());
        }
        return Err(error.into());
    }
    let mut entry = std::ptr::null_mut();
    let result = unsafe { acl_get_entry(acl, 0, &mut entry) }; // SDK ACL_FIRST_ENTRY
    let error = io::Error::last_os_error();
    unsafe {
        acl_free(acl);
    }
    if result == 0 {
        return Err(invalid("ZIP private root inherited ACL"));
    }
    if error.raw_os_error() != Some(libc::EINVAL) {
        return Err(error.into());
    }
    Ok(())
}
#[cfg(not(target_os = "macos"))]
fn reject_inherited_acl(_: &File) -> Result<()> {
    Ok(())
}

fn relative_parent(root: &File, path: &str) -> Result<(File, CString)> {
    let mut parts: Vec<_> = path.split('/').collect();
    let leaf = parts.pop().ok_or(Error::UnsafePath)?;
    let mut directory = root.try_clone()?;
    for part in parts {
        let name = CString::new(part).map_err(|_| Error::UnsafePath)?;
        directory = filesystem::open_child(&directory, &name, true)?;
    }
    Ok((
        directory,
        CString::new(leaf).map_err(|_| Error::UnsafePath)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn held_source_detects_mutation_and_mid_read_cancellation() {
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(b"source bytes").unwrap();
        let alias = file.try_clone().unwrap();
        let cancelled = AtomicBool::new(false);
        let limits = ArchiveLimits::default();
        let before = file.metadata().unwrap();
        let source = Source {
            file,
            before,
            read_bytes: Cell::new(0),
            limits: &limits,
            cancelled: &cancelled,
        };
        let mut bytes = [0; 6];
        source.read_at(&mut bytes, 0).unwrap();
        assert_eq!(&bytes, b"source");
        cancelled.store(true, Ordering::Release);
        assert!(source.read_at(&mut bytes, 6).is_err());
        cancelled.store(false, Ordering::Release);
        alias.write_at(b"changed size!", 0).unwrap();
        alias.sync_all().unwrap();
        assert!(matches!(source.unchanged(), Err(Error::ArtifactMismatch)));
    }

    #[test]
    fn tree_fingerprint_checks_cancellation_inside_file_chunks() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("file"), vec![0u8; 200_000]).unwrap();
        let calls = Cell::new(0);
        let result = filesystem::fingerprint_file(
            File::open(temp.path()).unwrap(),
            unsafe { libc::geteuid() },
            &|| {
                calls.set(calls.get() + 1);
                if calls.get() >= 4 {
                    Err(invalid("archive cancelled"))
                } else {
                    Ok(())
                }
            },
        );
        assert!(result.is_err());
        assert_eq!(calls.get(), 4);
    }
}
