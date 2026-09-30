//! 连接内的对账失效时钟。仅减少未变化时的重复审核，不代替持久源回执或隐私证明。
use rusqlite::Connection;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Stamp {
    instance: Vec<u8>,
    generation: i64,
    external_version: i64,
}

pub(crate) enum Domain {
    Projection,
    Sources,
}

/// TEMP 触发器只属于此连接；外部连接的提交另外通过 SQLite data_version 失效。
pub(crate) fn install(db: &Connection, domain: Domain) -> rusqlite::Result<()> {
    db.execute_batch(
        "CREATE TEMP TABLE inputia_memory_audit_clock(
        singleton INTEGER PRIMARY KEY CHECK(singleton=1),
        instance BLOB NOT NULL CHECK(length(instance)=16),
        generation INTEGER NOT NULL CHECK(typeof(generation)='integer' AND generation>=0),
        expected_triggers INTEGER NOT NULL CHECK(expected_triggers IN(3,6)));",
    )?;
    let tables: &[(&str, Option<&str>)] = match domain {
        Domain::Projection => &[("integration_items", None), ("integration_sources", None)],
        Domain::Sources => &[("memory_sources", Some("store_id"))],
    };
    db.execute(
        "INSERT INTO temp.inputia_memory_audit_clock VALUES(1,randomblob(16),0,?1)",
        [(tables.len() * 3) as u32],
    )?;
    for (table, exclude_commit) in tables {
        for action in ["INSERT", "UPDATE", "DELETE"] {
            let guard = match (exclude_commit, action) {
                (Some(column), "INSERT") => format!("WHEN NEW.{column} NOT LIKE 'commit:%'"),
                (Some(column), "DELETE") => format!("WHEN OLD.{column} NOT LIKE 'commit:%'"),
                (Some(column), _) => format!(
                    "WHEN OLD.{column} NOT LIKE 'commit:%' OR NEW.{column} NOT LIKE 'commit:%'"
                ),
                _ => String::new(),
            };
            // 表名、动作与字段全部来自上面的固定枚举，不接受外部 SQL 标识符。
            db.execute_batch(&format!("CREATE TEMP TRIGGER audit_{table}_{action} AFTER {action} ON main.{table} {guard}
                BEGIN
                    UPDATE inputia_memory_audit_clock SET generation=generation+1 WHERE singleton=1;
                    SELECT CASE WHEN changes()<>1 THEN RAISE(ABORT,'memory audit clock missing') END;
                END;"))?;
        }
    }
    Ok(())
}

pub(crate) fn external_version(db: &Connection) -> rusqlite::Result<i64> {
    db.pragma_query_value(None, "data_version", |row| row.get(0))
}

/// 只有时钟未安装或不完整可以回退完整对账；SQLite 读取错误不能当成“没有缓存”。
pub(crate) fn for_cache(db: &Connection) -> rusqlite::Result<Option<Stamp>> {
    let exists: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM temp.sqlite_schema WHERE type='table' AND name='inputia_memory_audit_clock')", [], |row| row.get(0))?;
    if !exists {
        return Ok(None);
    }
    match read(db) {
        Err(rusqlite::Error::InvalidQuery) => Ok(None),
        result => result.map(Some),
    }
}

