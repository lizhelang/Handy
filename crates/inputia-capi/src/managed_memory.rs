//! 输入法只持查询快照。时间从本地发起查询计起，迟到回复不能为旧数据重新发租约。
//! 本模块不打开数据库；认证、真实目标和来源授权由配对连接执行。
use inputia_core::{
    memory_snapshot::{MemoryQuery, MemorySnapshot},
    AppPolicy, LocalMemory, MemoryTerm,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, OnceLock, Weak,
    },
    time::{Duration, Instant},
};

const LEASE_MS: u64 = 2_000;
static NEXT_TICKET: AtomicU64 = AtomicU64::new(1);
type Result<T> = std::result::Result<T, &'static str>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyIdentity {
    pub server_instance: String,
    pub profile_id: String,
    pub policy_epoch: u64,
}
impl PolicyIdentity {
    fn validate(&self) -> Result<()> {
        if !identifier(&self.server_instance)
            || !identifier(&self.profile_id)
            || self.policy_epoch == 0
            || self.policy_epoch > i64::MAX as u64
        {
            Err("memory_policy_invalid")
        } else {
            Ok(())
        }
    }
}
fn identifier(value: &str) -> bool {
    inputia_core::integration::events::Identifier::parse(value.to_owned()).is_ok()
}
#[derive(Default)]
struct ProcessPolicy {
    identity: Option<PolicyIdentity>,
    generation: u64,
    retired_servers: BTreeSet<String>,
    sessions: Vec<Weak<Mutex<ManagedMemory>>>,
    clearing_generation: Option<u64>,
    domain: Option<(String, u64)>,
}
impl ProcessPolicy {
    fn apply(&mut self, value: PolicyIdentity) -> Result<u64> {
        value.validate()?;
        if let Some(current) = &self.identity {
            if current.profile_id != value.profile_id {
                return Err("memory_profile_changed");
            }
            if value.policy_epoch < current.policy_epoch {
                return Err("memory_policy_retired");
            }
            if current.server_instance == value.server_instance {
                if *current == value {
                    return Ok(self.generation);
                }
            } else {
                // 重启实例允许继续；迟到的旧连接不能再次被提升为当前服务。
                if self.retired_servers.contains(&value.server_instance)
                    || self.retired_servers.len() >= 64
                {
                    return Err("memory_server_retired");
                }
                self.retired_servers.insert(current.server_instance.clone());
            }
        }
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or("memory_generation_exhausted")?;
        self.identity = Some(value);
        Ok(self.generation)
    }
}
fn process_policy() -> Arc<Mutex<ProcessPolicy>> {
    static POLICY: OnceLock<Arc<Mutex<ProcessPolicy>>> = OnceLock::new();
    POLICY
        .get_or_init(|| Arc::new(Mutex::new(ProcessPolicy::default())))
        .clone()
}
/// 每次已认证屏障均清除所有会话，即使服务身份与epoch未变。
/// 调用方仅可在本函数成功且宿主自身缓存也已清除后确认屏障。
pub fn apply_process_policy(policy: PolicyIdentity) -> Result<u64> {
    apply_policy(&process_policy(), policy)
}
fn apply_policy(shared: &Arc<Mutex<ProcessPolicy>>, identity: PolicyIdentity) -> Result<u64> {
    let (generation, sessions) = {
        let mut policy = shared.lock().map_err(|_| "memory_policy_unavailable")?;
        let old = policy.generation;
        let mut generation = policy.apply(identity)?;
        if generation == old {
            generation = generation
                .checked_add(1)
                .ok_or("memory_generation_exhausted")?;
            policy.generation = generation;
        }
        policy.clearing_generation = Some(generation);
        policy.sessions.retain(|session| session.strong_count() > 0);
        (
            generation,
            policy
                .sessions
                .iter()
                .filter_map(Weak::upgrade)
                .collect::<Vec<_>>(),
        )
    };
    // 先释放策略锁，避免与按键侧 session→policy 的锁顺序相反。
    // 返回成功前清除每个存活session的正文缓存；仅改代数不足以确认清理完成。
    for session in sessions {
        session
            .lock()
            .map_err(|_| "memory_session_unavailable")?
            .clear();
    }
    let mut policy = shared.lock().map_err(|_| "memory_policy_unavailable")?;
    if policy.generation != generation {
        return Err("memory_policy_retired");
    }
    policy.clearing_generation = None;
    Ok(generation)
}

