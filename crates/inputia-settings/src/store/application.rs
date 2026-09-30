//! 短租约的输入引擎应用观察。与耐久保存回执分离，不将 settings-only 进程视为引擎。
use super::*;
const OBSERVATIONS: &str = ".inputia-settings-applications.json";
const LEASE_MS: u64 = 2_500;
const PROCESS_LIMIT: usize = 16;
const SESSION_LIMIT: usize = 64;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessIdentity {
    pid: u32,
    start_seconds: u64,
    start_microseconds: u64,
}
fn identity(pid: u32, uid: u32) -> Result<ProcessIdentity> {
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::uninit();
    let size = std::mem::size_of::<libc::proc_bsdinfo>();
    let received = unsafe {
        libc::proc_pidinfo(
            pid.try_into().map_err(|_| Error::InvalidRequest)?,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size as i32,
        )
    };
    if received != size as i32 {
        return Err(Error::StorageUnavailable);
    }
    let info = unsafe { info.assume_init() };
    if info.pbi_pid != pid || info.pbi_uid != uid || info.pbi_status == libc::SZOMB {
        return Err(Error::UnsafePath);
    }
    Ok(ProcessIdentity {
        pid,
        start_seconds: info.pbi_start_tvsec,
        start_microseconds: info.pbi_start_tvusec,
    })
}
fn now_ms() -> Result<u64> {
    let mut time = std::mem::MaybeUninit::<libc::timespec>::uninit();
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC_RAW, time.as_mut_ptr()) } != 0 {
        return Err(Error::StorageUnavailable);
    }
    let time = unsafe { time.assume_init() };
    let seconds: u64 = time
        .tv_sec
        .try_into()
        .map_err(|_| Error::StorageUnavailable)?;
    seconds
        .checked_mul(1000)
        .and_then(|v| v.checked_add((time.tv_nsec / 1_000_000) as u64))
        .ok_or(Error::StorageUnavailable)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationEntry {
    pub instance_id: String,
    pub store_id: String,
    pub revision: String,
    pub values_digest: String,
    pub applied_fields: BTreeSet<String>,
    pub unavailable_fields: BTreeSet<String>,
}
impl ApplicationEntry {
    pub fn new(snapshot: &Snapshot) -> Self {
        Self {
            instance_id: uuid::Uuid::new_v4().to_string(),
            store_id: snapshot.store_id.clone(),
            revision: snapshot.revision.clone(),
            values_digest: snapshot.values_digest.clone(),
            applied_fields: BTreeSet::new(),
            unavailable_fields: BTreeSet::new(),
        }
    }
    fn validate(&self) -> Result<()> {
        let known =
            serde_json::to_value(InputiaSettings::default()).map_err(|_| Error::InvalidDocument)?;
        if !valid_uuid(&self.instance_id)
            || !valid_uuid(&self.store_id)
            || revision(&self.revision).is_err()
            || !is_digest(&self.values_digest)
            || self.applied_fields.len() > 32
            || self.unavailable_fields.len() > 32
            || self
                .applied_fields
                .iter()
                .chain(&self.unavailable_fields)
                .any(|key| known.get(key).is_none() || key == "menu_icon_variant")
            || self
                .applied_fields
                .intersection(&self.unavailable_fields)
                .next()
                .is_some()
        {
            return Err(Error::InvalidDocument);
        }
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProcessObservation {
    process: ProcessIdentity,
    observed_at_ms: u64,
    sessions: Vec<ApplicationEntry>,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Observations {
    processes: Vec<ProcessObservation>,
}
#[derive(Debug, Serialize)]
pub struct ApplicationStatus {
    pub scope: &'static str,
    pub lease_ms: u64,
    pub sessions: Vec<ApplicationEntry>,
    pub current_store_id: String,
    pub current_revision: String,
    pub current_values_digest: String,
}
impl Store {
    fn observations(&self) -> Result<Observations> {
        let now = now_ms()?;
        let mut observations: Observations = match self.files.read(OBSERVATIONS, LIMIT, true)? {
            Some(raw) => {
                serde_json::from_value(strict_json(&raw)?).map_err(|_| Error::InvalidDocument)?
            }
            None => Observations::default(),
        };
        if observations.processes.len() > PROCESS_LIMIT {
            return Err(Error::InvalidDocument);
        }
        let mut identities = BTreeSet::new();
        for observation in &observations.processes {
            if !identities.insert(observation.process.pid)
                || observation.sessions.len() > SESSION_LIMIT
            {
                return Err(Error::InvalidDocument);
            }
            let mut instances = BTreeSet::new();
            for session in &observation.sessions {
                session.validate()?;
                if !instances.insert(&session.instance_id) {
                    return Err(Error::InvalidDocument);
                }
            }
        }
        observations.processes.retain(|record| {
            now.checked_sub(record.observed_at_ms)
                .is_some_and(|age| age <= LEASE_MS)
                && identity(record.process.pid, self.uid)
                    .is_ok_and(|actual| actual == record.process)
        });
        Ok(observations)
    }
    /// 只能由实际引擎适配器传入其存活 session；进程身份和时间由内核读取。
    pub fn publish_applications(&self, sessions: Vec<ApplicationEntry>) -> Result<()> {
        let document = self.load()?;
        if sessions.len() > SESSION_LIMIT {
            return Err(Error::InvalidRequest);
        }
        let mut instances = BTreeSet::new();
        for session in &sessions {
            session.validate()?;
            if !instances.insert(&session.instance_id)
                || session.store_id != document.header.store_id
            {
                return Err(Error::InvalidRequest);
            }
            // 不把旧 session 改写为新版本；真实旧版本保留，UI 必须显示仍待应用。
            if revision(&session.revision)? > revision(&document.header.revision)? {
                return Err(Error::InvalidRequest);
            }
            if session.revision == document.header.revision
                && session.values_digest != document.header.values_digest
            {
                return Err(Error::InvalidRequest);
            }
        }
        let mut observations = self.observations()?;
        let process = identity(std::process::id(), self.uid)?;
        observations
            .processes
            .retain(|record| record.process.pid != process.pid);
        if !sessions.is_empty() {
            if observations.processes.len() >= PROCESS_LIMIT {
                return Err(Error::Busy);
            }
            observations.processes.push(ProcessObservation {
                process,
                observed_at_ms: now_ms()?,
                sessions,
            });
        }
        let bytes =
            canonical(&serde_json::to_value(observations).map_err(|_| Error::InvalidDocument)?)?;
        if bytes.len() > LIMIT {
            return Err(Error::InvalidDocument);
        }
        maintenance::ensure_normal_start(&self.home, self.uid).map_err(|_| Error::Maintenance)?;
        self.files.replace_observation(OBSERVATIONS, &bytes)
    }
    pub fn application_status(&self) -> Result<ApplicationStatus> {
        let current = self.read()?;
        let sessions = self
            .observations()?
            .processes
            .into_iter()
            .flat_map(|v| v.sessions)
            .filter(|s| s.store_id == current.store_id)
            .collect();
        Ok(ApplicationStatus {
            scope: "observed_engine_sessions",
            lease_ms: LEASE_MS,
            sessions,
            current_store_id: current.store_id,
            current_revision: current.revision,
            current_values_digest: current.values_digest,
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_actual_process_identity_and_unexpired_session_evidence_survives() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().canonicalize().unwrap();
        let store = Store::open(&home.join("settings.json"), &home, unsafe {
            libc::geteuid()
        })
        .unwrap();
        let snapshot = store.read().unwrap();
        assert!(store.application_status().unwrap().sessions.is_empty());
        let mut entry = ApplicationEntry::new(&snapshot);
        entry.applied_fields.insert("schema_id".into());
        store.publish_applications(vec![entry.clone()]).unwrap();
        assert_eq!(store.application_status().unwrap().sessions.len(), 1);
        for fault in ["start", "expired", "future"] {
            let mut evidence = store.observations().unwrap();
            match fault {
                "start" => evidence.processes[0].process.start_microseconds += 1,
                "expired" => {
                    evidence.processes[0].observed_at_ms =
                        now_ms().unwrap().saturating_sub(LEASE_MS + 1)
                }
                _ => evidence.processes[0].observed_at_ms = now_ms().unwrap() + 10_000,
            }
            store
                .files
                .replace_observation(OBSERVATIONS, &serde_json::to_vec(&evidence).unwrap())
                .unwrap();
            assert!(store.application_status().unwrap().sessions.is_empty());
            store.publish_applications(vec![entry.clone()]).unwrap();
        }
        let mut future = entry.clone();
        future.revision = "1".into();
        assert!(store.publish_applications(vec![future]).is_err());
        let mut forged = entry.clone();
        forged.values_digest = "0".repeat(64);
        assert!(store.publish_applications(vec![forged]).is_err());
        store.publish_applications(vec![]).unwrap();
        assert!(store.application_status().unwrap().sessions.is_empty());
    }
}