fn read(db: &Connection) -> rusqlite::Result<Stamp> {
    let external_version = external_version(db)?;
    let (instance, generation, expected): (Vec<u8>, i64, i64) = db.query_row(
        "SELECT instance,generation,expected_triggers FROM temp.inputia_memory_audit_clock WHERE singleton=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let triggers: i64 = db.query_row("SELECT COUNT(*) FROM temp.sqlite_schema WHERE type='trigger' AND name IN(
        'audit_integration_items_INSERT','audit_integration_items_UPDATE','audit_integration_items_DELETE',
        'audit_integration_sources_INSERT','audit_integration_sources_UPDATE','audit_integration_sources_DELETE',
        'audit_memory_sources_INSERT','audit_memory_sources_UPDATE','audit_memory_sources_DELETE')", [], |row| row.get(0))?;
    if instance.len() != 16 || generation < 0 || triggers != expected || ![3, 6].contains(&expected)
    {
        return Err(rusqlite::Error::InvalidQuery);
    }
    Ok(Stamp {
        instance,
        generation,
        external_version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_trigger_or_exhausted_clock_cannot_reuse_an_audit() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE memory_sources(store_id TEXT,revision INTEGER)")
            .unwrap();
        install(&db, Domain::Sources).unwrap();
        db.execute_batch(
            "UPDATE temp.inputia_memory_audit_clock SET generation=9223372036854775807",
        )
        .unwrap();
        assert!(db
            .execute_batch("INSERT INTO memory_sources VALUES('source',1)")
            .is_err());
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM memory_sources", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
        db.execute_batch("DROP TRIGGER temp.audit_memory_sources_UPDATE")
            .unwrap();
        assert!(read(&db).is_err());
    }

    #[test]
    fn every_source_projection_mutation_changes_stamp_but_receipt_only_work_does_not() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE integration_items(id INTEGER PRIMARY KEY,body TEXT); CREATE TABLE integration_sources(id INTEGER PRIMARY KEY,store_id TEXT); CREATE TABLE receipts(id INTEGER);").unwrap();
        install(&db, Domain::Projection).unwrap();
        for sql in [
            "INSERT INTO integration_items VALUES(1,'a')",
            "UPDATE integration_items SET body='b'",
            "DELETE FROM integration_items",
            "INSERT INTO integration_sources VALUES(1,'a')",
            "UPDATE integration_sources SET store_id='b'",
            "DELETE FROM integration_sources",
        ] {
            let prior = read(&db).unwrap();
            db.execute_batch(sql).unwrap();
            assert_ne!(prior, read(&db).unwrap(), "{sql}");
        }
        let prior = read(&db).unwrap();
        db.execute_batch("INSERT INTO receipts VALUES(1)").unwrap();
        assert_eq!(prior, read(&db).unwrap());
    }

    #[test]
    fn external_commit_invalidates_same_generation_and_a_new_connection_has_a_new_identity() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("projection.db");
        let first = Connection::open(&path).unwrap();
        first.execute_batch("CREATE TABLE integration_items(id INTEGER); CREATE TABLE integration_sources(id INTEGER);").unwrap();
        install(&first, Domain::Projection).unwrap();
        let before = read(&first).unwrap();
        let second = Connection::open(&path).unwrap();
        second
            .execute_batch("INSERT INTO integration_items VALUES(1)")
            .unwrap();
        let after = read(&first).unwrap();
        assert_eq!(before.generation, after.generation);
        assert_ne!(before, after);
        install(&second, Domain::Projection).unwrap();
        assert_ne!(before.instance, read(&second).unwrap().instance);
    }

    #[test]
    fn typed_commits_do_not_invalidate_source_audits_and_rollback_restores_clock() {
        let mut db = Connection::open_in_memory().unwrap();
        db.execute_batch("CREATE TABLE memory_sources(store_id TEXT,revision INTEGER);")
            .unwrap();
        install(&db, Domain::Sources).unwrap();
        let before = read(&db).unwrap();
        db.execute_batch("INSERT INTO memory_sources VALUES('commit:span:1',1); UPDATE memory_sources SET revision=2; DELETE FROM memory_sources;").unwrap();
        assert_eq!(before, read(&db).unwrap());
        let tx = db.transaction().unwrap();
        tx.execute_batch("INSERT INTO memory_sources VALUES('real-source',1)")
            .unwrap();
        assert_ne!(before, read(&tx).unwrap());
        drop(tx);
        assert_eq!(before, read(&db).unwrap());
        db.execute_batch("INSERT INTO memory_sources VALUES('real-source',1)")
            .unwrap();
        assert_ne!(before, read(&db).unwrap());
    }
}
