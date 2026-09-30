use inputia_handy_runtime::{
    privacy_operation::{PrivacyRequest, PrivacyScope, PrivacyState},
    service::HistoryService,
};
use rusqlite::Connection;
use std::{
    path::Path,
    time::{Duration, Instant},
};
/// 所有遗忘测试走真实主服务协调；只创建临时fixture缺失的源表，不启用生产数据。
pub fn forget(root: &Path, term: Option<&str>) {
    for (file,sql) in [
  ("history.db","CREATE TABLE IF NOT EXISTS transcription_history(id INTEGER PRIMARY KEY AUTOINCREMENT,file_name TEXT,timestamp INTEGER,saved INTEGER,title TEXT,transcription_text TEXT,post_processed_text TEXT)"),
  ("clipboard.db","CREATE TABLE IF NOT EXISTS clipboard_history(id INTEGER PRIMARY KEY AUTOINCREMENT,content_type TEXT,full_text TEXT,title TEXT,is_favorite INTEGER,is_pinned INTEGER,created_at INTEGER,image_path TEXT,source_app TEXT)")
 ] {Connection::open(root.join(file)).unwrap().execute_batch(sql).unwrap();}
    let service = HistoryService::start(root.into(), "privacy-tests".into(), |_| {}).unwrap();
    let epoch = service.policy_epoch().unwrap();
    let id = format!("privacy-test-{epoch}");
    service
        .begin_privacy(PrivacyRequest {
            operation_id: id.clone(),
            scope: term
                .map(|term| PrivacyScope::ForgetTerm { term: term.into() })
                .unwrap_or(PrivacyScope::ClearLearned {}),
            expected_epoch: epoch,
        })
        .unwrap();
    let end = Instant::now() + Duration::from_secs(4);
    loop {
        if service.privacy_operation(id.clone()).unwrap().state == PrivacyState::Completed {
            break;
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(25));
    }
}
