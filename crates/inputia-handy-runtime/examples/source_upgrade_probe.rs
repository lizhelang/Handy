//! 仅用于显式提供的 history-copy.db；不查找、不打开日常数据路径。
use inputia_handy_runtime::source::{SourceOutbox, SourceTable, SOURCE_SCHEMA_VERSION};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn fingerprint(conn: &Connection) -> rusqlite::Result<(u64, Vec<u8>)> {
    let mut query = conn.prepare("SELECT id,file_name,timestamp,saved,title,transcription_text,post_processed_text,post_process_prompt,post_process_requested FROM transcription_history ORDER BY id")?;
    let mut rows = query.query([])?;
    let mut hash = Sha256::new();
    let mut count = 0;
    while let Some(row) = rows.next()? {
        for column in 0..9 {
            // Value的Debug仅进入内存摘要，不打印正文；类型和字段边界也参与摘要。
            let value = format!("{:?}", row.get_ref(column)?);
            hash.update((value.len() as u64).to_be_bytes());
            hash.update(value.as_bytes());
        }
        count += 1;
    }
    Ok((count, hash.finalize().to_vec()))
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("explicit history-copy.db required")?,
    );
    if !path.is_absolute()
        || path.file_name().and_then(|s| s.to_str()) != Some("history-copy.db")
        || path.canonicalize()? != path
    {
        return Err("only an absolute canonical history-copy.db is accepted".into());
    }
    let mut conn = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    let before = fingerprint(&conn)?;
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let identity: String =
        conn.query_row("SELECT store_id FROM unified_source_meta", [], |row| {
            row.get(0)
        })?;
    let events: u64 = conn.query_row("SELECT count(*) FROM unified_source_outbox", [], |row| {
        row.get(0)
    })?;
    for _ in 0..2 {
        let source = SourceOutbox::install(&mut conn, SourceTable::History)?;
        assert_eq!(source.store_id(), identity);
        assert_eq!(fingerprint(&conn)?, before);
        assert_eq!(
            conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))?,
            version
        );
        assert_eq!(
            conn.query_row("SELECT count(*) FROM unified_source_outbox", [], |row| row
                .get::<_, u64>(
                0
            ))?,
            events
        );
    }
    let unknown: u64 = conn.query_row("SELECT count(*) FROM transcription_history WHERE inputia_source_trust='unknown' AND inputia_source_app IS NULL", [], |row| row.get(0))?;
    assert_eq!(unknown, before.0);
    println!("source_upgrade_copy=pass schema={SOURCE_SCHEMA_VERSION} rows={} old_fields_unchanged=true source_identity_unchanged=true events_unchanged=true user_version_unchanged=true old_sources_unknown=true repetitions=2", before.0);
    Ok(())
}
