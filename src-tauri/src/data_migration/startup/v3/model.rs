//! schema 3 记录真实写入顺序；不将各文件的历史摘要做笛卡尔积。
use super::*;
use inputia_settings::store::{
    InitializationIntent, LedgerActivationIntent, TransitionIntent, TransitionPhase,
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) const GENERATION: &str = "control-settings-pending-v3";
pub(super) const SETTINGS_LIMIT: u64 = 512 * 1024;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartupPurpose {
    FullStartup,
    SettingsProtocol,
    SettingsReplay,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileStamp {
    pub sha256: String,
    pub size: u64,
}
impl FileStamp {
    pub(super) fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            hash_valid(&self.sha256) && self.size <= SETTINGS_LIMIT,
            "invalid settings digest"
        );
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Member {
    pub root_label: String,
    pub domain: String,
    pub name: String,
    pub original: Original,
}
impl Member {
    pub(super) fn digest(&self) -> Option<FileStamp> {
        digest(&self.original)
    }
}
pub(super) fn digest(original: &Original) -> Option<FileStamp> {
    match original {
        Original::Absent => None,
        Original::Present { sha256, size, .. } => Some(FileStamp {
            sha256: sha256.clone(),
            size: *size,
        }),
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Binding {
    pub domain: String,
    pub store_id: String,
    pub ledger_id: Option<String>,
    pub active_request: Option<RequestBinding>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RequestBinding {
    pub operation_id: String,
    pub request_digest: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Change {
    pub member: usize,
    pub before: Option<FileStamp>,
    pub after: FileStamp,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Owner {
    Initialize {
        domain: String,
        store_id: String,
    },
    Activate {
        domain: String,
        store_id: String,
        ledger_id: String,
        activation_id: String,
    },
    Transition {
        domain: String,
        store_id: String,
        ledger_id: String,
        operation_id: String,
        request_digest: String,
        phase: StepPhase,
    },
}
impl Owner {
    fn binding(&self) -> Binding {
        match self {
            Self::Initialize { domain, store_id } => Binding {
                domain: domain.clone(),
                store_id: store_id.clone(),
                ledger_id: None,
                active_request: None,
            },
            Self::Activate {
                domain,
                store_id,
                ledger_id,
                ..
            }
            | Self::Transition {
                domain,
                store_id,
                ledger_id,
                ..
            } => Binding {
                domain: domain.clone(),
                store_id: store_id.clone(),
                ledger_id: Some(ledger_id.clone()),
                active_request: None,
            },
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum StepPhase {
    PrepareRequest,
    CommitDocument,
    ResolveRequest,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Authorization {
    pub sequence: u64,
    pub owner: Owner,
    pub changes: Vec<Change>,
    /// 仅 Active 日志的同内容补同步，绑定既有整组前缀，不推进逻辑写入链。
    pub reaffirm_prefix: Option<usize>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Evidence {
    pub relative: PathBuf,
    pub sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Recovery {
    pub prefix: usize,
    pub cursor: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct JournalV3 {
    pub schema_version: u32,
    pub migration_id: String,
    pub attempt_id: String,
    pub roots_sha256: String,
    pub purpose: StartupPurpose,
    pub manifest_relative: PathBuf,
    pub manifest_sha256: String,
    pub phase: Phase,
    pub members: Vec<Member>,
    pub bindings: Vec<Binding>,
    pub authorizations: Vec<Authorization>,
    pub recovery: Option<Recovery>,
    pub completed: Option<Vec<Option<FileStamp>>>,
    pub full_completion: Option<Evidence>,
}
fn uuid(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| id.to_string() == value)
}
pub(super) fn index(members: &[Member], domain: &str, name: &str) -> Result<usize> {
    members
        .iter()
        .position(|m| m.domain == domain && m.name == name)
        .context("settings member not authorized")
}
pub(super) fn prefixes(journal: &JournalV3) -> Result<Vec<Vec<Option<FileStamp>>>> {
    let mut current = journal
        .members
        .iter()
        .map(Member::digest)
        .collect::<Vec<_>>();
    for value in current.iter().flatten() {
        value.validate()?;
    }
    let mut states = vec![current.clone()];
    let mut bindings = BTreeMap::new();
    for binding in &journal.bindings {
        anyhow::ensure!(
            uuid(&binding.store_id)
                && binding.ledger_id.as_ref().is_none_or(|id| uuid(id))
                && bindings
                    .insert(binding.domain.clone(), binding.clone())
                    .is_none(),
            "invalid baseline protocol binding"
        );
    }
    let mut initialized = BTreeSet::new();
    let mut activated = BTreeSet::new();
    let mut requests: BTreeMap<String, (String, StepPhase)> = BTreeMap::new();
    let mut active_ledger = BTreeMap::new();
    let mut latest_request = None;
    for binding in &journal.bindings {
        anyhow::ensure!(
            journal.members.iter().any(|m| m.domain == binding.domain),
            "baseline domain is not in scope"
        );
        if let Some(request) = &binding.active_request {
            anyhow::ensure!(
                binding.ledger_id.is_some()
                    && hash_valid(&request.request_digest)
                    && !request.operation_id.is_empty()
                    && request.operation_id.len() <= 256
                    && !request.operation_id.chars().any(char::is_control),
                "invalid baseline active request"
            );
            let pending = index(
                &journal.members,
                &binding.domain,
                CONTROL_SETTINGS_PENDING_NAME,
            )?;
            active_ledger.insert(request.operation_id.clone(), current[pending].clone());
            latest_request = Some(request.operation_id.clone());
            anyhow::ensure!(
                requests
                    .insert(
                        request.operation_id.clone(),
                        (request.request_digest.clone(), StepPhase::PrepareRequest)
                    )
                    .is_none(),
                "duplicate baseline active request"
            );
        }
    }
    for (offset, authorization) in journal.authorizations.iter().enumerate() {
        anyhow::ensure!(
            authorization.sequence == offset as u64 + 1 && !authorization.changes.is_empty(),
            "invalid authorization sequence"
        );
        anyhow::ensure!(
            journal.purpose != StartupPurpose::SettingsReplay
                || matches!(authorization.owner, Owner::Transition { .. }),
            "replay cannot initialize or activate a domain"
        );
        anyhow::ensure!(
            journal.purpose != StartupPurpose::SettingsProtocol
                || !matches!(authorization.owner, Owner::Transition { .. }),
            "activation cannot replay a request"
        );
        let binding = authorization.owner.binding();
        anyhow::ensure!(
            uuid(&binding.store_id) && binding.ledger_id.as_ref().is_none_or(|id| uuid(id)),
            "invalid authorization identity"
        );
        let document = index(
            &journal.members,
            &binding.domain,
            if binding.domain == "inputia.control-settings" {
                "settings_store.json"
            } else {
                "settings.json"
            },
        )?;
        let marker_name = if binding.domain == "inputia.control-settings" {
            ".inputia-control-settings-initialized.json"
        } else {
            ".inputia-settings-initialized.json"
        };
        let marker = index(&journal.members, &binding.domain, marker_name)?;
        if let Some(previous) = bindings.get(&binding.domain) {
            anyhow::ensure!(
                previous.store_id == binding.store_id
                    && (previous.ledger_id == binding.ledger_id
                        || matches!(authorization.owner, Owner::Activate { .. })
                            && previous.ledger_id.is_none()),
                "protocol identity changed inside attempt"
            );
        }
        match &authorization.owner {
            Owner::Initialize { .. } => {
                anyhow::ensure!(
                    initialized.insert(binding.domain.clone())
                        && current[marker].is_none()
                        && bindings
                            .get(&binding.domain)
                            .is_none_or(|previous| previous.store_id == binding.store_id
                                && previous.ledger_id.is_none()
                                && authorization.changes.len() == 1
                                && current[document].is_some()),
                    "initialization already owned"
                );
                let expected = if authorization.changes.len() == 1 {
                    vec![marker]
                } else {
                    vec![document, marker]
                };
                anyhow::ensure!(
                    authorization
                        .changes
                        .iter()
                        .map(|c| c.member)
                        .collect::<Vec<_>>()
                        == expected,
                    "initialization write order mismatch"
                );
            }
            Owner::Activate { activation_id, .. } => {
                let pending = index(
                    &journal.members,
                    &binding.domain,
                    CONTROL_SETTINGS_PENDING_NAME,
                )?;
                anyhow::ensure!(
                    uuid(activation_id)
                        && activated.insert(binding.domain.clone())
                        && current[pending].is_none()
                        && current[document].is_some()
                        && current[marker].is_some(),
                    "activation baseline invalid"
                );
                anyhow::ensure!(
                    authorization
                        .changes
                        .iter()
                        .map(|c| c.member)
                        .collect::<Vec<_>>()
                        == vec![pending, document, marker],
                    "activation write order mismatch"
                );
            }
            Owner::Transition {
                operation_id,
                request_digest,
                phase,
                ..
            } => {
                anyhow::ensure!(
                    binding.domain == "inputia.control-settings"
                        && operation_id.len() <= 256
                        && !operation_id.is_empty()
                        && !operation_id.chars().any(char::is_control)
                        && hash_valid(request_digest)
                        && bindings.get(&binding.domain).is_some_and(|b| b.ledger_id
                            == binding.ledger_id
                            && b.ledger_id.is_some()),
                    "transition identity mismatch"
                );
                let pending = index(
                    &journal.members,
                    &binding.domain,
                    CONTROL_SETTINGS_PENDING_NAME,
                )?;
                let target = if *phase == StepPhase::CommitDocument {
                    document
                } else {
                    pending
                };
                anyhow::ensure!(
                    authorization.changes.len() == 1
                        && authorization.changes[0].member == target
                        && current
                            .iter()
                            .enumerate()
                            .filter(|(i, _)| [*i == document, *i == marker, *i == pending]
                                .into_iter()
                                .any(|v| v))
                            .all(|(_, v)| v.is_some()),
                    "transition target mismatch"
                );
                if let Some(prefix) = authorization.reaffirm_prefix {
                    let change = &authorization.changes[0];
                    anyhow::ensure!(
                        *phase == StepPhase::PrepareRequest
                            && latest_request.as_ref() == Some(operation_id)
                            && requests
                                .get(operation_id)
                                .is_some_and(|(d, _)| d == request_digest)
                            && change.before.as_ref() == Some(&change.after)
                            && active_ledger.get(operation_id) == Some(&change.before)
                            && states
                                .get(prefix)
                                .is_some_and(|state| state[pending] == change.before),
                        "invalid active-ledger durability reaffirmation"
                    );
                    continue;
                }
                if let Some((digest, previous)) = requests.get(operation_id) {
                    anyhow::ensure!(
                        digest == request_digest
                            && matches!(
                                (previous, phase),
                                (StepPhase::PrepareRequest, StepPhase::CommitDocument)
                                    | (StepPhase::PrepareRequest, StepPhase::ResolveRequest)
                                    | (StepPhase::CommitDocument, StepPhase::ResolveRequest)
                                    | (StepPhase::ResolveRequest, StepPhase::PrepareRequest)
                            ),
                        "request phase fork"
                    );
                } else {
                    // Replay 可从已有 Active 的提交/收口开始；核心 observer 仍核真实原请求。
                    anyhow::ensure!(
                        *phase == StepPhase::PrepareRequest,
                        "request missing exact baseline or prepare authorization"
                    );
                }
                anyhow::ensure!(
                    !requests.iter().any(|(id, (_, phase))| id != operation_id
                        && *phase != StepPhase::ResolveRequest),
                    "another request is unresolved"
                );
                if *phase == StepPhase::PrepareRequest {
                    active_ledger.insert(
                        operation_id.clone(),
                        Some(authorization.changes[0].after.clone()),
                    );
                }
                latest_request = Some(operation_id.clone());
                requests.insert(operation_id.clone(), (request_digest.clone(), *phase));
            }
        }
        anyhow::ensure!(
            authorization.reaffirm_prefix.is_none(),
            "only active-ledger reaffirmation may name a prefix"
        );
        bindings.insert(binding.domain.clone(), binding);
        let mut changed = BTreeSet::new();
        for change in &authorization.changes {
            let member = journal
                .members
                .get(change.member)
                .context("invalid change member")?;
            anyhow::ensure!(
                changed.insert(change.member)
                    && member.domain == authorization.owner.binding().domain
                    && current[change.member] == change.before,
                "authorization does not extend exact frontier"
            );
            change.after.validate()?;
            current[change.member] = Some(change.after.clone());
            states.push(current.clone());
        }
    }
    Ok(states)
}

pub(super) fn initialization(
    j: &JournalV3,
    intent: &InitializationIntent,
) -> Result<Authorization> {
    let document = index(&j.members, &intent.domain, &intent.file_name)?;
    let marker = index(&j.members, &intent.domain, &intent.marker_name)?;
    let current = prefixes(j)?.pop().context("missing baseline")?;
    anyhow::ensure!(
        intent.will_create_marker
            && current[marker].is_none()
            && current[document].as_ref().map(|d| d.sha256.as_str())
                == intent.original_document_sha256.as_deref()
            && current[document].as_ref().map(|d| d.size) == intent.original_document_size,
        "initialization baseline mismatch"
    );
    let mut changes = vec![];
    if intent.will_write_document {
        changes.push(Change {
            member: document,
            before: current[document].clone(),
            after: FileStamp {
                sha256: intent.document_sha256.clone(),
                size: intent.document_size,
            },
        });
    } else {
        anyhow::ensure!(
            current[document].as_ref().is_some_and(
                |d| d.sha256 == intent.document_sha256 && d.size == intent.document_size
            ),
            "unchanged initialization document mismatch"
        );
    }
    changes.push(Change {
        member: marker,
        before: None,
        after: FileStamp {
            sha256: intent.marker_sha256.clone(),
            size: intent.marker_size,
        },
    });
    Ok(Authorization {
        sequence: j.authorizations.len() as u64 + 1,
        owner: Owner::Initialize {
            domain: intent.domain.clone(),
            store_id: intent.store_id.clone(),
        },
        changes,
        reaffirm_prefix: None,
    })
}
pub(super) fn activation(j: &JournalV3, intent: &LedgerActivationIntent) -> Result<Authorization> {
    let changes =
        intent
            .files
            .iter()
            .map(|f| {
                anyhow::ensure!(
                    f.original_sha256.is_some() == f.original_size.is_some(),
                    "activation original size mismatch"
                );
                Ok(Change {
                    member: index(&j.members, &intent.domain, &f.name)?,
                    before: f.original_sha256.as_ref().zip(f.original_size).map(
                        |(sha256, size)| FileStamp {
                            sha256: sha256.clone(),
                            size,
                        },
                    ),
                    after: FileStamp {
                        sha256: f.target_sha256.clone(),
                        size: f.target_size,
                    },
                })
            })
            .collect::<Result<Vec<_>>>()?;
    Ok(Authorization {
        sequence: j.authorizations.len() as u64 + 1,
        owner: Owner::Activate {
            domain: intent.domain.clone(),
            store_id: intent.store_id.clone(),
            ledger_id: intent.ledger_id.clone(),
            activation_id: intent.activation_id.clone(),
        },
        changes,
        reaffirm_prefix: None,
    })
}
pub(super) fn transition(j: &JournalV3, intent: &TransitionIntent) -> Result<Authorization> {
    let phase = match intent.phase {
        TransitionPhase::PrepareRequest => StepPhase::PrepareRequest,
        TransitionPhase::CommitDocument => StepPhase::CommitDocument,
        TransitionPhase::ResolveRequest => StepPhase::ResolveRequest,
    };
    Ok(Authorization {
        sequence: j.authorizations.len() as u64 + 1,
        owner: Owner::Transition {
            domain: intent.domain.clone(),
            store_id: intent.store_id.clone(),
            ledger_id: intent.ledger_id.clone(),
            operation_id: intent.operation_id.clone(),
            request_digest: intent.request_digest.clone(),
            phase,
        },
        reaffirm_prefix: None,
        changes: vec![Change {
            member: index(&j.members, &intent.domain, &intent.file_name)?,
            before: Some(FileStamp {
                sha256: intent.before.sha256.clone(),
                size: intent.before.size,
            }),
            after: FileStamp {
                sha256: intent.after.sha256.clone(),
                size: intent.after.size,
            },
        }],
    })
}
