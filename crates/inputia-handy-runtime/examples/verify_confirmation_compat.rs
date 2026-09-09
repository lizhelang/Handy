//! 旧基线兼容代码打开新版本合成库，不进行历史导入或覆盖快照。
use inputia_handy_runtime::store::{HistoryQuery, IntegrationStore};
use rusqlite::Connection;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("explicit integration-copy.db required")?,
    );
    if !path.is_absolute()
        || !path.is_file()
        || path.file_name().and_then(|p| p.to_str()) != Some("integration-copy.db")
    {
        return Err("existing absolute integration-copy.db required".into());
    }
    for _ in 0..2 {
        let mut store = IntegrationStore::open(&path, "compat-fixture")?;
        store.enable_learning(&[93; 32])?;
        assert_eq!(store.policy_epoch()?, 2);
        assert_eq!(store.item_count()?, 2);
        let words = store.list_terms(10, 0)?;
        assert_eq!(words.len(), 1);
        assert_eq!(words[0].term, "AlphaTerm");
        assert_eq!(words[0].contributions, 1);
        let items = store.query(&HistoryQuery::default())?;
        assert_eq!(items.len(), 2);
        assert!(items.iter().all(|item| item.snapshot.starred
            && item.snapshot.pinned
            && item.snapshot.title.as_deref() == Some("fixture title")
            && item.snapshot.asset_ref.as_deref() == Some("fixture.wav")));
        let conn = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        for (table, count) in [
            ("learning_confirmation_receipts", 2),
            ("learning_forget_receipts", 1),
            ("learning_forgotten", 1),
        ] {
            assert_eq!(
                conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| row
                    .get::<_, i64>(0))?,
                count
            );
        }
    }
    println!("compat_reopen=pass repetitions=2 receipts_preserved=true forgotten_not_revived=true metadata_preserved=true");
    Ok(())
}
