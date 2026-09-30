//! 恢复先固定合法中断点，再按单向 cursor 重入；未知原件或隔离物均保留。
use super::*;
fn source(guard: &StartupTransaction, member: &Member) -> Result<PathBuf> {
    let root = guard
        .roots
        .iter()
        .find(|r| r.label == member.root_label)
        .context("restore root missing")?;
    checked_path(&root.root, Path::new(&member.name))
}
fn quarantine(guard: &StartupTransaction, member: &Member) -> Result<PathBuf> {
    let root = guard
        .roots
        .iter()
        .find(|r| r.label == member.root_label)
        .context("quarantine root missing")?;
    checked_path(
        &root.root,
        &PathBuf::from("migration_restore_quarantine")
            .join(format!("startup-{}", guard.journal.attempt_id))
            .join(&member.name),
    )
}
fn matches(path: &Path, expected: &Option<FileStamp>) -> Result<bool> {
    Ok(digest(&observe(path)?) == *expected)
}
/// cursor 之前必须已还原，cursor 允许一个操作的 before/after，其后必须仍是选定前缀。
fn validate_disk(
    guard: &StartupTransaction,
    initial: &[Option<FileStamp>],
    cursor: usize,
) -> Result<()> {
    for (i, member) in guard.journal.members.iter().enumerate() {
        let src = source(guard, member)?;
        let q = quarantine(guard, member)?;
        let now = digest(&observe(&src)?);
        let preserved = digest(&observe(&q)?);
        let baseline = member.digest();
        let before = now == initial[i] && preserved.is_none();
        let after = if baseline.is_none() {
            now.is_none() && preserved == initial[i]
        } else {
            now == baseline && preserved.is_none()
        };
        anyhow::ensure!(
            if i < cursor {
                after
            } else if i == cursor {
                before || after
            } else {
                before
            },
            "restore source/quarantine is not the fixed interruption; preserve for repair"
        );
    }
    Ok(())
}
pub(super) fn recover(guard: &mut StartupTransaction) -> Result<()> {
    guard.validate()?;
    verify_backup(&guard.outcome)?;
    let states = prefixes(&guard.journal)?;
    if guard.journal.phase == Phase::Mutating {
        let current = observed_digests(&guard.roots, &guard.journal.members)?;
        let prefix = states
            .iter()
            .rposition(|state| state == &current)
            .context("settings are not one authorized operation prefix; preserve for repair")?;
        for member in &guard.journal.members {
            anyhow::ensure!(
                observe(&quarantine(guard, member)?)? == Original::Absent,
                "unexpected prior quarantine"
            );
        }
        guard.journal.phase = Phase::Restoring;
        guard.journal.recovery = Some(Recovery { prefix, cursor: 0 });
        guard.persist()?;
        fault("v3_restoring")?;
    }
    anyhow::ensure!(
        guard.journal.phase == Phase::Restoring,
        "recovery was not armed"
    );
    let recovery = guard
        .journal
        .recovery
        .clone()
        .context("recovery cursor missing")?;
    let initial = &states[recovery.prefix];
    validate_disk(guard, initial, recovery.cursor)?;
    for i in recovery.cursor..guard.journal.members.len() {
        // 每次可能的写入前审整组，不能先覆盖一部分再发现另一个未知文件。
        validate_disk(guard, initial, i)?;
        let member = &guard.journal.members[i];
        let src = source(guard, member)?;
        let q = quarantine(guard, member)?;
        let baseline = member.digest();
        match baseline {
            None => {
                if let Some(expected) = &initial[i] {
                    let parent = src.parent().context("settings parent missing")?;
                    let qparent = q.parent().context("quarantine parent missing")?;
                    create_dirs_durable(qparent)?;
                    if observe(&src)? != Original::Absent {
                        anyhow::ensure!(
                            matches(&src, &Some(expected.clone()))?
                                && observe(&q)? == Original::Absent,
                            "settings changed before quarantine"
                        );
                        fs::rename(&src, &q)?;
                        fault("v3_quarantine_renamed")?;
                    }
                    anyhow::ensure!(
                        matches(&q, &Some(expected.clone()))? && observe(&src)? == Original::Absent,
                        "quarantine ownership changed"
                    );
                    open_read(&q)?.sync_all()?;
                    fault("v3_quarantine_file_synced")?;
                    sync_dir(parent)?;
                    fault("v3_quarantine_source_synced")?;
                    sync_dir(qparent)?;
                    fault("v3_quarantine_target_synced")?;
                    sync_dir(qparent.parent().context("quarantine ancestor missing")?)?;
                    fault("v3_quarantine_ancestor_synced")?;
                }
            }
            Some(expected) => {
                if !matches(&src, &Some(expected.clone()))? {
                    anyhow::ensure!(
                        matches(&src, &initial[i])?,
                        "settings changed before baseline restore"
                    );
                    let entry = guard
                        .outcome
                        .manifest
                        .entries
                        .iter()
                        .find(|e| {
                            e.source_root_label == member.root_label
                                && e.source_relative_path == Path::new(&member.name)
                        })
                        .context("baseline payload missing")?;
                    let payload =
                        checked_path(&guard.outcome.backup_dir, &entry.backup_relative_path)?;
                    let parent = src.parent().context("restore parent missing")?;
                    create_dirs_durable(parent)?;
                    #[cfg(test)]
                    super::tests::inject_before_present_restore();
                    restore_present(&payload, &src, &initial[i], &expected)?;
                    fault("v3_present_restored")?;
                }
                // 重入时即使已是原摘要也须重新同步，不能把上次同步失败当成功。
                open_read(&src)?.sync_all()?;
                sync_dir(src.parent().context("restore parent missing")?)?;
                fault("v3_present_synced")?;
            }
        }
        guard
            .journal
            .recovery
            .as_mut()
            .context("recovery cursor lost")?
            .cursor = i + 1;
        guard.persist()?;
        fault("v3_restore_cursor")?;
    }
    validate_disk(guard, initial, guard.journal.members.len())?;
    if guard.journal.purpose == StartupPurpose::FullStartup {
        // 设置已按 cursor 恢复；完整迁移才可重放其余 DB/录音，两个小 purpose 永不走这里。
        let mut other = guard.outcome.clone();
        other.manifest.entries.retain(|entry| {
            !guard.journal.members.iter().any(|m| {
                m.root_label == entry.source_root_label
                    && Path::new(&m.name) == entry.source_relative_path
            })
        });
        restore_backup_inner(
            &other,
            &guard.roots,
            SqliteRestorePolicy::ConsistentSnapshot,
            false,
        )?;
    }
    fault("v3_restored")?;
    guard.journal.phase = Phase::Recovered;
    guard.persist()?;
    fault("v3_recovered")
}

