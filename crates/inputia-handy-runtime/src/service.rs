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
        atomic::{AtomicBool, Ordering},
        mpsc::{self, SyncSender},
        Arc,
    },
    thread,
    time::Duration,
};

type ServiceResult<T> = Result<T, String>;
type Job = Box<dyn FnOnce(&mut ServiceResult<Worker>) + Send>;

struct Worker {
    learning_key: [u8; 32],
    store: IntegrationStore,
    sources: Vec<SourcePump>,
    last_error: Option<String>,
    generation: u64,
    changed: Box<dyn Fn(u64) + Send>,
}

impl Worker {
    fn sync_once(&mut self) -> ServiceResult<bool> {
        let mut changed = false;
        let mut failure = None;
        for source in &mut self.sources {
            match source.sync_batch(&mut self.store) {
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
        failure.map_or(Ok(changed), Err)
    }
}

pub struct HistoryService {
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
        let thread = thread::Builder::new()
            .name("handy-unified-history".into())
            .spawn(move || {
                let mut worker = (|| {
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
                    Ok(Worker {
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

    pub fn list_terms(&self, limit: u32, offset: u64) -> ServiceResult<Vec<LearnedTermView>> {
        self.call(move |worker| {
            worker
                .store
                .list_terms(limit, offset)
                .map_err(|e| e.to_string())
        })
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
        self.stopping.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
