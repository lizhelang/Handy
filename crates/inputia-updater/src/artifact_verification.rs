//! 将已授权发布身份与事务的准确盘面位置合并；本层不执行代码验签或配对验签。
use crate::{
    native_code::{CodeExpectation, CodePurpose, CodeRole},
    Entry, Error, Fingerprint, Result, Role, Subject, VerificationPurpose,
};
use inputia_release::native_policy::{NativeComponentRole, NativeReleasePolicy};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Clone, Debug)]
struct ComponentPolicySnapshot {
    role: NativeComponentRole,
    bundle_id: String,
    team_id: String,
    slices: Vec<(String, String)>,
}
#[derive(Clone, Debug)]
struct PairPolicySnapshot {
    sha256: String,
    signer_key_id: String,
    schema: u64,
}
#[derive(Clone, Debug)]
struct ReleasePolicySnapshot {
    product_id: String,
    release_id: String,
    version: String,
    build: u64,
    source_commit: String,
    components: Vec<ComponentPolicySnapshot>,
    pair: PairPolicySnapshot,
}
impl From<&NativeReleasePolicy> for ReleasePolicySnapshot {
    fn from(policy: &NativeReleasePolicy) -> Self {
        Self {
            product_id: policy.product_id().into(),
            release_id: policy.release_id().into(),
            version: policy.version().into(),
            build: policy.build(),
            source_commit: policy.source_commit().into(),
            components: policy
                .components()
                .iter()
                .map(|component| ComponentPolicySnapshot {
                    role: component.role(),
                    bundle_id: component.bundle_id().into(),
                    team_id: component.team_id().into(),
                    slices: component
                        .slices()
                        .iter()
                        .map(|slice| (slice.architecture().into(), slice.cdhash().into()))
                        .collect(),
                })
                .collect(),
            pair: PairPolicySnapshot {
                sha256: policy.pair_manifest().sha256().into(),
                signer_key_id: policy.pair_manifest().signer_key_id().into(),
                schema: policy.pair_manifest().schema(),
            },
        }
    }
}

#[derive(Clone, Debug)]
pub struct CodeVerificationTarget {
    expectation: CodeExpectation,
    tree: Fingerprint,
}
impl CodeVerificationTarget {
    pub fn expectation(&self) -> &CodeExpectation {
        &self.expectation
    }
    pub fn tree(&self) -> &Fingerprint {
        &self.tree
    }
}

#[derive(Clone, Debug)]
pub struct PairVerificationTarget {
    exact_path: PathBuf,
    sha256: String,
    signer_key_id: String,
    schema: u64,
    tree: Fingerprint,
}
impl PairVerificationTarget {
    pub fn exact_path(&self) -> &std::path::Path {
        &self.exact_path
    }
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
    pub fn signer_key_id(&self) -> &str {
        &self.signer_key_id
    }
    pub fn schema(&self) -> u64 {
        self.schema
    }
    pub fn tree(&self) -> &Fingerprint {
        &self.tree
    }
}

/// 只能由已授权 `NativeReleasePolicy` 与当前事务 Entry 生成；不等于验签成功回执。
#[derive(Clone, Debug)]
pub struct ArtifactVerificationPlan {
    subject: Subject,
    code: Vec<CodeVerificationTarget>,
    pair: PairVerificationTarget,
}
impl ArtifactVerificationPlan {
    pub fn new(
        subject: &Subject,
        policy: &NativeReleasePolicy,
        entries: &[Entry],
        purpose: VerificationPurpose,
    ) -> Result<Self> {
        Self::from_snapshot(
            subject,
            &ReleasePolicySnapshot::from(policy),
            entries,
            purpose,
        )
    }

