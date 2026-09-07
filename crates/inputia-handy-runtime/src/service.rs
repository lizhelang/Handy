//! 单一后台写入线程；Tauri/Host 通过请求队列访问，不在按键线程等待。

use crate::{
    source::SourceTable,
    store::{
        ContentRevision, HistoryQuery, IndexedItem, IntegrationStore, LearnedTermView, TermSnapshot,
    },
    sync::SourcePump,
};
use inputia_core::integration::{
    privacy::{PrivacyContext, PrivacyPolicy},
    terms::HotwordBudget,
};
use rusqlite::{Connection, OpenFlags};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        mpsc::{self, SyncSender},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

type ServiceResult<T> = Result<T, String>;
type Job = Box<dyn FnOnce(&mut ServiceResult<Worker>) + Send>;

/// GUI 发键边界只读取原子快照，不等待数据库或服务队列。
/// 所有新增的策略/删除写入口必须在写入前调用 Worker::revoke_outputs。
#[derive(Clone)]
pub struct OutputPermit {
    generation: Arc<AtomicU64>,
    expected: u64,
    expires: Instant,
}

#[derive(Default)]
struct SourceWrites {
    active: AtomicUsize,
    generation: AtomicU64,
}

/// 源写入期间阻止新许可；Drop 后仍须由同步屏障消费源事务才可重新输出。
pub struct SourceWriteGuard {
    source: Arc<SourceWrites>,
}
impl Drop for SourceWriteGuard {
    fn drop(&mut self) {
        self.source.generation.fetch_add(1, Ordering::AcqRel);
        self.source.active.fetch_sub(1, Ordering::AcqRel);
    }
}

impl OutputPermit {
    pub fn check(&self) -> ServiceResult<()> {
        if self.generation.load(Ordering::Acquire) == self.expected && Instant::now() < self.expires
        {
            Ok(())
        } else {
            Err("output permission expired or was revoked".into())
        }
    }
}

struct Worker {
    output_generation: Arc<AtomicU64>,
    source_writes: Arc<SourceWrites>,
    _writer_lease: std::fs::File,
    learning_key: [u8; 32],
    store: IntegrationStore,
    sources: Vec<SourcePump>,
    last_error: Option<String>,
    generation: u64,
    changed: Box<dyn Fn(u64) + Send>,
}

impl Worker {
    fn revoke_outputs(&self) {
        self.output_generation.fetch_add(1, Ordering::AcqRel);
    }

    fn sync_once(&mut self) -> ServiceResult<bool> {
        let mut changed = false;
        let mut failure = None;
        for source in &mut self.sources {
            let generation = self.output_generation.clone();
            match source.sync_batch_before_apply(&mut self.store, || {
                generation.fetch_add(1, Ordering::AcqRel);
            }) {
                Ok(result) => changed |= result.applied_events > 0 || result.restored_records > 0,
                Err(error) => {
                    failure.get_or_insert_with(|| error.to_string());
                }
            }
        }
        if changed {
            self.generation = self.generation.saturating_add(1);
            (self.changed)(self.generation);
        }
        if failure.is_some() {
            self.revoke_outputs();
        }
        failure.map_or(Ok(changed), Err)
    }
}