#[derive(Clone, Serialize)]
pub struct QueryTicket {
    pub ticket: u64,
    pub query: MemoryQuery,
    pub composing: String,
    pub policy: PolicyIdentity,
}
struct Pending {
    ticket: QueryTicket,
    process_generation: u64,
    started: Instant,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallSnapshot {
    pub ticket: u64,
    pub policy: PolicyIdentity,
    pub domain_uuid: String,
    pub generation: u64,
    pub query: MemoryQuery,
    pub composing: String,
    pub terms: Vec<MemoryTerm>,
    pub max_age_ms: u64,
}
struct Cached {
    snapshot: MemorySnapshot,
    composing: String,
    process_generation: u64,
    deadline: Instant,
    ticket: u64,
    domain_generation: u64,
}

/// 每个session拥有自己的查询票据；并发后台只交回复值，不接触Rime。
pub struct ManagedMemory {
    policy: Arc<Mutex<ProcessPolicy>>,
    enabled: bool,
    pending: BTreeMap<u8, Pending>,
    cached: BTreeMap<u8, Cached>,
    displayed_rank: Option<(u64, u64, u64, Instant)>,
    rank_was_revoked: bool,
}
impl ManagedMemory {
    pub fn new(enabled: bool) -> Result<Arc<Mutex<Self>>> {
        Self::shared_with_policy(enabled, process_policy())
    }
    fn shared_with_policy(
        enabled: bool,
        policy: Arc<Mutex<ProcessPolicy>>,
    ) -> Result<Arc<Mutex<Self>>> {
        let mut current = policy.lock().map_err(|_| "memory_policy_unavailable")?;
        current
            .sessions
            .retain(|session| session.strong_count() > 0);
        if current.sessions.len() >= 256 {
            return Err("memory_session_budget");
        }
        let session = Arc::new(Mutex::new(Self::with_policy(enabled, policy.clone())));
        current.sessions.push(Arc::downgrade(&session));
        Ok(session)
    }
    fn with_policy(enabled: bool, policy: Arc<Mutex<ProcessPolicy>>) -> Self {
        Self {
            policy,
            enabled,
            pending: BTreeMap::new(),
            cached: BTreeMap::new(),
            displayed_rank: None,
            rank_was_revoked: false,
        }
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    pub fn begin(&mut self, query: MemoryQuery, composing: String) -> Result<QueryTicket> {
        self.begin_at(query, composing, Instant::now())
    }
    fn begin_at(
        &mut self,
        query: MemoryQuery,
        composing: String,
        started: Instant,
    ) -> Result<QueryTicket> {
        if !self.enabled {
            return Err("memory_disabled");
        }
        query.validate().map_err(|_| "memory_query_invalid")?;
        if composing.len() > 512 || composing.chars().any(char::is_control) {
            return Err("memory_query_invalid");
        }
        let policy = self
            .policy
            .lock()
            .map_err(|_| "memory_policy_unavailable")?;
        if policy.clearing_generation.is_some() {
            return Err("memory_policy_clearing");
        }
        let identity = policy.identity.clone().ok_or("memory_policy_required")?;
        let ticket = NEXT_TICKET
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |old| {
                old.checked_add(1)
            })
            .map_err(|_| "memory_ticket_exhausted")?;
        let ticket = QueryTicket {
            ticket,
            query,
            composing,
            policy: identity,
        };
        self.pending.insert(
            kind(&ticket.query),
            Pending {
                ticket: ticket.clone(),
                process_generation: policy.generation,
                started,
            },
        );
        Ok(ticket)
    }
    pub fn install(&mut self, value: InstallSnapshot) -> Result<()> {
        self.install_with_clock(value, Instant::now)
    }
    #[cfg(test)]
    fn install_at(&mut self, value: InstallSnapshot, now: Instant) -> Result<()> {
        self.install_with_clock(value, || now)
    }
    fn install_with_clock(
        &mut self,
        value: InstallSnapshot,
        clock: impl Fn() -> Instant,
    ) -> Result<()> {
        if !self.enabled
            || !identifier(&value.domain_uuid)
            || value.generation == 0
            || !(1..=LEASE_MS).contains(&value.max_age_ms)
        {
            return Err("memory_snapshot_invalid");
        }
        let query_kind = kind(&value.query);
        let pending = self
            .pending
            .get(&query_kind)
            .ok_or("memory_query_retired")?;
        if pending.ticket.ticket != value.ticket
            || pending.ticket.query != value.query
            || pending.ticket.composing != value.composing
            || pending.ticket.policy != value.policy
        {
            return Err("memory_query_retired");
        }
        let mut policy = self
            .policy
            .lock()
            .map_err(|_| "memory_policy_unavailable")?;
        if policy.clearing_generation.is_some() {
            return Err("memory_policy_clearing");
        }
        if policy.generation != pending.process_generation
            || policy.identity.as_ref() != Some(&value.policy)
        {
            return Err("memory_policy_retired");
        }
        let deadline = pending
            .started
            .checked_add(Duration::from_millis(value.max_age_ms))
            .ok_or("memory_snapshot_invalid")?;
        if clock() >= deadline {
            return Err("memory_snapshot_expired");
        }
        if let Some((domain, generation)) = &policy.domain {
            if domain != &value.domain_uuid || value.generation < *generation {
                return Err("memory_domain_retired");
            }
        }
        let snapshot =
            MemorySnapshot::new(value.query, value.terms).map_err(|_| "memory_snapshot_invalid")?;
        if clock() >= deadline {
            return Err("memory_snapshot_expired");
        }
        policy.domain = Some((value.domain_uuid, value.generation));
        self.cached.insert(
            query_kind,
            Cached {
                snapshot,
                composing: value.composing,
                process_generation: policy.generation,
                deadline,
                ticket: value.ticket,
                domain_generation: value.generation,
            },
        );
        self.pending.remove(&query_kind);
        Ok(())
    }
    pub fn memory_for(&mut self, query: &MemoryQuery, composing: &str) -> Result<LocalMemory> {
        self.memory_for_with_clock(query, composing, Instant::now)
    }
    /// 引擎可只请求已完整覆盖集合的子集；任何新候选都要求新查询，不能按全局热门词补猜。
    pub fn rank_memory_for(
        &mut self,
        candidates: &[inputia_core::Candidate],
        composing: &str,
    ) -> Result<LocalMemory> {
        let query = self
            .cached
            .get(&0)
            .ok_or("memory_snapshot_required")?
            .snapshot
            .query()
            .clone();
        let MemoryQuery::Rank { candidate_texts } = &query else {
            return Err("memory_query_invalid");
        };
        if candidates
            .iter()
            .any(|candidate| !candidate_texts.contains(&candidate.text))
        {
            return Err("memory_query_retired");
        }
        self.memory_for(&query, composing)
    }
    /// 普通按键/导航推进当前输入状态后，不接受此前发起的异步重排结果。
    pub fn cancel_pending(&mut self) {
        self.pending.clear();
    }
    #[cfg(test)]
    fn memory_for_at(
        &mut self,
        query: &MemoryQuery,
        composing: &str,
        now: Instant,
    ) -> Result<LocalMemory> {
        self.memory_for_with_clock(query, composing, || now)
    }
    fn memory_for_with_clock(
        &mut self,
        query: &MemoryQuery,
        composing: &str,
        clock: impl Fn() -> Instant,
    ) -> Result<LocalMemory> {
        if !self.enabled {
            return Err("memory_disabled");
        }
        let cached = self
            .cached
            .get(&kind(query))
            .ok_or("memory_snapshot_required")?;
        let policy = self
            .policy
            .lock()
            .map_err(|_| "memory_policy_unavailable")?;
        if policy.clearing_generation.is_some() {
            return Err("memory_policy_clearing");
        }
        if cached.process_generation != policy.generation
            || clock() >= cached.deadline
            || policy.domain.as_ref().map(|(_, generation)| *generation)
                != Some(cached.domain_generation)
        {
            return Err("memory_snapshot_expired");
        }
        if composing != cached.composing {
            return Err("memory_query_retired");
        }
        let memory = cached
            .snapshot
            .memory_for(query, AppPolicy::default())
            .map_err(|_| "memory_query_retired")?;
        if clock() >= cached.deadline {
            return Err("memory_snapshot_expired");
        }
        if matches!(query, MemoryQuery::Rank { .. }) {
            self.displayed_rank = Some((
                cached.ticket,
                cached.process_generation,
                cached.domain_generation,
                cached.deadline,
            ));
        }
        Ok(memory)
    }
    /// 返回true时宿主必须撤掉已排序候选，并拒绝旧索引选择；不能改序后选同一个数字。
    pub fn take_rank_revocation(&mut self) -> bool {
        self.take_rank_revocation_with_clock(Instant::now)
    }
    #[cfg(test)]
    fn take_rank_revocation_at(&mut self, now: Instant) -> bool {
        self.take_rank_revocation_with_clock(|| now)
    }
    fn take_rank_revocation_with_clock(&mut self, clock: impl Fn() -> Instant) -> bool {
        if self.rank_was_revoked {
            self.rank_was_revoked = false;
            return true;
        }
        let Some((_, generation, domain_generation, deadline)) = self.displayed_rank else {
            return false;
        };
        let current = self.policy.lock().ok().and_then(|policy| {
            policy.clearing_generation.is_none().then_some((
                policy.generation,
                policy.domain.as_ref().map(|(_, generation)| *generation),
            ))
        });
        let now = clock();
        if now >= deadline || current != Some((generation, Some(domain_generation))) {
            // 新一代查询刚安装时，只撤掉旧代正文；不能连有效的新回复也一起丢掉。
            self.cached.retain(|_, cached| {
                now < cached.deadline
                    && current == Some((cached.process_generation, Some(cached.domain_generation)))
            });
            self.pending.clear();
            self.displayed_rank = None;
            true
        } else {
            false
        }
    }