pub(super) fn restore_present(
    payload: &Path,
    target: &Path,
    expected_current: &Option<FileStamp>,
    expected: &FileStamp,
) -> Result<()> {
    let before = observe(target)?;
    anyhow::ensure!(
        digest(&before) == *expected_current,
        "restore target no longer matches the authorized interruption"
    );
    let parent = target.parent().context("restore target parent missing")?;
    let stage = checked_path(
        parent,
        Path::new(&format!(".migration-{}.restore", uuid::Uuid::new_v4())),
    )?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let mut output = options.open(&stage)?;
    let bytes = read_bytes(payload, SETTINGS_LIMIT)?;
    anyhow::ensure!(
        bytes.len() as u64 == expected.size && sha(&bytes) == expected.sha256,
        "restore payload changed after verification"
    );
    output.write_all(&bytes)?;
    output.set_permissions(open_read(payload)?.metadata()?.permissions())?;
    fault("v3_restore_temp_written")?;
    output.sync_all()?;
    fault("v3_restore_file_synced")?;
    anyhow::ensure!(
        observe(target)? == before,
        "settings changed before atomic restore"
    );
    fs::rename(&stage, target)?;
    fault("v3_restore_renamed")?;
    sync_dir(parent)?;
    fault("v3_restore_directory_synced")
}
