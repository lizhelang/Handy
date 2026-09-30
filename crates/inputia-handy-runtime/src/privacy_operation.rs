//! 真正遗忘的耐久跨域回执。恢复载荷经本地密钥 AEAD 保护，完成后清除。
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
type Result<T> = std::result::Result<T, String>;
pub const READER_LEASE_MS: u64 = 2_000;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PrivacyScope {
    ForgetTerm { term: String },
    ClearLearned {},
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PrivacyRequest {
    pub operation_id: String,
    pub scope: PrivacyScope,
    pub expected_epoch: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyState {
    Accepted,
    Processing,
    PartialFailure,
    Completed,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DomainReceipts {
    pub integration: bool,
    pub personalization: bool,
    pub readers: bool,
    pub legacy_memory: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrivacyOperation {
    pub operation_id: String,
    pub scope: String,
    pub expected_epoch: u64,
    pub epoch: u64,
    pub state: PrivacyState,
    pub domain_receipts: DomainReceipts,
    pub failure: Option<String>,
    pub coverage: crate::legacy_memory::MemoryCoverage,
}
impl PrivacyRequest {
    pub fn validate(&self) -> Result<()> {
        inputia_core::integration::events::Identifier::parse(self.operation_id.clone())
            .map_err(str::to_owned)?;
        if self.expected_epoch == 0 || self.expected_epoch >= i64::MAX as u64 {
            return Err("privacy_epoch_invalid".into());
        }
        if let PrivacyScope::ForgetTerm { term } = &self.scope {
            if term.trim().is_empty() || term.chars().count() > 128 || term.contains('\0') {
                return Err("privacy_term_invalid".into());
            }
        }
        Ok(())
    }
    pub fn digest(&self, key: &[u8]) -> Result<Vec<u8>> {
        self.validate()?;
        if key.len() != 32 {
            return Err("privacy_key_invalid".into());
        }
        let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key);
        let mut context = ring::hmac::Context::with_key(&key);
        context.update(b"inputia-privacy-operation-v1\0");
        context.update(&serde_json::to_vec(self).map_err(|_| "privacy_request_invalid")?);
        Ok(context.sign().as_ref().to_vec())
    }
    fn kind(&self) -> &'static str {
        match self.scope {
            PrivacyScope::ForgetTerm { .. } => "forget_term",
            PrivacyScope::ClearLearned {} => "clear_learned",
        }
    }
}
fn err(_: rusqlite::Error) -> String {
    "privacy_journal_unavailable".into()
}
pub(crate) fn initialize(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS privacy_operations(operation_id TEXT PRIMARY KEY,scope TEXT NOT NULL,expected_epoch INTEGER NOT NULL,payload BLOB,digest BLOB NOT NULL,integration_digest BLOB NOT NULL,epoch INTEGER NOT NULL,personalization_epoch INTEGER,failure TEXT,completed INTEGER NOT NULL DEFAULT 0 CHECK(completed IN(0,1)));
 CREATE TABLE IF NOT EXISTS privacy_readers(reader_id TEXT PRIMARY KEY,epoch INTEGER NOT NULL,settled INTEGER NOT NULL DEFAULT 0 CHECK(settled IN(0,1)));").map_err(err)?;
    let columns: Vec<String> = db
        .prepare("PRAGMA table_info(privacy_operations)")
        .map_err(err)?
        .query_map([], |r| r.get(1))
        .map_err(err)?
        .collect::<std::result::Result<_, _>>()
        .map_err(err)?;
    for (name, ty) in [
        ("required_legacy", "INTEGER NOT NULL DEFAULT 0"),
        ("coverage_version", "INTEGER NOT NULL DEFAULT 1"),
        ("legacy_domain_uuid", "TEXT"),
        ("legacy_epoch", "INTEGER"),
    ] {
        if !columns.iter().any(|v| v == name) {
            db.execute(
                &format!("ALTER TABLE privacy_operations ADD COLUMN {name} {ty}"),
                [],
            )
            .map_err(err)?;
        }
    }
    Ok(())
}
pub(crate) fn existing(
    db: &Connection,
    request: &PrivacyRequest,
    key: &[u8],
) -> Result<Option<PrivacyOperation>> {
    let digest: Option<Vec<u8>> = db
        .query_row(
            "SELECT digest FROM privacy_operations WHERE operation_id=?1",
            [&request.operation_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(err)?;
    match digest {
        Some(digest) if digest == request.digest(key)? => get(db, &request.operation_id).map(Some),
        Some(_) => Err("privacy_request_conflict".into()),
        None => Ok(None),
    }
}
fn cipher(key: &[u8]) -> Result<ring::aead::LessSafeKey> {
    Ok(ring::aead::LessSafeKey::new(
        ring::aead::UnboundKey::new(&ring::aead::AES_256_GCM, key)
            .map_err(|_| "privacy_key_invalid")?,
    ))
}
/// 必须位于全局epoch、贡献撤销和forget回执同一个IMMEDIATE事务中。
pub(crate) fn accept(
    db: &Connection,
    key: &[u8],
    request: &PrivacyRequest,
    epoch: u64,
    integration_digest: &[u8],
) -> Result<()> {
    if epoch != request.expected_epoch + 1 {
        return Err("privacy_epoch_invalid".into());
    }
    let mut nonce = [0; 12];
    getrandom::getrandom(&mut nonce).map_err(|_| "privacy_random_unavailable")?;
    let mut payload = serde_json::to_vec(request).map_err(|_| "privacy_request_invalid")?;
    cipher(key)?
        .seal_in_place_append_tag(
            ring::aead::Nonce::assume_unique_for_key(nonce),
            ring::aead::Aad::from(request.operation_id.as_bytes()),
            &mut payload,
        )
        .map_err(|_| "privacy_payload_invalid")?;
    let mut sealed = nonce.to_vec();
    sealed.extend(payload);
    db.execute("INSERT INTO privacy_operations(operation_id,scope,expected_epoch,payload,digest,integration_digest,epoch) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![request.operation_id,request.kind(),request.expected_epoch,sealed,request.digest(key)?,integration_digest,epoch]).map_err(err)?;
    db.execute("UPDATE privacy_operations SET required_legacy=COALESCE((SELECT CAST(value AS INTEGER) FROM integration_meta WHERE key='memory_required'),0),coverage_version=2,legacy_domain_uuid=(SELECT value FROM integration_meta WHERE key='memory_domain_uuid') WHERE operation_id=?1",[&request.operation_id]).map_err(err)?;
    Ok(())
}
pub(crate) fn request(db: &Connection, id: &str, key: &[u8]) -> Result<PrivacyRequest> {
    let (payload, digest): (Option<Vec<u8>>, Vec<u8>) = db
        .query_row(
            "SELECT payload,digest FROM privacy_operations WHERE operation_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(err)?;
    let payload = payload
        .filter(|p| p.len() > 12)
        .ok_or("privacy_payload_unavailable")?;
    let nonce: [u8; 12] = payload[..12]
        .try_into()
        .map_err(|_| "privacy_payload_invalid")?;
    let mut bytes = payload[12..].to_vec();
    let plain = cipher(key)?
        .open_in_place(
            ring::aead::Nonce::assume_unique_for_key(nonce),
            ring::aead::Aad::from(id.as_bytes()),
            &mut bytes,
        )
        .map_err(|_| "privacy_payload_invalid")?;
    let request: PrivacyRequest =
        serde_json::from_slice(plain).map_err(|_| "privacy_request_invalid")?;
    if request.operation_id != id || request.digest(key)? != digest {
        return Err("privacy_request_conflict".into());
    }
    Ok(request)
}
pub(crate) fn pending(db: &Connection) -> Result<Vec<String>> {
    ids(
        db,
        "SELECT operation_id FROM privacy_operations WHERE completed=0 ORDER BY rowid LIMIT 32",
        [],
    )
}
fn ids<const N: usize>(db: &Connection, sql: &str, args: [&str; N]) -> Result<Vec<String>> {
    db.prepare(sql)
        .map_err(err)?
        .query_map(rusqlite::params_from_iter(args), |r| r.get(0))
        .map_err(err)?
        .collect::<std::result::Result<_, _>>()
        .map_err(err)
}
pub(crate) fn audit_page(db: &Connection, cursor: &str) -> Result<Vec<String>> {
    ids(db,"SELECT operation_id FROM privacy_operations WHERE operation_id>?1 ORDER BY operation_id LIMIT 32",[cursor])
}
pub(crate) fn has_pending(db: &Connection) -> Result<bool> {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM privacy_operations WHERE completed=0)",
        [],
        |r| r.get(0),
    )
    .map_err(err)
}
pub(crate) fn get(db: &Connection, id: &str) -> Result<PrivacyOperation> {
    let (scope,expected,epoch,personal,failure,completed):(String,u64,u64,Option<u64>,Option<String>,bool)=db.query_row("SELECT scope,expected_epoch,epoch,personalization_epoch,failure,completed FROM privacy_operations WHERE operation_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).map_err(err)?;
    if !matches!(scope.as_str(), "forget_term" | "clear_learned") || epoch != expected + 1 {
        return Err("privacy_journal_invalid".into());
    }
    let readers = !db
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM privacy_readers WHERE epoch<?1 AND settled=0)",
            [epoch],
            |r| r.get::<_, bool>(0),
        )
        .map_err(err)?;
    let integration = failure.as_deref() != Some("privacy_integration_unverified");
    let (required, legacy_epoch, version) = legacy_evidence(db, id)?;
    let personalization = personal.is_some() && failure.as_deref().is_none_or(legacy_failure);
    let legacy = required
        && legacy_epoch == Some(epoch)
        && version == 2
        && !failure.as_deref().is_some_and(legacy_failure);
    let coverage = if !required {
        crate::legacy_memory::MemoryCoverage::PrimaryOnly
    } else if version < 2 {
        crate::legacy_memory::MemoryCoverage::LegacyCoverageUnresolved
    } else {
        crate::legacy_memory::MemoryCoverage::AllDomains
    };
    Ok(PrivacyOperation {
        operation_id: id.into(),
        scope,
        expected_epoch: expected,
        epoch,
        state: if failure.is_some() {
            PrivacyState::PartialFailure
        } else if completed && personalization && readers && (!required || legacy) {
            PrivacyState::Completed
        } else if personal.is_some() {
            PrivacyState::Processing
        } else {
            PrivacyState::Accepted
        },
        domain_receipts: DomainReceipts {
            integration,
            personalization,
            readers,
            legacy_memory: legacy,
        },
        failure,
        coverage,
    })
}
pub(crate) fn list(db: &Connection) -> Result<Vec<PrivacyOperation>> {
    ids(
        db,
        "SELECT operation_id FROM privacy_operations ORDER BY completed ASC,rowid DESC LIMIT 32",
        [],
    )?
    .into_iter()
    .map(|id| get(db, &id))
    .collect()
}
pub(crate) fn evidence(db: &Connection, id: &str) -> Result<(Vec<u8>, Option<u64>, bool)> {
    db.query_row("SELECT digest,personalization_epoch,completed FROM privacy_operations WHERE operation_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(err)
}
pub(crate) fn verify_integration(db: &Connection, id: &str) -> Result<()> {
    let valid:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM privacy_operations p JOIN learning_forget_receipts r ON p.operation_id=r.operation_id WHERE p.operation_id=?1 AND p.epoch=p.expected_epoch+1 AND p.epoch=r.result_epoch AND p.integration_digest=r.digest AND p.epoch<=CAST((SELECT value FROM integration_meta WHERE key='policy_epoch') AS INTEGER))",[id],|r|r.get(0)).map_err(err)?;
    if valid {
        Ok(())
    } else {
        Err("privacy_integration_unverified".into())
    }
}
pub(crate) fn personal_receipt(db: &Connection, id: &str, epoch: u64) -> Result<()> {
    db.execute(
        "UPDATE privacy_operations SET personalization_epoch=?2,failure=NULL WHERE operation_id=?1",
        params![id, epoch],
    )
    .map_err(err)?;
    Ok(())
}
pub(crate) fn fail(db: &Connection, id: &str, reason: &str) -> Result<()> {
    db.execute(
        "UPDATE privacy_operations SET failure=?2,completed=0 WHERE operation_id=?1",
        params![id, reason],
    )
    .map_err(err)?;
    Ok(())
}
pub(crate) fn settle(db: &Connection, id: &str) -> Result<()> {
    db.execute("UPDATE privacy_operations SET completed=1,failure=NULL,payload=NULL WHERE operation_id=?1 AND personalization_epoch IS NOT NULL AND (required_legacy=0 OR (coverage_version=2 AND legacy_epoch=epoch AND legacy_domain_uuid IS NOT NULL)) AND failure IS NULL AND NOT EXISTS(SELECT 1 FROM privacy_readers WHERE epoch<privacy_operations.epoch AND settled=0)",[id]).map_err(err)?;
    Ok(())
}
pub(crate) fn reader_ids(db: &Connection) -> Result<Vec<String>> {
    ids(
        db,
        "SELECT reader_id FROM privacy_readers WHERE settled=0",
        [],
    )
}
pub(crate) fn issue_reader(db: &Connection, id: &str, epoch: u64) -> Result<()> {
    db.execute("INSERT INTO privacy_readers(reader_id,epoch,settled) VALUES(?1,?2,0) ON CONFLICT(reader_id) DO UPDATE SET epoch=excluded.epoch,settled=0",params![id,epoch]).map_err(err)?;
    Ok(())
}
pub(crate) fn settle_reader(db: &Connection, id: &str, epoch: Option<u64>) -> Result<bool> {
    Ok(db.execute("UPDATE privacy_readers SET settled=1 WHERE reader_id=?1 AND settled=0 AND (?2 IS NULL OR epoch<?2)",params![id,epoch]).map_err(err)?>0)
}
pub(crate) fn audit_gate(db: &Connection, pending: bool) -> Result<()> {
    db.execute("INSERT INTO integration_meta(key,value) VALUES('privacy_audit_pending',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",[if pending{"1"}else{"0"}]).map_err(err)?;
    Ok(())
}
pub fn ensure_readable(root: &std::path::Path) -> Result<()> {
    let path = root.join("integration.db");
    if !path.exists() {
        return Ok(());
    }
    let db = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(err)?;
    let installed:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='privacy_operations' AND type='table')",[],|r|r.get(0)).map_err(err)?;
    if installed {
        let audit:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM integration_meta WHERE key='privacy_audit_pending' AND value='1')",[],|r|r.get(0)).map_err(err)?;
        if audit || has_primary_pending(&db)? {
            return Err("privacy_operation_pending".into());
        }
    }
    Ok(())
}
fn legacy_failure(reason: &str) -> bool {
    reason.starts_with("memory_") || reason == "legacy_coverage_unresolved"
}
pub(crate) fn has_primary_pending(db: &Connection) -> Result<bool> {
    let mut q=db.prepare("SELECT operation_id,personalization_epoch,failure,epoch FROM privacy_operations WHERE completed=0").map_err(err)?;
    let rows = q
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<u64>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, u64>(3)?,
            ))
        })
        .map_err(err)?;
    for row in rows {
        let (_id, personal, failure, epoch) = row.map_err(err)?;
        if personal.is_none()
            || failure
                .as_deref()
                .is_some_and(|reason| !legacy_failure(reason))
        {
            return Ok(true);
        }
        let readers: bool = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM privacy_readers WHERE epoch<?1 AND settled=0)",
                [epoch],
                |r| r.get(0),
            )
            .map_err(err)?;
        if readers {
            return Ok(true);
        }
    }
    Ok(false)
}
pub(crate) fn configure_legacy(
    db: &Connection,
    required: bool,
    domain: Option<&str>,
) -> Result<()> {
    if !required {
        return Ok(());
    }
    db.execute("INSERT INTO integration_meta VALUES('memory_required','1') ON CONFLICT(key) DO UPDATE SET value='1'",[]).map_err(err)?;
    if let Some(domain) = domain {
        let previous: Option<String> = db
            .query_row(
                "SELECT value FROM integration_meta WHERE key='memory_domain_uuid'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        if previous.as_deref().is_some_and(|old| old != domain) {
            return Err("memory_domain_conflict".into());
        }
        db.execute(
            "INSERT OR IGNORE INTO integration_meta VALUES('memory_domain_uuid',?1)",
            [domain],
        )
        .map_err(err)?;
    }
    db.execute("UPDATE privacy_operations SET required_legacy=1,coverage_version=CASE WHEN payload IS NULL THEN 1 ELSE 2 END,completed=0,failure=CASE WHEN payload IS NULL THEN 'legacy_coverage_unresolved' ELSE failure END WHERE required_legacy=0",[]).map_err(err)?;
    Ok(())
}
pub(crate) fn legacy_evidence(db: &Connection, id: &str) -> Result<(bool, Option<u64>, u32)> {
    db.query_row("SELECT required_legacy,legacy_epoch,coverage_version FROM privacy_operations WHERE operation_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(err)
}
pub(crate) fn legacy_domain(db: &Connection, id: &str) -> Result<Option<String>> {
    db.query_row(
        "SELECT legacy_domain_uuid FROM privacy_operations WHERE operation_id=?1",
        [id],
        |r| r.get(0),
    )
    .map_err(err)
}
pub(crate) fn bind_legacy(db: &Connection, id: &str, domain: &str) -> Result<()> {
    if legacy_domain(db, id)?
        .as_deref()
        .is_some_and(|old| old != domain)
    {
        return Err("memory_domain_conflict".into());
    }
    db.execute("UPDATE privacy_operations SET legacy_domain_uuid=?2 WHERE operation_id=?1 AND payload IS NOT NULL AND coverage_version=2",params![id,domain]).map_err(err)?;
    Ok(())
}
pub(crate) fn legacy_receipt(db: &Connection, id: &str, epoch: u64) -> Result<()> {
    db.execute("UPDATE privacy_operations SET legacy_epoch=?2,failure=NULL WHERE operation_id=?1 AND epoch=?2 AND legacy_domain_uuid IS NOT NULL AND coverage_version=2",params![id,epoch]).map_err(err)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{personalization, store::IntegrationStore};
    fn request(id: &str) -> PrivacyRequest {
        PrivacyRequest {
            operation_id: id.into(),
            scope: PrivacyScope::ForgetTerm {
                term: "私有词".into(),
            },
            expected_epoch: 1,
        }
    }
    #[test]
    fn clear_scope_rejects_extra_fields_instead_of_widening_a_term_request() {
        let valid = serde_json::json!({"operation_id":"strict-clear","expected_epoch":1,"scope":{"kind":"clear_learned"}});
        let request: PrivacyRequest = serde_json::from_value(valid.clone()).unwrap();
        assert_eq!(request.scope, PrivacyScope::ClearLearned {});
        let mut malformed = valid.clone();
        malformed["scope"]["term"] = serde_json::json!("private term");
        assert!(serde_json::from_value::<PrivacyRequest>(malformed).is_err());
        let mut unknown = valid;
        unknown["scope"]["unexpected"] = serde_json::json!(true);
        assert!(serde_json::from_value::<PrivacyRequest>(unknown).is_err());
    }
    #[test]
    fn request_receipts_require_private_key_for_dictionary_checks() {
        use sha2::{Digest, Sha256};
        let req = request("opaque-receipt");
        assert_ne!(req.digest(&[1; 32]).unwrap(), req.digest(&[2; 32]).unwrap());
        assert_ne!(
            req.digest(&[1; 32]).unwrap(),
            Sha256::digest(serde_json::to_vec(&req).unwrap()).to_vec()
        );
    }
    #[test]
    fn acceptance_is_one_epoch_and_receipt_replay_is_exact() {
        let root = tempfile::tempdir().unwrap();
        let key = [7; 32];
        let mut store =
            IntegrationStore::open(root.path().join("integration.db"), "privacy").unwrap();
        store.enable_learning(&key).unwrap();
        let req = request("privacy-one");
        let result = store.begin_privacy(&key, &req).unwrap();
        assert_eq!(result.epoch, 2);
        assert_eq!(store.policy_epoch().unwrap(), 2);
        assert_eq!(result.state, PrivacyState::Accepted);
        assert_eq!(store.begin_privacy(&key, &req).unwrap(), result);
        let mut conflict = req.clone();
        conflict.scope = PrivacyScope::ClearLearned {};
        assert!(store.begin_privacy(&key, &conflict).is_err());
        assert!(store.begin_privacy(&key, &request("privacy-two")).is_err());
        assert!(ensure_readable(root.path()).is_err());
        let epoch =
            personalization::apply_privacy(root.path(), &req, &req.digest(&key).unwrap()).unwrap();
        assert_eq!(
            personalization::apply_privacy(root.path(), &req, &req.digest(&key).unwrap()).unwrap(),
            epoch
        );
        personal_receipt(store.attachment_connection(), &req.operation_id, epoch).unwrap();
        settle(store.attachment_connection(), &req.operation_id).unwrap();
        assert_eq!(
            get(store.attachment_connection(), &req.operation_id)
                .unwrap()
                .state,
            PrivacyState::Completed
        );
        assert!(ensure_readable(root.path()).is_ok());
        assert_eq!(store.policy_epoch().unwrap(), 2);
    }
    #[test]
    fn domain_commit_before_journal_and_old_readers_do_not_fake_completion() {
        let root = tempfile::tempdir().unwrap();
        let key = [8; 32];
        let mut store =
            IntegrationStore::open(root.path().join("integration.db"), "privacy").unwrap();
        store.enable_learning(&key).unwrap();
        issue_reader(store.attachment_connection(), "host", 1).unwrap();
        let req = request("crash-after-domain");
        store.begin_privacy(&key, &req).unwrap();
        let personal_epoch =
            personalization::apply_privacy(root.path(), &req, &req.digest(&key).unwrap()).unwrap();
        drop(store);
        let mut store =
            IntegrationStore::open(root.path().join("integration.db"), "privacy").unwrap();
        store.enable_learning(&key).unwrap();
        assert_eq!(
            personalization::apply_privacy(root.path(), &req, &req.digest(&key).unwrap()).unwrap(),
            personal_epoch
        );
        personal_receipt(
            store.attachment_connection(),
            &req.operation_id,
            personal_epoch,
        )
        .unwrap();
        settle(store.attachment_connection(), &req.operation_id).unwrap();
        assert_eq!(
            get(store.attachment_connection(), &req.operation_id)
                .unwrap()
                .state,
            PrivacyState::Processing
        );
        settle_reader(store.attachment_connection(), "host", Some(0)).unwrap();
        settle(store.attachment_connection(), &req.operation_id).unwrap();
        assert_eq!(
            get(store.attachment_connection(), &req.operation_id)
                .unwrap()
                .state,
            PrivacyState::Processing
        );
        settle_reader(store.attachment_connection(), "host", Some(2)).unwrap();
        settle(store.attachment_connection(), &req.operation_id).unwrap();
        assert_eq!(
            get(store.attachment_connection(), &req.operation_id)
                .unwrap()
                .state,
            PrivacyState::Completed
        );
    }
    #[test]
    fn clear_retains_event_receipts_and_failure_progress() {
        let root = tempfile::tempdir().unwrap();
        let key = [9; 32];
        let mut store =
            IntegrationStore::open(root.path().join("integration.db"), "privacy").unwrap();
        store.enable_learning(&key).unwrap();
        personalization::manage(
            root.path(),
            "personalization_status",
            &serde_json::json!({}),
        )
        .unwrap();
        let db = Connection::open(root.path().join("knowledge/personalization.sqlite")).unwrap();
        db.execute("INSERT INTO evidence(event_id,fingerprint,text,normalized,previous,code,weight,created) VALUES('old','hash','私有词','私有词','','',1,0)",[]).unwrap();
        let req = PrivacyRequest {
            scope: PrivacyScope::ClearLearned {},
            ..request("clear")
        };
        store.begin_privacy(&key, &req).unwrap();
        fail(
            store.attachment_connection(),
            &req.operation_id,
            "personalization_unavailable",
        )
        .unwrap();
        assert_eq!(
            get(store.attachment_connection(), &req.operation_id)
                .unwrap()
                .state,
            PrivacyState::PartialFailure
        );
        let epoch =
            personalization::apply_privacy(root.path(), &req, &req.digest(&key).unwrap()).unwrap();
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM evidence", [], |r| r.get::<_, u32>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            db.query_row(
                "SELECT COUNT(*) FROM retired_events WHERE event_id='old'",
                [],
                |r| r.get::<_, u32>(0)
            )
            .unwrap(),
            1
        );
        personal_receipt(store.attachment_connection(), &req.operation_id, epoch).unwrap();
        settle(store.attachment_connection(), &req.operation_id).unwrap();
        let json =
            serde_json::to_string(&get(store.attachment_connection(), &req.operation_id).unwrap())
                .unwrap();
        assert!(!json.contains("私有词"));
        assert!(personalization::manage(
            root.path(),
            "personalization_clear",
            &serde_json::json!({})
        )
        .is_err());
    }
}