    /// Core推进至其他组合或已提交后，旧视图不再拥有下一次普通空格/数字。
    pub fn finish_rank_view(&mut self, composing: &str, uses_memory_scores: bool) {
        if !uses_memory_scores
            || composing.is_empty()
            || self
                .cached
                .get(&0)
                .is_none_or(|cached| cached.composing != composing)
        {
            self.displayed_rank = None;
            self.rank_was_revoked = false;
            self.cached.remove(&0);
            self.pending.remove(&0);
        }
    }
    pub fn clear(&mut self) {
        self.rank_was_revoked |= self.displayed_rank.is_some();
        self.cached.clear();
        self.pending.clear();
        self.displayed_rank = None;
    }
}
fn kind(query: &MemoryQuery) -> u8 {
    match query {
        MemoryQuery::Rank { .. } => 0,
        MemoryQuery::Completion { .. } => 1,
        MemoryQuery::EnglishCompletion { .. } => 2,
        MemoryQuery::Clipboard { .. } => 3,
        MemoryQuery::VoiceHotwords { .. } => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn identity(epoch: u64) -> PolicyIdentity {
        PolicyIdentity {
            server_instance: "server-1".into(),
            profile_id: "profile-1".into(),
            policy_epoch: epoch,
        }
    }
    fn query() -> MemoryQuery {
        MemoryQuery::Rank {
            candidate_texts: vec!["你".into(), "泥".into()],
        }
    }
    fn fixture() -> (Arc<Mutex<ProcessPolicy>>, ManagedMemory) {
        let policy = Arc::new(Mutex::new(ProcessPolicy::default()));
        policy.lock().unwrap().apply(identity(1)).unwrap();
        let memory = ManagedMemory::with_policy(true, policy.clone());
        (policy, memory)
    }
    fn reply(ticket: &QueryTicket) -> InstallSnapshot {
        InstallSnapshot {
            ticket: ticket.ticket,
            policy: ticket.policy.clone(),
            domain_uuid: "domain-1".into(),
            generation: 1,
            query: ticket.query.clone(),
            composing: ticket.composing.clone(),
            terms: vec![MemoryTerm {
                text: "泥".into(),
                typed_count: 3,
                voice_count: 0,
                clipboard_count: 0,
                last_used_tick: 1,
            }],
            max_age_ms: 2_000,
        }
    }
    #[test]
    fn late_response_does_not_renew_deadline_or_reuse_a_replaced_query() {
        let (_, mut memory) = fixture();
        let start = Instant::now();
        let ticket = memory.begin_at(query(), "ni".into(), start).unwrap();
        memory
            .install_at(reply(&ticket), start + Duration::from_millis(1_999))
            .unwrap();
        assert!(memory
            .memory_for_at(&query(), "ni", start + Duration::from_millis(1_999))
            .is_ok());
        assert!(memory
            .memory_for_at(&query(), "ni", start + Duration::from_millis(2_000))
            .is_err());
        assert!(memory.take_rank_revocation_at(start + Duration::from_millis(2_000)));
        let old = memory.begin_at(query(), "ni".into(), start).unwrap();
        let new = memory.begin_at(query(), "ni".into(), start).unwrap();
        assert!(memory.install_at(reply(&old), start).is_err());
        memory.install_at(reply(&new), start).unwrap();
        assert!(memory.install_at(reply(&new), start).is_err());
    }
    #[test]
    fn process_barrier_revokes_every_session_and_old_server_cannot_return() {
        let (policy, mut first) = fixture();
        let mut second = ManagedMemory::with_policy(true, policy.clone());
        let start = Instant::now();
        for memory in [&mut first, &mut second] {
            let ticket = memory.begin_at(query(), "ni".into(), start).unwrap();
            memory.install_at(reply(&ticket), start).unwrap();
            memory.memory_for_at(&query(), "ni", start).unwrap();
        }
        policy.lock().unwrap().apply(identity(2)).unwrap();
        assert!(first.take_rank_revocation_at(start));
        assert!(second.take_rank_revocation_at(start));
        assert!(first.memory_for_at(&query(), "ni", start).is_err());
        let mut new = identity(2);
        new.server_instance = "server-2".into();
        policy.lock().unwrap().apply(new).unwrap();
        assert!(policy.lock().unwrap().apply(identity(3)).is_err());
    }
    #[test]
    fn wrong_scope_epoch_domain_and_generation_are_rejected() {
        let (policy, mut memory) = fixture();
        let start = Instant::now();
        let ticket = memory.begin_at(query(), "ni".into(), start).unwrap();
        let mut wrong = reply(&ticket);
        wrong.composing = "nin".into();
        assert!(memory.install_at(wrong, start).is_err());
        let mut wrong = reply(&ticket);
        wrong.policy.policy_epoch = 2;
        assert!(memory.install_at(wrong, start).is_err());
        let mut high = reply(&ticket);
        high.generation = 3;
        memory.install_at(high, start).unwrap();
        let next = memory.begin_at(query(), "ni".into(), start).unwrap();
        assert!(memory.install_at(reply(&next), start).is_err());
        let mut other = reply(&next);
        other.generation = 4;
        other.domain_uuid = "other-domain".into();
        assert!(memory.install_at(other, start).is_err());
        policy.lock().unwrap().apply(identity(2)).unwrap();
        assert!(memory.install_at(reply(&next), start).is_err());
        assert!(memory.memory_for_at(&query(), "ni", start).is_err());
    }
    #[test]
    fn domain_floor_survives_epoch_server_and_session_changes() {
        let (policy, mut first) = fixture();
        let start = Instant::now();
        let ticket = first.begin_at(query(), "ni".into(), start).unwrap();
        let mut high = reply(&ticket);
        high.generation = 10;
        first.install_at(high, start).unwrap();
        let mut next_identity = identity(2);
        next_identity.server_instance = "server-2".into();
        apply_policy(&policy, next_identity).unwrap();
        let mut next = ManagedMemory::with_policy(true, policy.clone());
        let ticket = next.begin_at(query(), "ni".into(), start).unwrap();
        assert_eq!(
            next.install_at(reply(&ticket), start),
            Err("memory_domain_retired")
        );
        let mut other = reply(&ticket);
        other.domain_uuid = "domain-2".into();
        other.generation = 11;
        assert_eq!(next.install_at(other, start), Err("memory_domain_retired"));
        let mut valid = reply(&ticket);
        valid.generation = 10;
        next.install_at(valid, start).unwrap();
    }
    #[test]
    fn newer_generation_invalidates_other_query_kinds_and_sessions() {
        let (policy, mut first) = fixture();
        let mut second = ManagedMemory::with_policy(true, policy);
        let start = Instant::now();
        for memory in [&mut first, &mut second] {
            let ticket = memory.begin_at(query(), "ni".into(), start).unwrap();
            memory.install_at(reply(&ticket), start).unwrap();
            memory.memory_for_at(&query(), "ni", start).unwrap();
        }
        let clipboard = MemoryQuery::Clipboard { limit: 5 };
        let ticket = first
            .begin_at(clipboard.clone(), "ni".into(), start)
            .unwrap();
        let mut newer = reply(&ticket);
        newer.terms.clear();
        newer.generation = 2;
        first.install_at(newer, start).unwrap();
        assert!(first.memory_for_at(&query(), "ni", start).is_err());
        assert!(second.memory_for_at(&query(), "ni", start).is_err());
        assert!(first.take_rank_revocation_at(start));
        assert!(second.take_rank_revocation_at(start));
        assert!(first.memory_for_at(&clipboard, "ni", start).is_ok());
    }
    #[test]
    fn every_barrier_physically_clears_registered_sessions_even_at_same_epoch() {
        let policy = Arc::new(Mutex::new(ProcessPolicy::default()));
        apply_policy(&policy, identity(1)).unwrap();
        let sessions = [
            ManagedMemory::shared_with_policy(true, policy.clone()).unwrap(),
            ManagedMemory::shared_with_policy(true, policy.clone()).unwrap(),
        ];
        for _ in 0..2 {
            for shared in &sessions {
                let mut memory = shared.lock().unwrap();
                let ticket = memory.begin(query(), "ni".into()).unwrap();
                memory.install(reply(&ticket)).unwrap();
                memory.memory_for(&query(), "ni").unwrap();
            }
            apply_policy(&policy, identity(1)).unwrap();
            for shared in &sessions {
                let mut memory = shared.lock().unwrap();
                assert!(memory.cached.is_empty());
                assert!(memory.pending.is_empty());
                assert!(memory.take_rank_revocation());
            }
        }
    }
    #[test]
    fn failed_barrier_cannot_ack_until_all_sessions_are_cleared() {
        let policy = Arc::new(Mutex::new(ProcessPolicy::default()));
        apply_policy(&policy, identity(1)).unwrap();
        let shared = ManagedMemory::shared_with_policy(true, policy.clone()).unwrap();
        let poison = shared.clone();
        assert!(std::thread::spawn(move || {
            let _guard = poison.lock().unwrap();
            panic!("synthetic session failure");
        })
        .join()
        .is_err());
        assert_eq!(
            apply_policy(&policy, identity(1)),
            Err("memory_session_unavailable")
        );
        assert!(policy.lock().unwrap().clearing_generation.is_some());
        assert_eq!(
            apply_policy(&policy, identity(1)),
            Err("memory_session_unavailable")
        );
        shared.clear_poison();
        assert_eq!(
            shared.lock().unwrap().begin(query(), "ni".into()).err(),
            Some("memory_policy_clearing")
        );
        apply_policy(&policy, identity(1)).unwrap();
        assert!(policy.lock().unwrap().clearing_generation.is_none());
    }
    #[test]
    fn waiting_for_policy_lock_cannot_return_an_expired_snapshot_or_selection() {
        for selection in [false, true] {
            let (policy, mut memory) = fixture();
            let ticket = memory.begin(query(), "ni".into()).unwrap();
            let mut value = reply(&ticket);
            value.max_age_ms = 40;
            memory.install(value).unwrap();
            memory.memory_for(&query(), "ni").unwrap();
            let held = policy.lock().unwrap();
            let worker = std::thread::spawn(move || {
                if selection {
                    memory.take_rank_revocation()
                } else {
                    memory.memory_for(&query(), "ni").is_err()
                }
            });
            std::thread::sleep(Duration::from_millis(60));
            drop(held);
            assert!(worker.join().unwrap());
        }
    }
    #[test]
    fn copying_snapshot_cannot_extend_deadline() {
        let (_, mut memory) = fixture();
        let start = Instant::now();
        let ticket = memory.begin_at(query(), "ni".into(), start).unwrap();
        memory.install_at(reply(&ticket), start).unwrap();
        let count = std::cell::Cell::new(0);
        assert!(memory
            .memory_for_with_clock(&query(), "ni", || {
                count.set(count.get() + 1);
                if count.get() == 1 {
                    start
                } else {
                    start + Duration::from_millis(2000)
                }
            })
            .is_err());
    }
    #[test]
    fn finished_or_base_only_view_retires_old_rank_without_swallowing_next_key() {
        for (composing, uses_memory) in [("", false), ("nia", false), ("ni", false)] {
            let (policy, mut memory) = fixture();
            let start = Instant::now();
            let ticket = memory.begin_at(query(), "ni".into(), start).unwrap();
            memory.install_at(reply(&ticket), start).unwrap();
            memory.memory_for_at(&query(), "ni", start).unwrap();
            memory.finish_rank_view(composing, uses_memory);
            policy.lock().unwrap().apply(identity(2)).unwrap();
            assert!(!memory.take_rank_revocation_at(start + Duration::from_secs(3)));
        }
    }
    #[test]
    fn disabled_and_unconfigured_states_do_not_create_a_memory_lease() {
        let empty = Arc::new(Mutex::new(ProcessPolicy::default()));
        let mut memory = ManagedMemory::with_policy(true, empty);
        assert!(memory.begin(query(), "ni".into()).is_err());
        let (policy, _) = fixture();
        let mut disabled = ManagedMemory::with_policy(false, policy);
        assert!(disabled.begin(query(), "ni".into()).is_err());
    }
}