    fn from_snapshot(
        subject: &Subject,
        policy: &ReleasePolicySnapshot,
        entries: &[Entry],
        purpose: VerificationPurpose,
    ) -> Result<Self> {
        let code_purpose = match &purpose {
            VerificationPurpose::DownloadedNew
            | VerificationPurpose::StagedNew
            | VerificationPurpose::InstalledNew => {
                if policy.release_id != subject.new_release_id {
                    return Err(Error::Adapter("new release policy mismatch"));
                }
                CodePurpose::NewRelease
            }
            VerificationPurpose::RollbackOld { release_id } => {
                if policy.release_id != *release_id || release_id == &subject.new_release_id {
                    return Err(Error::Adapter("rollback release policy mismatch"));
                }
                CodePurpose::PreviousRelease
            }
        };
        let mut locations = BTreeMap::new();
        for entry in entries {
            if !matches!(
                entry.role,
                Role::Control | Role::Ime | Role::Settings | Role::PairManifest
            ) || locations.insert(entry.role, entry).is_some()
            {
                return Err(Error::Adapter("artifact verification roles"));
            }
        }
        if locations.len() != 4 {
            return Err(Error::Adapter("artifact verification roles"));
        }
        let mut code = Vec::new();
        for component in policy.components.iter().filter(|component| {
            matches!(
                component.role,
                NativeComponentRole::Control
                    | NativeComponentRole::Ime
                    | NativeComponentRole::Settings
            )
        }) {
            let role = updater_role(component.role);
            let entry = locations
                .get(&role)
                .ok_or(Error::Adapter("missing component verification role"))?;
            let exact_bundle_path = selected_path(entry, &purpose);
            let expectation = code_expectation(
                subject,
                policy,
                component,
                code_purpose.clone(),
                exact_bundle_path,
            )?;
            code.push(CodeVerificationTarget {
                expectation,
                tree: entry.new.clone(),
            });
        }
        if code.len() != Role::COMPONENTS.len() {
            return Err(Error::Adapter("release component policy incomplete"));
        }
        let pair_entry = locations
            .get(&Role::PairManifest)
            .ok_or(Error::Adapter("pair manifest verification role"))?;
        let pair_policy = &policy.pair;
        if pair_policy.schema != 2 {
            return Err(Error::Adapter("pair manifest schema"));
        }
        Ok(Self {
            subject: subject.clone(),
            code,
            pair: PairVerificationTarget {
                exact_path: selected_path(pair_entry, &purpose).clone(),
                sha256: pair_policy.sha256.clone(),
                signer_key_id: pair_policy.signer_key_id.clone(),
                schema: pair_policy.schema,
                tree: pair_entry.new.clone(),
            },
        })
    }
    pub fn subject(&self) -> &Subject {
        &self.subject
    }
    pub fn code(&self) -> &[CodeVerificationTarget] {
        &self.code
    }
    pub fn pair(&self) -> &PairVerificationTarget {
        &self.pair
    }
}