pub struct HistoryService {
    output_generation: Arc<AtomicU64>,
    source_writes: Arc<SourceWrites>,
    sender: SyncSender<Job>,
    stopping: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl HistoryService {
    /// root 必须是已完成备份及源表迁移的受管目录；此入口不会创建业务源库。
    pub fn start(
        root: PathBuf,
        profile_id: String,
        changed: impl Fn(u64) + Send + 'static,
    ) -> ServiceResult<Self> {
        let (sender, receiver) = mpsc::sync_channel::<Job>(32);
        let stopping = Arc::new(AtomicBool::new(false));
        let stop = stopping.clone();
        let output_generation = Arc::new(AtomicU64::new(1));
        let worker_generation = output_generation.clone();
        let source_writes = Arc::new(SourceWrites::default());
        let worker_source_writes = source_writes.clone();
        let thread = thread::Builder::new()
            .name("handy-unified-history".into())
            .spawn(move || {
                let mut worker = (|| {
                    let mut options = std::fs::OpenOptions::new();
                    options.read(true).write(true).create(true).truncate(false);
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::OpenOptionsExt;
                        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
                    }
                    let writer = options
                        .open(root.join("integration-writer.lock"))
                        .map_err(|_| "unable to open integration writer lease".to_owned())?;
                    writer
                        .try_lock()
                        .map_err(|_| "unified history already has a writer".to_owned())?;
                    let mut sources = Vec::new();
                    for (file, table) in [
                        ("history.db", SourceTable::History),
                        ("clipboard.db", SourceTable::Clipboard),
                    ] {
                        let connection = Connection::open_with_flags(
                            root.join(file),
                            OpenFlags::SQLITE_OPEN_READ_WRITE,
                        )
                        .map_err(|_| "source database unavailable".to_owned())?;
                        sources.push(
                            SourcePump::attach(connection, table).map_err(|e| e.to_string())?,
                        );
                    }
                    let mut store =
                        IntegrationStore::open(root.join("integration.db"), &profile_id)
                            .map_err(|e| e.to_string())?;
                    let key = crate::private_key::load_or_create(
                        &root.join("integration-learning.key"),
                        !store.learning_initialized().map_err(|e| e.to_string())?,
                    )
                    .map_err(|_| "learning key unavailable or unsafe".to_owned())?;
                    store.enable_learning(&key).map_err(|e| e.to_string())?;
                    store.initialize_outputs().map_err(|e| e.to_string())?;
                    store
                        .initialize_voice_sessions()
                        .map_err(|e| e.to_string())?;
                    Ok(Worker {
                        output_generation: worker_generation,
                        source_writes: worker_source_writes,
                        _writer_lease: writer,
                        learning_key: key,
                        store,
                        sources,
                        last_error: None,
                        generation: 0,
                        changed: Box::new(changed),
                    })
                })();
                while !stop.load(Ordering::Acquire) {
                    if let Ok(state) = &mut worker {
                        state.last_error = state.sync_once().err();
                    }
                    match receiver.recv_timeout(Duration::from_millis(100)) {
                        Ok(job) => job(&mut worker),
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
            })
            .map_err(|_| "unable to start history worker".to_owned())?;
        Ok(Self {
            output_generation,
            source_writes,
            sender,
            stopping,
            thread: Some(thread),
        })
    }

    fn call<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Worker) -> ServiceResult<T> + Send + 'static,
    ) -> ServiceResult<T> {
        let (sender, receiver) = mpsc::sync_channel(1);
        self.sender
            .try_send(Box::new(move |worker| {
                let result = match worker {
                    Ok(state) => work(state),
                    Err(error) => Err(error.clone()),
                };
                let _ = sender.send(result);
            }))
            .map_err(|_| "history service is busy or stopped".to_owned())?;
        receiver
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| "history service request timed out".to_owned())?
    }

    /// 同步请求仅供阻塞任务池调用，不能在 GUI 或输入法按键线程调用。
    pub fn query(&self, query: HistoryQuery) -> ServiceResult<Vec<IndexedItem>> {
        self.call(move |worker| {
            if let Some(error) = &worker.last_error {
                return Err(error.clone());
            }
            worker.store.query(&query).map_err(|e| e.to_string())
        })
    }

    pub fn revisions(&self, item_id: String) -> ServiceResult<Vec<ContentRevision>> {
        self.call(move |worker| {
            if let Some(error) = &worker.last_error {
                return Err(error.clone());
            }
            worker.store.revisions(&item_id).map_err(|e| e.to_string())
        })
    }

    pub fn get_item(&self, item_id: String, expected_revision: u64) -> ServiceResult<IndexedItem> {
        self.call(move |worker| {
            if let Some(error) = &worker.last_error {
                return Err(error.clone());
            }
            let item = worker
                .store
                .get(&item_id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "history item no longer exists".to_owned())?;
            if item.revision != expected_revision {
                return Err("history item has a newer revision".into());
            }
            Ok(item)
        })
    }

    pub fn update_item(
        &self,
        item_id: String,
        expected_revision: u64,
        operation_id: String,
        patch: crate::source::HistoryPatch,
    ) -> ServiceResult<u64> {
        self.call(move |worker| {
            worker.revoke_outputs();
            let item = worker
                .store
                .get(&item_id)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "history item no longer exists".to_owned())?;
            let source = worker
                .sources
                .iter_mut()
                .find(|source| source.store_id() == item.store_id)
                .ok_or_else(|| "history source unavailable".to_owned())?;
            let result = source
                .update_record(&item.record_id, expected_revision, &operation_id, &patch)
                .map_err(|e| e.to_string())?;
            let revision = result
                .response
                .parse::<u64>()
                .map_err(|_| "invalid source receipt".to_owned())?;
            // 消费到该修订才确认 UI 修改完成；超时重试仍复用相同 operation_id。
            for _ in 0..100 {
                match worker.sync_once() {
                    Ok(_) => {}
                    Err(error) => {
                        worker.last_error = Some(error.clone());
                        return Err(error);
                    }
                }
                if worker
                    .store
                    .get(&item_id)
                    .map_err(|e| e.to_string())?
                    .is_some_and(|current| current.revision >= revision)
                {
                    worker.last_error = None;
                    return Ok(revision);
                }
            }
            Err("source updated but projection is still pending".into())
        })
    }

    pub fn list_terms(&self, limit: u32, offset: u64) -> ServiceResult<Vec<LearnedTermView>> {
        self.call(move |worker| {
            worker
                .store
                .list_terms(limit, offset)
                .map_err(|e| e.to_string())
        })
    }

    pub fn policy_epoch(&self) -> ServiceResult<u64> {
        self.call(|worker| worker.store.policy_epoch().map_err(|e| e.to_string()))
    }

    pub fn prepare_voice_request(
        &self,
        request: crate::voice_protocol::VoiceRequest,
        client: String,
        server: String,
        applied_epoch: Option<u64>,
    ) -> ServiceResult<crate::voice_ledger::SessionRecord> {
        self.call(move |worker| {
            worker
                .store
                .prepare_voice_request(&request, &client, &server, applied_epoch)
                .map_err(|e| e.to_string())
        })
    }
    pub fn claim_voice_request(
        &self,
        request: crate::voice_protocol::VoiceRequest,
        client: String,
        server: String,
        applied_epoch: Option<u64>,
    ) -> ServiceResult<bool> {
        self.call(move |worker| {
            worker
                .store
                .claim_voice_request(&request, &client, &server, applied_epoch)
                .map_err(|e| e.to_string())
        })
    }
    pub fn project_voice_session(
        &self,
        client: String,
        server: String,
        view: crate::voice_protocol::VoiceSessionView,
    ) -> ServiceResult<bool> {
        self.call(move |worker| {
            worker
                .store
                .project_voice_session(&client, &server, &view)
                .map_err(|e| e.to_string())
        })
    }
    pub fn voice_session(
        &self,
        session_id: String,
    ) -> ServiceResult<Option<crate::voice_ledger::SessionRecord>> {
        self.call(move |worker| {
            worker
                .store
                .voice_session(&session_id)
                .map_err(|e| e.to_string())
        })
    }
    pub fn prepare_output(
        &self,
        intent: crate::output_ledger::OutputIntent,
    ) -> ServiceResult<crate::output_ledger::OutputRecord> {
        self.call(move |worker| {
            worker
                .store
                .prepare_output(&intent)
                .map_err(|e| e.to_string())
        })
    }

    /// 按本次保存的源记录定位；不通过最新记录或全文搜索猜测身份。
    pub fn prepare_saved_voice_result(
        &self,
        start: crate::voice_protocol::VoiceRequest,
        history_id: i64,
        expected_text: String,
    ) -> ServiceResult<crate::output_ledger::OutputRecord> {
        self.call(move |worker| {
            if history_id <= 0 || expected_text.is_empty() {
                return Err("invalid saved voice result".into());
            }
            worker.sync_once()?;
            let source = worker
                .sources
                .iter()
                .find(|source| source.source_table() == SourceTable::History)
                .ok_or("history source unavailable")?;
            let id = crate::store::item_id(source.store_id(), &history_id.to_string());
            let item = worker
                .store
                .get(&id)
                .map_err(|error| error.to_string())?
                .ok_or("saved voice result is not projected")?;
            if item.snapshot.source_kind != crate::store::SourceKind::Voice
                || item.snapshot.text.as_deref() != Some(expected_text.as_str())
            {
                return Err("saved voice result changed before output preparation".into());
            }
            let session = worker
                .store
                .voice_session(&start.session_id)
                .map_err(|error| error.to_string())?
                .ok_or("voice session missing")?;
            if session.start != start {
                return Err("voice start identity changed".into());
            }
            worker
                .store
                .prepare_voice_result(
                    &start.session_id,
                    &start.client_instance,
                    &start.server_instance,
                    &id,
                    item.revision,
                )
                .map_err(|error| error.to_string())
        })
    }

    /// 只用于本次转写已保存的源记录；有待消费的源变更先完成投影再准备。
    pub fn prepare_voice_result(
        &self,
        session_id: String,
        client: String,
        server: String,
        item_id: String,
        revision: u64,
    ) -> ServiceResult<crate::output_ledger::OutputRecord> {
        self.call(move |worker| {
            worker.sync_once()?;
            worker
                .store
                .prepare_voice_result(&session_id, &client, &server, &item_id, revision)
                .map_err(|error| error.to_string())
        })
    }

    pub fn voice_result(
        &self,
        session_id: String,
    ) -> ServiceResult<Option<crate::output_ledger::OutputRecord>> {
        self.call(move |worker| {
            worker
                .store
                .voice_result(&session_id)
                .map_err(|error| error.to_string())
        })
    }
    pub fn claim_output(&self, intent: crate::output_ledger::OutputIntent) -> ServiceResult<bool> {
        self.call(move |worker| {
            worker
                .store
                .claim_output(&intent)
                .map_err(|e| e.to_string())
        })
    }

    /// 取得执行所有权与短期许可在同一后台请求内完成。许可不得跨进程持久化。
    pub fn claim_output_with_permit(
        &self,
        intent: crate::output_ledger::OutputIntent,
    ) -> ServiceResult<Option<OutputPermit>> {
        self.call(move |worker| {
            // 外部 manager 的源事务与投影分别持连接；先验证完整同步屏障，
            // 不能在“已撤销旧许可、尚未投影”的窗口签发另一个旧内容许可。
            let source_generation = worker.source_writes.generation.load(Ordering::Acquire);
            if worker.source_writes.active.load(Ordering::Acquire) != 0 {
                return Err("source mutation is in progress".into());
            }
            let mut caught_up = false;
            for _ in 0..100 {
                if !worker.sync_once()? {
                    caught_up = true;
                    break;
                }
            }
            let expected_generation = worker.output_generation.load(Ordering::Acquire);
            if !caught_up
                || worker.source_writes.active.load(Ordering::Acquire) != 0
                || worker.source_writes.generation.load(Ordering::Acquire) != source_generation
            {
                return Err("source changed during output authorization".into());
            }
            if worker.last_error.is_some() {
                return Err("history synchronization unavailable".into());
            }
            if !worker
                .store
                .claim_output(&intent)
                .map_err(|e| e.to_string())?
            {
                return Ok(None);
            }
            Ok(Some(OutputPermit {
                generation: worker.output_generation.clone(),
                expected: expected_generation,
                expires: Instant::now() + Duration::from_secs(2),
            }))
        })
    }

    /// 必须在任何源正文/元数据/附件变更之前持有此守卫，直到源事务提交。
    pub fn begin_source_write(&self) -> SourceWriteGuard {
        self.source_writes.active.fetch_add(1, Ordering::AcqRel);
        self.source_writes.generation.fetch_add(1, Ordering::AcqRel);
        self.output_generation.fetch_add(1, Ordering::AcqRel);
        SourceWriteGuard {
            source: self.source_writes.clone(),
        }
    }
    pub fn finish_output(
        &self,
        intent: crate::output_ledger::OutputIntent,
        outcome: crate::output_ledger::OutputOutcome,
    ) -> ServiceResult<crate::output_ledger::OutputRecord> {
        self.call(move |worker| {
            worker
                .store
                .finish_output(&intent, outcome)
                .map_err(|e| e.to_string())
        })
    }
    pub fn output_record(
        &self,
        id: String,
    ) -> ServiceResult<Option<crate::output_ledger::OutputRecord>> {
        self.call(move |worker| worker.store.output_record(&id).map_err(|e| e.to_string()))
    }

    /// 模型调用方提供已捕获的真实目标策略；此方法不自行猜测前台来源。
    pub fn session_hotwords(
        &self,
        policy: PrivacyPolicy,
        context: PrivacyContext,
        explicit: Vec<String>,
        budget: HotwordBudget,
    ) -> ServiceResult<TermSnapshot> {
        self.call(move |worker| {
            worker
                .store
                .term_snapshot(&worker.learning_key, &policy, context, &explicit, budget)
                .map_err(|e| e.to_string())
        })
    }

    pub fn term_snapshot_is_current(&self, snapshot: TermSnapshot) -> ServiceResult<bool> {
        self.call(move |worker| {
            worker
                .store
                .term_snapshot_is_current(&snapshot)
                .map_err(|e| e.to_string())
        })
    }

    /// 测试和显式刷新使用的有界屏障；达到上限报告未追平，不冒充同步完成。
    pub fn synchronize(&self) -> ServiceResult<u64> {
        self.call(|worker| {
            for _ in 0..100 {
                match worker.sync_once() {
                    Ok(false) => {
                        worker.last_error = None;
                        return Ok(worker.generation);
                    }
                    Ok(true) => {}
                    Err(error) => {
                        worker.last_error = Some(error.clone());
                        return Err(error);
                    }
                }
            }
            Err("history backlog requires another synchronization pass".into())
        })
    }
}

impl Drop for HistoryService {
    fn drop(&mut self) {
        self.output_generation.fetch_add(1, Ordering::AcqRel);
        self.stopping.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod output_permit_tests {
    use super::*;

    #[test]
    fn delayed_voice_claim_times_out_without_granting_caller_execution_or_reclaim() {
        use crate::voice_protocol::*;
        let root = tempfile::tempdir().unwrap();
        let history = Connection::open(root.path().join("history.db")).unwrap();
        history.execute_batch("CREATE TABLE transcription_history(id INTEGER PRIMARY KEY,file_name TEXT,timestamp INTEGER,saved INTEGER,title TEXT,transcription_text TEXT,post_processed_text TEXT);").unwrap();
        Connection::open(root.path().join("clipboard.db")).unwrap().execute_batch("CREATE TABLE clipboard_history(id INTEGER PRIMARY KEY,content_type TEXT,full_text TEXT,title TEXT,is_favorite INTEGER,is_pinned INTEGER,created_at INTEGER,image_path TEXT,source_app TEXT);").unwrap();
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let once = AtomicBool::new(true);
        let service = HistoryService::start(root.path().into(), "fixture".into(), move |_| {
            if once.swap(false, Ordering::AcqRel) {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            }
        })
        .unwrap();
        let request = VoiceRequest {
            request_id: "start".into(),
            session_id: "voice".into(),
            client_instance: "host".into(),
            server_instance: "server".into(),
            policy_epoch: 1,
            command: VoiceCommand::Start {
                target: HostTargetToken {
                    target_id: "target".into(),
                    host_instance: "host".into(),
                    controller_id: "controller".into(),
                    activation_generation: 1,
                    field_id: None,
                    selection_generation: 0,
                    composition_generation: 0,
                    source_app: None,
                },
                post_process: false,
                terms: VoiceTermsVersion {
                    policy_epoch: 1,
                    learning_generation: 0,
                },
            },
        };
        service
            .prepare_voice_request(request.clone(), "host".into(), "server".into(), Some(1))
            .unwrap();
        history
            .execute_batch(
                "INSERT INTO transcription_history VALUES(1,NULL,1,0,NULL,'synthetic',NULL);",
            )
            .unwrap();
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let result =
            service.claim_voice_request(request.clone(), "host".into(), "server".into(), Some(1));
        assert!(result.is_err(), "超时不能返回执行资格");
        release_tx.send(()).unwrap();
        // 队列随后可能提交已消费claim；重试只能查询事实，不再次启动。
        assert!(!service
            .claim_voice_request(request, "host".into(), "server".into(), Some(1))
            .unwrap());
        assert_eq!(
            service
                .voice_session("voice".into())
                .unwrap()
                .unwrap()
                .view
                .phase,
            VoicePhase::Preparing
        );
    }

    #[test]
    fn stale_and_expired_permits_fail_without_a_service_call() {
        let generation = Arc::new(AtomicU64::new(1));
        let mut permit = OutputPermit {
            generation: generation.clone(),
            expected: 1,
            expires: Instant::now() + Duration::from_secs(2),
        };
        assert!(permit.check().is_ok());
        generation.fetch_add(1, Ordering::AcqRel);
        assert!(permit.check().is_err());
        permit.expected = 2;
        assert!(permit.check().is_ok());
        permit.expires = Instant::now();
        assert!(permit.check().is_err());
    }
}