fn updater_role(role: NativeComponentRole) -> Role {
    match role {
        NativeComponentRole::Control => Role::Control,
        NativeComponentRole::Ime => Role::Ime,
        NativeComponentRole::Settings => Role::Settings,
        NativeComponentRole::Updater | NativeComponentRole::Bootstrap => unreachable!(),
    }
}
fn code_role(role: NativeComponentRole) -> CodeRole {
    match role {
        NativeComponentRole::Control => CodeRole::Control,
        NativeComponentRole::Ime => CodeRole::Ime,
        NativeComponentRole::Settings => CodeRole::Settings,
        NativeComponentRole::Updater => CodeRole::Updater,
        NativeComponentRole::Bootstrap => CodeRole::Bootstrap,
    }
}
fn selected_path<'a>(entry: &'a Entry, purpose: &VerificationPurpose) -> &'a PathBuf {
    match purpose {
        VerificationPurpose::InstalledNew => &entry.destination,
        VerificationPurpose::DownloadedNew
        | VerificationPurpose::StagedNew
        | VerificationPurpose::RollbackOld { .. } => &entry.stage,
    }
}
fn code_expectation(
    subject: &Subject,
    policy: &ReleasePolicySnapshot,
    component: &ComponentPolicySnapshot,
    purpose: CodePurpose,
    exact_bundle_path: &std::path::Path,
) -> Result<CodeExpectation> {
    let expectation = CodeExpectation {
        schema_version: 1,
        subject: subject.clone(),
        purpose,
        product_id: policy.product_id.clone(),
        role: code_role(component.role),
        exact_bundle_path: exact_bundle_path.into(),
        bundle_id: component.bundle_id.clone(),
        release_id: policy.release_id.clone(),
        version: policy.version.clone(),
        build: policy.build,
        source_commit: policy.source_commit.clone(),
        team_id: component.team_id.clone(),
        architectures: component
            .slices
            .iter()
            .map(|slice| slice.0.clone())
            .collect(),
        cdhashes: component
            .slices
            .iter()
            .map(|slice| slice.1.clone())
            .collect(),
    };
    expectation
        .validate()
        .map_err(|_| Error::Adapter("native code expectation"))?;
    Ok(expectation)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subject() -> Subject {
        Subject {
            transaction_id: "11111111-1111-4111-8111-111111111111".into(),
            plan_sha256: "a".repeat(64),
            installation_id: "22222222-2222-4222-8222-222222222222".into(),
            new_release_id: "inputia-new".into(),
        }
    }
    fn fingerprint(seed: char) -> Fingerprint {
        Fingerprint {
            sha256: seed.to_string().repeat(64),
            bytes: 10,
            entries: 1,
        }
    }
    fn entry(role: Role, name: &str, seed: char) -> Entry {
        Entry {
            role,
            source: None,
            destination: format!("/installed/{name}").into(),
            stage: format!("/staged/{name}").into(),
            backup: format!("/backup/{name}").into(),
            failed: format!("/failed/{name}").into(),
            old: None,
            new: fingerprint(seed),
        }
    }
    fn entries() -> Vec<Entry> {
        vec![
            entry(Role::Control, "Inputia.app", '1'),
            entry(Role::Ime, "InputiaInputMethod.app", '2'),
            entry(Role::Settings, "InputiaSettings.app", '3'),
            entry(Role::PairManifest, "pair.json", '4'),
        ]
    }
    fn component(
        role: NativeComponentRole,
        bundle_id: &str,
        seed: char,
    ) -> ComponentPolicySnapshot {
        ComponentPolicySnapshot {
            role,
            bundle_id: bundle_id.into(),
            team_id: "TESTTEAM01".into(),
            slices: vec![("arm64".into(), seed.to_string().repeat(40))],
        }
    }
    fn policy(release_id: &str) -> ReleasePolicySnapshot {
        ReleasePolicySnapshot {
            product_id: "com.inputia".into(),
            release_id: release_id.into(),
            version: "1.1.1".into(),
            build: 85,
            source_commit: "b".repeat(40),
            components: vec![
                component(NativeComponentRole::Control, "com.inputia.control", '1'),
                component(NativeComponentRole::Ime, "com.inputia.ime", '2'),
                component(NativeComponentRole::Settings, "com.inputia.settings", '3'),
                component(NativeComponentRole::Updater, "com.inputia.updater", '4'),
                component(NativeComponentRole::Bootstrap, "com.inputia.bootstrap", '5'),
            ],
            pair: PairPolicySnapshot {
                sha256: "6".repeat(64),
                signer_key_id: "release-test-key".into(),
                schema: 2,
            },
        }
    }

    #[test]
    fn binds_exact_transaction_paths_and_authorized_identity() {
        let subject = subject();
        let entries = entries();
        let policy = policy(&subject.new_release_id);
        let staged = ArtifactVerificationPlan::from_snapshot(
            &subject,
            &policy,
            &entries,
            VerificationPurpose::StagedNew,
        )
        .unwrap();
        assert_eq!(staged.subject(), &subject);
        assert_eq!(staged.code().len(), 3);
        assert_eq!(staged.code()[0].expectation().role, CodeRole::Control);
        assert_eq!(
            staged.code()[0].expectation().exact_bundle_path,
            entries[0].stage
        );
        assert_eq!(staged.code()[0].tree(), &entries[0].new);
        assert_eq!(staged.pair().exact_path(), entries[3].stage);
        assert_eq!(staged.pair().tree(), &entries[3].new);
        assert_eq!(staged.pair().sha256(), "6".repeat(64));
        assert_eq!(staged.pair().signer_key_id(), "release-test-key");
        assert_eq!(staged.pair().schema(), 2);

        let installed = ArtifactVerificationPlan::from_snapshot(
            &subject,
            &policy,
            &entries,
            VerificationPurpose::InstalledNew,
        )
        .unwrap();
        assert_eq!(
            installed.code()[0].expectation().exact_bundle_path,
            entries[0].destination
        );
        assert_eq!(installed.pair().exact_path(), entries[3].destination);
    }

    #[test]
    fn binds_rollback_to_a_distinct_authorized_release() {
        let subject = subject();
        let entries = entries();
        let old = policy("inputia-old");
        let rollback = ArtifactVerificationPlan::from_snapshot(
            &subject,
            &old,
            &entries,
            VerificationPurpose::RollbackOld {
                release_id: "inputia-old".into(),
            },
        )
        .unwrap();
        assert_eq!(
            rollback.code()[0].expectation().purpose,
            CodePurpose::PreviousRelease
        );
        assert_eq!(rollback.code()[0].expectation().release_id, "inputia-old");
        assert!(ArtifactVerificationPlan::from_snapshot(
            &subject,
            &policy("inputia-new"),
            &entries,
            VerificationPurpose::RollbackOld {
                release_id: "inputia-new".into(),
            },
        )
        .is_err());
    }

    #[test]
    fn rejects_role_ambiguity_incomplete_policy_and_invalid_pair_schema() {
        let subject = subject();
        let mut ambiguous = entries();
        ambiguous.push(ambiguous[0].clone());
        assert!(ArtifactVerificationPlan::from_snapshot(
            &subject,
            &policy("inputia-new"),
            &ambiguous,
            VerificationPurpose::StagedNew,
        )
        .is_err());
        let mut extra = entries();
        extra.push(entry(Role::Receipt, "receipt.json", '7'));
        assert!(ArtifactVerificationPlan::from_snapshot(
            &subject,
            &policy("inputia-new"),
            &extra,
            VerificationPurpose::StagedNew,
        )
        .is_err());
        let mut incomplete = policy("inputia-new");
        incomplete
            .components
            .retain(|component| component.role != NativeComponentRole::Ime);
        assert!(ArtifactVerificationPlan::from_snapshot(
            &subject,
            &incomplete,
            &entries(),
            VerificationPurpose::StagedNew,
        )
        .is_err());
        let mut wrong_pair = policy("inputia-new");
        wrong_pair.pair.schema = 1;
        assert!(ArtifactVerificationPlan::from_snapshot(
            &subject,
            &wrong_pair,
            &entries(),
            VerificationPurpose::StagedNew,
        )
        .is_err());
    }

    #[test]
    fn rejects_release_mismatch_and_unsafe_bundle_path() {
        let subject = subject();
        assert!(ArtifactVerificationPlan::from_snapshot(
            &subject,
            &policy("inputia-other"),
            &entries(),
            VerificationPurpose::DownloadedNew,
        )
        .is_err());
        let mut unsafe_entries = entries();
        unsafe_entries[0].stage = "relative/Inputia.app".into();
        assert!(ArtifactVerificationPlan::from_snapshot(
            &subject,
            &policy("inputia-new"),
            &unsafe_entries,
            VerificationPurpose::DownloadedNew,
        )
        .is_err());
    }
}
