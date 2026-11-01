//! Release-owner handoff, not a worker installer. Only authenticated native host
//! transport may call these functions. A package version is not runtime evidence.
use super::{conflict, daemon_grant, NativeRequest};
use crate::buddy::improvement::{self, artifacts};
use crate::buddy::improvement_evaluation::{digest, valid_digest};
use crate::buddy::operator_store::{OperatorStore, OperatorStoreError, Stored};
use crate::buddy::resource_execution::{ExactActionGrant, ResourceDocument};
use crate::buddy::action_execution::{Authorizer, PreparedAction};
use refact_buddy_core::operator_types::{
    ActionSpec, ProjectIdentity, AuthorityClass, ConfirmationClass,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const ADAPTER: &str = "candidate_host_release";
pub const ACTION: &str = "native_host_package_lifecycle";
pub const NAMESPACE: &str = "improvement_host_release";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HostTarget {
    Vscode,
    Jetbrains,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HostOperation {
    Install,
    Recover,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReleaseDraft {
    /// Exact prior native release whose preimage is this evaluated recovery package.
    pub recovery_ref: Option<String>,
    pub project: ProjectIdentity,
    pub candidate_id: String,
    pub candidate_digest: String,
    pub proof_id: String,
    pub artifact_digest: String,
    pub host_session_id: String,
    pub target: HostTarget,
    pub operation: HostOperation,
    pub expected_installed_manifest_digest: String,
    pub expected_running_manifest_digest: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HostReleaseRequest {
    pub schema_version: u32,
    pub prepared_hash: String,
    pub grant_id: String,
    pub project_id: String,
    pub host_session_id: String,
    pub target: HostTarget,
    pub operation: HostOperation,
    pub candidate_id: String,
    pub candidate_digest: String,
    pub proof_id: String,
    pub artifact_digest: String,
    pub manifest_digest: String,
    pub package_version: String,
    pub expected_installed_manifest_digest: String,
    pub expected_running_manifest_digest: String,
    pub files: BTreeMap<String, String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InstalledIdentity {
    pub manifest_digest: String,
    pub version: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RunningIdentity {
    pub manifest_digest: String,
    pub version: String,
    pub host_session_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptStatus {
    Declined,
    InstallerReturned,
    OutcomeUnknown,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HostReleaseReceipt {
    pub schema_version: u32,
    pub prepared_hash: String,
    pub grant_id: String,
    pub host_session_id: String,
    pub artifact_digest: String,
    pub status: ReceiptStatus,
    pub installed: Option<InstalledIdentity>,
    pub running: Option<RunningIdentity>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HostRunningObservation {
    pub schema_version: u32,
    pub prepared_hash: String,
    pub grant_id: String,
    /// Original dispatch session; only running.host_session_id is the new session.
    pub host_session_id: String,
    pub artifact_digest: String,
    pub installed: InstalledIdentity,
    pub running: RunningIdentity,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReleaseRecord {
    pub schema_version: u32,
    pub project: ProjectIdentity,
    pub request: HostReleaseRequest,
    pub receipt: Option<HostReleaseReceipt>,
    pub installed_verified: bool,
    pub running_verified: bool,
    #[serde(default)]
    pub running_observation: Option<HostRunningObservation>,
}

pub fn validate_spec(spec: &ActionSpec, draft: &ReleaseDraft) -> Result<(), OperatorStoreError> {
    let target = match draft.target {
        HostTarget::Vscode => "vscode",
        HostTarget::Jetbrains => "jetbrains",
    };
    if spec.schema_version != 1
        || spec.adapter != ADAPTER
        || spec.kind != ACTION
        || spec.resource.resource_kind != ADAPTER
        || spec.resource.resource_id != target
        || spec.project != draft.project
        || spec.expected_version != digest(draft)?
        || spec.resource.version != spec.expected_version
        || spec.confirmation != ConfirmationClass::UserConfirmationRequired
        || spec.authority_class != AuthorityClass::Mutating
        || draft.host_session_id.is_empty()
        || draft.host_session_id.len() > 128
        || [
            &draft.candidate_digest,
            &draft.artifact_digest,
            &draft.expected_installed_manifest_digest,
            &draft.expected_running_manifest_digest,
        ]
        .iter()
        .any(|v| !valid_digest(v))
    {
        return Err(conflict("release spec binding"));
    }
    Ok(())
}

/// Validate the exact Basic-user prepared action again immediately before handing
/// out bytes. A worker bearer, event body or self-declared host cannot supply authority.
fn authorized(
    store: &OperatorStore,
    native: &NativeRequest,
) -> Result<(ActionSpec, ReleaseDraft), OperatorStoreError> {
    if !valid_digest(&native.prepared_hash) {
        return Err(conflict("prepared hash"));
    }
    let prepared: Stored<ResourceDocument<PreparedAction>> = store
        .get_document("prepared_action", &native.prepared_hash)?
        .ok_or_else(|| conflict("prepared missing"))?;
    let prepared = prepared.value.payload;
    let spec = prepared.spec;
    if prepared.spec_hash != native.prepared_hash
        || crate::buddy::action_execution::action_spec_hash(&spec, &prepared.bound_parameter_ref)
            != native.prepared_hash
        || prepared.bound_parameter_ref != spec.parameter_ref
        || &spec.project != store.identity()
    {
        return Err(conflict("prepared changed"));
    }
    let bound: Stored<ResourceDocument<ReleaseDraft>> = store
        .get_document("prepared_draft", &native.prepared_hash)?
        .ok_or_else(|| conflict("bound draft missing"))?;
    let reference = spec
        .parameter_ref
        .as_deref()
        .ok_or_else(|| conflict("reference missing"))?;
    let current: Stored<ResourceDocument<ReleaseDraft>> = store
        .get_document("resource_draft", reference)?
        .ok_or_else(|| conflict("draft missing"))?;
    if bound.value.payload != current.value.payload {
        return Err(conflict("draft changed"));
    }
    let draft = bound.value.payload;
    validate_spec(&spec, &draft)?;
    let grant: Stored<ResourceDocument<ExactActionGrant>> = store
        .get_document("action_grant", &native.grant_id)?
        .ok_or_else(|| conflict("grant missing"))?;
    let grant = grant.value.payload;
    let envelope: Stored<serde_json::Value> = store
        .get_document("prepared_action", &native.prepared_hash)?
        .ok_or_else(|| conflict("preparation missing"))?;
    if !daemon_grant(&grant, &native.grant_id)
        || !grant.authorize(&spec, &grant.requester, grant.decision.as_ref())
        || envelope.value["daemon_prepared_by"].as_str() != Some(&grant.requester.source_id)
        || envelope.value["daemon_preparation_version"].as_u64() != Some(1)
    {
        return Err(conflict("native user grant denied"));
    }
    Ok((spec, draft))
}

#[cfg(target_os = "linux")]
pub fn prepare_request(
    store: &OperatorStore,
    native: &NativeRequest,
) -> Result<(HostReleaseRequest, Vec<u8>), OperatorStoreError> {
    let (_, draft) = authorized(store, native)?;
    let candidate = improvement::activation_gate(
        store,
        &draft.candidate_id,
        &draft.candidate_digest,
        &draft.proof_id,
    )?;
    // Recovery is another evaluated known-good package and fresh exact grant,
    // not an automatic downgrade/replay of a previous installation operation.
    let (artifact, inspection, bytes, files) =
        artifacts::release_package(store, &candidate).map_err(conflict)?;
    let format = match draft.target {
        HostTarget::Vscode => artifacts::ArtifactFormat::VscodeVsix,
        HostTarget::Jetbrains => artifacts::ArtifactFormat::JetbrainsZip,
    };
    if artifact.format != format || artifact.artifact_digest != draft.artifact_digest {
        return Err(conflict("package target changed"));
    }
    let manifest_digest = digest(&files)?;
    if draft.operation == HostOperation::Recover {
        let previous: Stored<ReleaseRecord> = store
            .get_document(
                NAMESPACE,
                draft
                    .recovery_ref
                    .as_deref()
                    .ok_or_else(|| conflict("recovery reference missing"))?,
            )?
            .ok_or_else(|| conflict("original release missing"))?;
        if previous.value.project != draft.project
            || !previous.value.installed_verified
            || previous.value.request.target != draft.target
            || previous.value.request.expected_installed_manifest_digest != manifest_digest
            || previous.value.request.manifest_digest != draft.expected_installed_manifest_digest
        {
            return Err(conflict(
                "recovery preimage package differs from native original release",
            ));
        }
    } else if draft.recovery_ref.is_some() {
        return Err(conflict("install cannot inherit recovery authority"));
    }
    Ok((
        HostReleaseRequest {
            schema_version: 1,
            prepared_hash: native.prepared_hash.clone(),
            grant_id: native.grant_id.clone(),
            project_id: draft.project.project_id,
            host_session_id: draft.host_session_id,
            target: draft.target,
            operation: draft.operation,
            candidate_id: draft.candidate_id,
            candidate_digest: draft.candidate_digest,
            proof_id: draft.proof_id,
            artifact_digest: draft.artifact_digest,
            manifest_digest: digest(&files)?,
            package_version: inspection
                .package_version
                .ok_or_else(|| conflict("package version missing"))?,
            expected_installed_manifest_digest: draft.expected_installed_manifest_digest,
            expected_running_manifest_digest: draft.expected_running_manifest_digest,
            files,
        },
        bytes,
    ))
}

/// Called only by the native authenticated host transport, never by a generic
/// resource worker endpoint. Unknown/mismatched reports never become success.
pub fn record_receipt(
    store: &mut OperatorStore,
    receipt: HostReleaseReceipt,
) -> Result<ReleaseRecord, OperatorStoreError> {
    let retained: Stored<ReleaseRecord> = store
        .get_document(NAMESPACE, &receipt.prepared_hash)?
        .ok_or_else(|| conflict("release not dispatched"))?;
    let mut record = retained.value;
    let request = &record.request;
    if receipt.schema_version != 1
        || receipt.grant_id != request.grant_id
        || receipt.host_session_id != request.host_session_id
        || receipt.artifact_digest != request.artifact_digest
    {
        return Err(conflict("foreign host receipt"));
    }
    if let Some(old) = &record.receipt {
        if old == &receipt {
            return Ok(record);
        }
        return Err(conflict("terminal host receipt cannot be overwritten"));
    }
    record.installed_verified = receipt.status == ReceiptStatus::InstallerReturned
        && receipt.installed.as_ref().is_some_and(|i| {
            i.manifest_digest == request.manifest_digest && i.version == request.package_version
        });
    record.running_verified = record.installed_verified
        && receipt.running.as_ref().is_some_and(|r| {
            r.manifest_digest == request.manifest_digest
                && r.version == request.package_version
                && r.host_session_id != request.host_session_id
                && !r.host_session_id.is_empty()
        });
    record.receipt = Some(receipt);
    store.put_reserved_document(
        "host_release",
        &record.request.prepared_hash,
        NAMESPACE,
        &record.request.prepared_hash,
        &record,
        Some(retained.revision),
    )?;
    Ok(record)
}

/// Preserve the installer receipt verbatim and append at most one matching fresh
/// native activation observation. This endpoint cannot trigger activation/reload.
pub fn record_running(
    store: &mut OperatorStore,
    observation: HostRunningObservation,
) -> Result<ReleaseRecord, OperatorStoreError> {
    let retained: Stored<ReleaseRecord> = store
        .get_document(NAMESPACE, &observation.prepared_hash)?
        .ok_or_else(|| conflict("release missing"))?;
    let mut record = retained.value;
    if record.running_observation.as_ref() == Some(&observation) {
        return Ok(record);
    }
    let request = &record.request;
    // Restart-required installers truthfully return the old installed identity.
    // Only a returned installer authorizes reconciliation, never decline/unknown.
    if observation.schema_version != 1
        || record.running_observation.is_some()
        || !record
            .receipt
            .as_ref()
            .is_some_and(|r| r.status == ReceiptStatus::InstallerReturned)
        || observation.prepared_hash != request.prepared_hash
        || observation.grant_id != request.grant_id
        || observation.host_session_id != request.host_session_id
        || observation.artifact_digest != request.artifact_digest
        || observation.installed.manifest_digest != request.manifest_digest
        || observation.installed.version != request.package_version
        || observation.running.host_session_id.is_empty()
        || observation.running.host_session_id == request.host_session_id
        || observation.running.manifest_digest != request.manifest_digest
        || observation.running.version != request.package_version
    {
        return Err(conflict("fresh native activation observation differs"));
    }
    record.installed_verified = true;
    record.running_verified = true;
    record.running_observation = Some(observation);
    store.put_reserved_document(
        "host_release",
        &record.request.prepared_hash,
        NAMESPACE,
        &record.request.prepared_hash,
        &record,
        Some(retained.revision),
    )?;
    Ok(record)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    /// Independent evaluation is fixture evidence, not a compiler/IDE certification.
    /// All candidate/proof checks and archive inspection/private staging remain real.
    #[cfg(target_os = "linux")]
    pub(crate) async fn evaluated_package(
        runtime: &crate::buddy::outcome_runtime::OutcomeRuntime,
        target: HostTarget,
        version: &str,
    ) -> (ReleaseDraft, Vec<u8>) {
        use crate::buddy::improvement::*;
        use crate::buddy::improvement_evaluation::*;
        use refact_buddy_core::operator_types::*;
        use sha2::{Digest, Sha256};
        use std::io::Write;
        fn archive(files: &[(&str, &[u8])]) -> Vec<u8> {
            let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            for (name, bytes) in files {
                zip.start_file(*name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                zip.write_all(bytes).unwrap();
            }
            zip.finish().unwrap().into_inner()
        }
        let gui = format!("bundled GUI {version}");
        let version = version.to_string();
        let project = runtime.project();
        let root = std::path::Path::new(&project.canonical_root);
        if !root.join(".git").exists() {
            let repo = git2::Repository::init(root).unwrap();
            std::fs::write(root.join(".gitignore"), "target/\n.refact/\n").unwrap();
            std::fs::write(root.join("source.rs"), "// scratch source\n").unwrap();
            let mut index = repo.index().unwrap();
            for path in [".gitignore", "source.rs"] {
                index.add_path(std::path::Path::new(path)).unwrap();
            }
            index.write().unwrap();
            let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
            let signature = git2::Signature::now("Fixture", "fixture@example.invalid").unwrap();
            repo.commit(Some("HEAD"), &signature, &signature, "scratch", &tree, &[])
                .unwrap();
        }
        let worktrees = crate::worktrees::service::WorktreeService::new_async(
            root.parent().unwrap().join("release-cache"),
            root.to_path_buf(),
        )
        .await
        .unwrap();
        let checkout = worktrees
            .create_worktree(crate::worktrees::types::CreateWorktreeRequest {
                branch: Some(format!("refact/chat/package-{version}")),
                ..Default::default()
            })
            .await
            .unwrap()
            .worktree
            .meta;
        std::fs::write(
            checkout.root.join("source.rs"),
            format!("// evaluated {version}\n"),
        )
        .unwrap();
        let source = crate::buddy::improvement_runner::inspect_source(
            &worktrees,
            &checkout.id,
            &["source.rs".into()],
        )
        .await
        .unwrap();
        let bytes = match target {
            HostTarget::Vscode => {
                let package = serde_json::json!({"name":"codify","publisher":"smallcloud","version":version,"main":"out/main.js","engines":{"vscode":"^1.90.0"}}).to_string();
                let xml = format!("<PackageManifest><Identity Id=\"codify\" Version=\"{version}\" Publisher=\"smallcloud\"/><Property Id=\"Microsoft.VisualStudio.Code.Engine\" Value=\"^1.90.0\"/></PackageManifest>");
                archive(&[
                    ("extension.vsixmanifest", xml.as_bytes()),
                    ("extension/package.json", package.as_bytes()),
                    ("extension/out/main.js", b"native code"),
                    ("extension/dist/chat/index.js", gui.as_bytes()),
                ])
            }
            HostTarget::Jetbrains => {
                let xml = format!("<idea-plugin><id>com.smallcloud.codify</id><name>Refact</name><version>{version}</version><idea-version since-build=\"243\"/></idea-plugin>");
                let jar = archive(&[
                    ("META-INF/plugin.xml", xml.as_bytes()),
                    ("webview/chat/index.js", gui.as_bytes()),
                ]);
                archive(&[("refact/lib/refact.jar", &jar)])
            }
        };
        let artifact_digest = hex::encode(Sha256::digest(&bytes));
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
        let mut files = BTreeMap::new();
        for index in 0..archive.len() {
            use std::io::Read;
            let mut file = archive.by_index(index).unwrap();
            let mut contents = Vec::new();
            file.read_to_end(&mut contents).unwrap();
            files.insert(
                file.name().to_string(),
                hex::encode(Sha256::digest(&contents)),
            );
        }
        drop(archive);
        // The staged filename is part of the artifact contract.  A VS Code
        // package is a ZIP internally, but its owner descriptor must still
        // name it as a `.vsix`; inspection deliberately rejects a generic
        // `.zip` for that target.
        let relative_path = match target {
            HostTarget::Vscode => "target/package.vsix",
            HostTarget::Jetbrains => "target/package.zip",
        };
        std::fs::create_dir_all(checkout.root.join("target")).unwrap();
        std::fs::write(checkout.root.join(relative_path), &bytes).unwrap();
        let manifest = digest(&files).unwrap();
        let id = format!("package-{version}");
        let (draft, artifact) = runtime
            .with_store(move |store| {
                let contract: serde_json::Value = serde_json::from_str(include_str!(
                    "../../crates/refact-buddy-core/tests/fixtures/operator_contract_v1.json"
                ))
                .unwrap();
                let mut original: RunOutcome =
                    serde_json::from_value(contract["run_outcome"].clone()).unwrap();
                original.project = project.clone();
                original.run_id = format!("original-{id}");
                store.append_run_outcome(&original).unwrap();
                let operator = NativeSourceIdentity {
                    source_kind: NativeSourceKind::Tool,
                    source_id: "independent-fixture-review".into(),
                    source_version: "1".into(),
                };
                let spec = EvaluationSpec {
                    schema_version: 1,
                    project: project.clone(),
                    id: format!("eval-{id}"),
                    criteria: vec![Criterion {
                        id: "package-review".into(),
                        command_or_protocol: "independent scratch package review".into(),
                        expected_result: "passed".into(),
                        external_review: true,
                    }],
                    corpus_digest: "a".repeat(64),
                    privacy_scope: "scratch only".into(),
                    authority: operator.clone(),
                    max_age_seconds: 3600,
                    execution: None,
                };
                register_spec(store, &spec, &operator).unwrap();
                let candidate = CandidateRecord {
                    schema_version: 1,
                    project: project.clone(),
                    candidate: ImprovementCandidate {
                        schema_version: 1,
                        candidate_id: id.clone(),
                        project: project.clone(),
                        target: "source_candidate".into(),
                        worktree_ref: Some(checkout.id.clone()),
                        config_draft_ref: None,
                        patch_digest: Some(source.patch_digest.clone()),
                        build_identity: Some(artifact_digest.clone()),
                        evaluation_spec_version: spec.id.clone(),
                        proof_refs: vec![],
                        activation_decision: None,
                        previous_known_good_identity: None,
                    },
                    target: DeploymentTarget::PluginPackage,
                    target_files: vec!["source.rs".into()],
                    content_digest: source.patch_digest.clone(),
                    evaluation_digest: digest(&spec).unwrap(),
                    source_identity: source.source_digest.clone(),
                    commit_identity: Some("scratch-commit".into()),
                    engine_build: None,
                    gui_build: None,
                    plugin_package: Some(artifact_digest.clone()),
                    installed_version: None,
                    running_host: None,
                    original_run_id: original.run_id.clone(),
                    original_problem: original.evidence.clone(),
                    activation_action: None,
                    activation_draft_digest: None,
                    submitted_by: operator.clone(),
                };
                register_candidate(store, &candidate, &operator).unwrap();
                let mut evaluation = original;
                evaluation.run_id = format!("review-{id}");
                evaluation.input_fingerprint = digest(&candidate).unwrap();
                evaluation.status = RunStatus::Completed;
                evaluation.evidence[0].digest = Some(artifact_digest.clone());
                evaluation.evidence[0].source = operator.clone();
                evaluation.evidence[0].observed_at = chrono::Utc::now().to_rfc3339();
                store.append_run_outcome(&evaluation).unwrap();
                let proof = EvaluationProof {
                    schema_version: 1,
                    project: project.clone(),
                    id: format!("proof-{id}"),
                    candidate_digest: digest(&candidate).unwrap(),
                    evaluation_digest: digest(&spec).unwrap(),
                    authority: operator,
                    observed_at: chrono::Utc::now().to_rfc3339(),
                    checks: vec![CheckObservation {
                        criterion_id: spec.criteria[0].id.clone(),
                        command_or_protocol: spec.criteria[0].command_or_protocol.clone(),
                        result: CheckResult::Passed,
                        observed_result: "scratch attestation; not live IDE evidence".into(),
                        evidence: evaluation.evidence[0].clone(),
                        run_id: evaluation.run_id,
                    }],
                };
                record_proof(store, &id, &proof, &proof.authority).unwrap();
                let artifact = artifacts::ArtifactDraft {
                    project: project.clone(),
                    candidate_id: id.clone(),
                    candidate_digest: proof.candidate_digest.clone(),
                    proof_id: proof.id.clone(),
                    worktree_id: checkout.id.clone(),
                    relative_path: relative_path.into(),
                    format: match target {
                        HostTarget::Vscode => artifacts::ArtifactFormat::VscodeVsix,
                        HostTarget::Jetbrains => artifacts::ArtifactFormat::JetbrainsZip,
                    },
                    artifact_digest: artifact_digest.clone(),
                    manifest_digest: manifest,
                    attested_version: version.clone(),
                    attested_os: "linux".into(),
                    attested_arch: "x86_64".into(),
                    attested_host_requirement: match target {
                        HostTarget::Vscode => "^1.90.0",
                        HostTarget::Jetbrains => "since-build=243;until-build=*",
                    }
                    .into(),
                    retained_known_good: vec![],
                };

                assert!(activation_gate(store, &id, &proof.candidate_digest, &proof.id).is_ok());
                Ok((
                    ReleaseDraft {
                        recovery_ref: None,
                        project,
                        candidate_id: id,
                        candidate_digest: proof.candidate_digest,
                        proof_id: proof.id,
                        artifact_digest,
                        host_session_id: "old-host".into(),
                        target,
                        operation: HostOperation::Install,
                        expected_installed_manifest_digest: "e".repeat(64),
                        expected_running_manifest_digest: "e".repeat(64),
                    },
                    artifact,
                ))
            })
            .await
            .unwrap();
        let version = digest(&artifact).unwrap();
        let spec = ActionSpec {
            schema_version: 1,
            action_id: format!("stage-{}", draft.candidate_id),
            kind: artifacts::ACTION.into(),
            adapter: artifacts::ADAPTER.into(),
            project: runtime.project(),
            resource: ResourceRef {
                resource_kind: artifacts::ADAPTER.into(),
                resource_id: draft.candidate_id.clone(),
                version: version.clone(),
            },
            expected_version: version,
            parameter_ref: Some("stage-draft".into()),
            availability: ActionAvailability::Available,
            confirmation: ConfirmationClass::UserConfirmationRequired,
            authority_class: AuthorityClass::Mutating,
            capabilities: ActionCapabilities {
                can_preview: true,
                can_execute: true,
                can_verify: true,
                can_recover: false,
                can_cancel: false,
            },
        };
        let adapter = artifacts::ArtifactAdapter::new(
            runtime.clone(),
            worktrees,
            std::sync::Arc::new(crate::buddy::resource_execution::BoundDraft::new(
                "stage-draft".into(),
                artifact,
            )),
        );
        adapter.inspect(&spec).await.unwrap();
        // The fixture supplies explicit exact consent; real ActionService executes
        // the public adapter, including its source/proof/read/private-stage gates.
        use crate::buddy::action_execution::*;
        use crate::buddy::resource_execution::*;
        let requester = NativeSourceIdentity {
            source_kind: NativeSourceKind::External,
            source_id: "scratch-stage-user".into(),
            source_version: "daemon-basic-user-v1".into(),
        };
        let grant = ExactActionGrant {
            native_rights: None,
            recovery_execution_id: None,
            spec_hash: action_spec_hash(&spec, &spec.parameter_ref),
            requester: requester.clone(),
            decision: None,
            expires_at: (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
            revoked: false,
        };
        let action = ActionRequest {
            execution_id: format!("stage-{}", draft.candidate_id),
            idempotency_key: format!("stage-{}", draft.candidate_id),
            spec: spec.clone(),
            bound_parameter_ref: spec.parameter_ref.clone(),
            requester,
            decision: None,
            worker: runtime.worker(),
        };
        let mut catalog = ResourceCatalog::default();
        catalog
            .register(artifacts::ACTION.into(), std::sync::Arc::new(adapter))
            .unwrap();
        let service = catalog
            .service(runtime.clone(), std::sync::Arc::new(grant))
            .unwrap();
        let result = service.execute_async(&action).await;
        assert!(
            matches!(&result, ActionResult::Finished(r) if r.execution.status == ActionExecutionState::Completed),
            "{result:?}"
        );
        (draft, bytes)
    }
    #[cfg(target_os = "linux")]
    pub(crate) fn package_manifest(
        store: &OperatorStore,
        candidate: &improvement::CandidateRecord,
    ) -> String {
        let (_, _, _, files) = artifacts::release_package(store, candidate).unwrap();
        digest(&files).unwrap()
    }
    pub(crate) fn request() -> HostReleaseRequest {
        HostReleaseRequest {
            schema_version: 1,
            prepared_hash: "a".repeat(64),
            grant_id: "grant".into(),
            project_id: "fixture".into(),
            host_session_id: "old-host".into(),
            target: HostTarget::Vscode,
            operation: HostOperation::Install,
            candidate_id: "candidate".into(),
            candidate_digest: "b".repeat(64),
            proof_id: "proof".into(),
            artifact_digest: "c".repeat(64),
            manifest_digest: "d".repeat(64),
            package_version: "1.2.3".into(),
            expected_installed_manifest_digest: "e".repeat(64),
            expected_running_manifest_digest: "e".repeat(64),
            files: BTreeMap::from([("out/main.js".into(), "f".repeat(64))]),
        }
    }
    /// Durable scratch fixture only. Does not assert candidate evaluation or install.
    pub(crate) async fn seed_claim(
        runtime: &crate::buddy::outcome_runtime::OutcomeRuntime,
    ) -> HostReleaseRequest {
        let mut request = request();
        request.project_id = runtime.project().project_id;
        let returned = request.clone();
        runtime
            .with_store(move |store| {
                let record = ReleaseRecord {
                    schema_version: 1,
                    project: store.identity().clone(),
                    request,
                    receipt: None,
                    installed_verified: false,
                    running_verified: false,
                    running_observation: None,
                };
                let key = &record.request.prepared_hash;
                store.reserve_native_documents(
                    "host_release",
                    key,
                    &[(NAMESPACE, key.clone(), 3)],
                )?;
                store.put_reserved_document("host_release", key, NAMESPACE, key, &record, None)?;
                Ok(())
            })
            .await
            .unwrap();
        returned
    }
    #[test]
    fn wire_never_accepts_host_commands_or_body_authority() {
        let value = serde_json::to_value(request()).unwrap();
        for key in ["command", "url", "path", "user_confirmed", "auth_token"] {
            let mut bad = value.clone();
            bad[key] = serde_json::json!("untrusted");
            assert!(serde_json::from_value::<HostReleaseRequest>(bad).is_err());
        }
        let mut wrong = value;
        wrong["target"] = serde_json::json!("shell");
        assert!(serde_json::from_value::<HostReleaseRequest>(wrong).is_err());
    }
    #[tokio::test]
    async fn jetbrains_restart_observation_requires_returned_installer_and_exact_fresh_identities()
    {
        let root = tempfile::tempdir().unwrap();
        let project = ProjectIdentity {
            project_id: "fixture".into(),
            canonical_root: root.path().to_string_lossy().into_owned(),
        };
        let worker = refact_buddy_core::operator_types::WorkerIdentity {
            worker_id: "fixture".into(),
            engine_instance_id: "fixture".into(),
            epoch: 1,
        };
        let runtime =
            crate::buddy::outcome_runtime::OutcomeRuntime::open(root.path(), project, worker)
                .unwrap();
        runtime
            .with_store(|store| {
                for (index, status) in [
                    None,
                    Some(ReceiptStatus::Declined),
                    Some(ReceiptStatus::OutcomeUnknown),
                    Some(ReceiptStatus::InstallerReturned),
                ]
                .into_iter()
                .enumerate()
                {
                    let mut request = request();
                    request.target = HostTarget::Jetbrains;
                    request.prepared_hash = index.to_string().repeat(64);
                    let key = &request.prepared_hash;
                    let record = ReleaseRecord {
                        schema_version: 1,
                        project: store.identity().clone(),
                        request: request.clone(),
                        receipt: None,
                        installed_verified: false,
                        running_verified: false,
                        running_observation: None,
                    };
                    store.reserve_native_documents(
                        "host_release",
                        key,
                        &[(NAMESPACE, key.clone(), 3)],
                    )?;
                    store.put_reserved_document(
                        "host_release",
                        key,
                        NAMESPACE,
                        key,
                        &record,
                        None,
                    )?;
                    let receipt = status.map(|status| HostReleaseReceipt {
                        schema_version: 1,
                        prepared_hash: key.clone(),
                        grant_id: request.grant_id.clone(),
                        host_session_id: request.host_session_id.clone(),
                        artifact_digest: request.artifact_digest.clone(),
                        status,
                        installed: Some(InstalledIdentity {
                            manifest_digest: request.expected_installed_manifest_digest.clone(),
                            version: "old".into(),
                        }),
                        running: Some(RunningIdentity {
                            manifest_digest: request.expected_running_manifest_digest.clone(),
                            version: "old".into(),
                            host_session_id: request.host_session_id.clone(),
                        }),
                    });
                    if let Some(receipt) = &receipt {
                        let pending = record_receipt(store, receipt.clone())?;
                        assert!(!pending.installed_verified && !pending.running_verified);
                    }
                    let observation = HostRunningObservation {
                        schema_version: 1,
                        prepared_hash: key.clone(),
                        grant_id: request.grant_id.clone(),
                        host_session_id: request.host_session_id.clone(),
                        artifact_digest: request.artifact_digest.clone(),
                        installed: InstalledIdentity {
                            manifest_digest: request.manifest_digest.clone(),
                            version: request.package_version.clone(),
                        },
                        running: RunningIdentity {
                            manifest_digest: request.manifest_digest.clone(),
                            version: request.package_version.clone(),
                            host_session_id: "fresh-host".into(),
                        },
                    };
                    // Exact frozen plugin wire: both installed identity and artifact are required.
                    let wire = serde_json::to_value(&observation).unwrap();
                    assert_eq!(
                        serde_json::from_value::<HostRunningObservation>(wire.clone()).unwrap(),
                        observation
                    );
                    for field in ["artifact_digest", "installed"] {
                        let mut missing = wire.clone();
                        missing.as_object_mut().unwrap().remove(field);
                        assert!(serde_json::from_value::<HostRunningObservation>(missing).is_err());
                    }
                    let before: Stored<ReleaseRecord> =
                        store.get_document(NAMESPACE, key)?.unwrap();
                    for field in [
                        "schema",
                        "request",
                        "grant",
                        "artifact",
                        "dispatch_session",
                        "installed_manifest",
                        "installed_version",
                        "running_manifest",
                        "running_version",
                        "old_session",
                        "empty_session",
                    ] {
                        let mut wrong = observation.clone();
                        match field {
                            "schema" => wrong.schema_version = 2,
                            "request" => wrong.prepared_hash = "f".repeat(64),
                            "grant" => wrong.grant_id = "foreign".into(),
                            "artifact" => wrong.artifact_digest = "f".repeat(64),
                            "dispatch_session" => wrong.host_session_id = "fresh-host".into(),
                            "installed_manifest" => {
                                wrong.installed.manifest_digest =
                                    request.expected_installed_manifest_digest.clone()
                            }
                            "installed_version" => wrong.installed.version = "old".into(),
                            "running_manifest" => {
                                wrong.running.manifest_digest =
                                    request.expected_running_manifest_digest.clone()
                            }
                            "running_version" => wrong.running.version = "old".into(),
                            "old_session" => {
                                wrong.running.host_session_id = request.host_session_id.clone()
                            }
                            "empty_session" => wrong.running.host_session_id.clear(),
                            _ => unreachable!(),
                        }
                        assert!(record_running(store, wrong).is_err(), "{field}");
                        assert_eq!(
                            store
                                .get_document::<ReleaseRecord>(NAMESPACE, key)?
                                .unwrap()
                                .revision,
                            before.revision
                        );
                    }
                    if receipt
                        .as_ref()
                        .is_some_and(|r| r.status == ReceiptStatus::InstallerReturned)
                    {
                        let verified = record_running(store, observation.clone())?;
                        assert!(verified.installed_verified && verified.running_verified);
                        assert_eq!(verified.receipt, receipt);
                        let after: Stored<ReleaseRecord> =
                            store.get_document(NAMESPACE, key)?.unwrap();
                        assert_ne!(after.revision, before.revision);
                        assert!(record_running(store, observation.clone())?.running_verified);
                        let mut contradictory = observation;
                        contradictory.running.host_session_id = "another-fresh-host".into();
                        assert!(record_running(store, contradictory).is_err());
                        assert!(record_receipt(store, receipt.unwrap())?.running_verified);
                        assert_eq!(
                            store
                                .get_document::<ReleaseRecord>(NAMESPACE, key)?
                                .unwrap()
                                .revision,
                            after.revision
                        );
                        // Dispatch + immutable receipt + one observation exhaust the three-write claim.
                        let mut fourth = after.value;
                        fourth.running_verified = false;
                        assert!(store
                            .put_reserved_document(
                                "host_release",
                                key,
                                NAMESPACE,
                                key,
                                &fourth,
                                Some(after.revision)
                            )
                            .is_err());
                        assert_eq!(
                            store
                                .get_document::<ReleaseRecord>(NAMESPACE, key)?
                                .unwrap()
                                .revision,
                            after.revision
                        );
                    } else {
                        assert!(record_running(store, observation).is_err());
                        let after: Stored<ReleaseRecord> =
                            store.get_document(NAMESPACE, key)?.unwrap();
                        assert_eq!(after.revision, before.revision);
                        assert!(!after.value.installed_verified && !after.value.running_verified);
                        assert_eq!(after.value.receipt, receipt);
                    }
                }
                Ok(())
            })
            .await
            .unwrap();
        runtime.shutdown().await;
    }
    #[tokio::test]
    async fn installed_receipt_does_not_prove_activation_and_reopen_keeps_history() {
        let root = tempfile::tempdir().unwrap();
        let project = ProjectIdentity {
            project_id: "fixture".into(),
            canonical_root: root.path().to_string_lossy().into_owned(),
        };
        let worker = refact_buddy_core::operator_types::WorkerIdentity {
            worker_id: "fixture".into(),
            engine_instance_id: "fixture".into(),
            epoch: 1,
        };
        let runtime =
            crate::buddy::outcome_runtime::OutcomeRuntime::open(root.path(), project, worker)
                .unwrap();
        let request = seed_claim(&runtime).await;
        runtime
            .with_store(move |store| {
                let receipt = HostReleaseReceipt {
                    schema_version: 1,
                    prepared_hash: request.prepared_hash.clone(),
                    grant_id: request.grant_id.clone(),
                    host_session_id: request.host_session_id.clone(),
                    artifact_digest: request.artifact_digest.clone(),
                    status: ReceiptStatus::InstallerReturned,
                    installed: Some(InstalledIdentity {
                        manifest_digest: request.manifest_digest.clone(),
                        version: request.package_version.clone(),
                    }),
                    running: Some(RunningIdentity {
                        manifest_digest: request.expected_running_manifest_digest.clone(),
                        version: "old".into(),
                        host_session_id: request.host_session_id.clone(),
                    }),
                };
                let mut wrong = receipt.clone();
                wrong.artifact_digest = "f".repeat(64);
                assert!(record_receipt(store, wrong).is_err());
                let first = record_receipt(store, receipt.clone())?;
                assert!(first.installed_verified);
                assert!(!first.running_verified);
                assert!(record_receipt(store, receipt.clone())?.installed_verified);
                let mut observation = HostRunningObservation {
                    schema_version: 1,
                    prepared_hash: request.prepared_hash.clone(),
                    grant_id: request.grant_id.clone(),
                    host_session_id: request.host_session_id.clone(),
                    artifact_digest: request.artifact_digest.clone(),
                    installed: InstalledIdentity {
                        manifest_digest: request.manifest_digest.clone(),
                        version: request.package_version.clone(),
                    },
                    running: RunningIdentity {
                        manifest_digest: request.manifest_digest.clone(),
                        version: request.package_version.clone(),
                        host_session_id: request.host_session_id.clone(),
                    },
                };
                assert!(record_running(store, observation.clone()).is_err());
                observation.running.host_session_id = "fresh-activation".into();
                let final_record = record_running(store, observation.clone())?;
                assert!(final_record.running_verified);
                assert_eq!(final_record.receipt, Some(receipt));
                assert!(record_running(store, observation)?.running_verified);
                Ok(())
            })
            .await
            .unwrap();
        runtime.shutdown().await;
        let reopened = crate::buddy::outcome_runtime::OutcomeRuntime::open(
            root.path(),
            runtime.project(),
            runtime.worker(),
        )
        .unwrap();
        reopened
            .with_store(|store| {
                let retained: Stored<ReleaseRecord> =
                    store.get_document(NAMESPACE, &"a".repeat(64))?.unwrap();
                assert!(retained.value.running_verified);
                assert!(retained.value.receipt.is_some());
                Ok(())
            })
            .await
            .unwrap();
        reopened.shutdown().await;
    }
    fn scratch_store(dir: &tempfile::TempDir) -> OperatorStore {
        let project = ProjectIdentity {
            project_id: "release-fixture".into(),
            canonical_root: dunce::canonicalize(dir.path())
                .unwrap()
                .to_string_lossy()
                .into_owned(),
        };
        OperatorStore::open(dir.path(), &project).unwrap()
    }

    fn release_draft(project: &ProjectIdentity, target: HostTarget) -> ReleaseDraft {
        ReleaseDraft {
            recovery_ref: None,
            project: project.clone(),
            candidate_id: "candidate".into(),
            candidate_digest: "b".repeat(64),
            proof_id: "proof".into(),
            artifact_digest: "c".repeat(64),
            host_session_id: "old-host".into(),
            target,
            operation: HostOperation::Install,
            expected_installed_manifest_digest: "e".repeat(64),
            expected_running_manifest_digest: "e".repeat(64),
        }
    }

    fn release_spec(draft: &ReleaseDraft, reference: &str) -> ActionSpec {
        use refact_buddy_core::operator_types::{ActionAvailability, ActionCapabilities, ResourceRef};
        let version = digest(draft).unwrap();
        ActionSpec {
            schema_version: 1,
            action_id: reference.into(),
            kind: ACTION.into(),
            adapter: ADAPTER.into(),
            project: draft.project.clone(),
            resource: ResourceRef {
                resource_kind: ADAPTER.into(),
                resource_id: match draft.target {
                    HostTarget::Vscode => "vscode",
                    HostTarget::Jetbrains => "jetbrains",
                }
                .into(),
                version: version.clone(),
            },
            expected_version: version,
            parameter_ref: Some(reference.into()),
            availability: ActionAvailability::Available,
            confirmation: ConfirmationClass::UserConfirmationRequired,
            authority_class: AuthorityClass::Mutating,
            capabilities: ActionCapabilities {
                can_preview: true,
                can_execute: true,
                can_verify: true,
                can_recover: false,
                can_cancel: false,
            },
        }
    }

    fn replace<T: Serialize + serde::de::DeserializeOwned + Clone>(
        store: &mut OperatorStore,
        namespace: &str,
        id: &str,
        value: &T,
    ) {
        let revision = store
            .get_document::<serde_json::Value>(namespace, id)
            .unwrap()
            .unwrap()
            .revision;
        store
            .put_document(namespace, id, value, Some(revision))
            .unwrap();
    }

    /// Seeds exactly the four documents the native host transport must present:
    /// the immutable draft, the bytes bound at preparation, the daemon-owned
    /// preparation envelope, and the Basic-user exact grant.
    fn seed_authorization(
        store: &mut OperatorStore,
        draft: &ReleaseDraft,
        reference: &str,
    ) -> NativeRequest {
        use crate::buddy::action_execution::action_spec_hash;
        use refact_buddy_core::operator_types::{
            DecisionReference, NativeSourceIdentity, NativeSourceKind, WorkerIdentity,
        };
        let spec = release_spec(draft, reference);
        let hash = action_spec_hash(&spec, &spec.parameter_ref);
        let requester = NativeSourceIdentity {
            source_kind: NativeSourceKind::External,
            source_id: "scratch-basic-user".into(),
            source_version: "daemon-basic-user-v1".into(),
        };
        let decision = DecisionReference {
            project: draft.project.clone(),
            worker: WorkerIdentity {
                worker_id: "fixture".into(),
                engine_instance_id: "fixture".into(),
                epoch: 1,
            },
            native: NativeSourceIdentity {
                source_kind: NativeSourceKind::External,
                source_id: reference.into(),
                source_version: "daemon-installation-grant-v1".into(),
            },
        };
        let prepared = PreparedAction {
            spec: spec.clone(),
            bound_parameter_ref: spec.parameter_ref.clone(),
            spec_hash: hash.clone(),
        };
        let envelope = serde_json::json!({"schema_version":1,"project":draft.project,"payload":prepared,"daemon_prepared_by":requester.source_id,"daemon_preparation_version":1});
        let document = ResourceDocument {
            schema_version: 1,
            project: draft.project.clone(),
            payload: draft.clone(),
        };
        store
            .put_document("resource_draft", reference, &document, None)
            .unwrap();
        store
            .put_document("prepared_draft", &hash, &document, None)
            .unwrap();
        store
            .put_document("prepared_action", &hash, &envelope, None)
            .unwrap();
        let grant = ResourceDocument {
            schema_version: 1,
            project: draft.project.clone(),
            payload: ExactActionGrant {
                native_rights: None,
                recovery_execution_id: None,
                spec_hash: hash.clone(),
                requester,
                decision: Some(decision),
                expires_at: (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
                revoked: false,
            },
        };
        store
            .put_document("action_grant", reference, &grant, None)
            .unwrap();
        NativeRequest {
            prepared_hash: hash,
            grant_id: reference.into(),
        }
    }

    fn seed_dispatch(store: &mut OperatorStore, prepared_hash: &str) -> HostReleaseRequest {
        let mut request = request();
        request.prepared_hash = prepared_hash.to_owned();
        request.project_id = store.identity().project_id.clone();
        let record = ReleaseRecord {
            schema_version: 1,
            project: store.identity().clone(),
            request: request.clone(),
            receipt: None,
            installed_verified: false,
            running_verified: false,
            running_observation: None,
        };
        store
            .reserve_native_documents(
                "host_release",
                prepared_hash,
                &[(NAMESPACE, prepared_hash.to_owned(), 3)],
            )
            .unwrap();
        store
            .put_reserved_document(
                "host_release",
                prepared_hash,
                NAMESPACE,
                prepared_hash,
                &record,
                None,
            )
            .unwrap();
        request
    }

    fn receipt_for(
        request: &HostReleaseRequest,
        status: ReceiptStatus,
        installed: Option<InstalledIdentity>,
        running: Option<RunningIdentity>,
    ) -> HostReleaseReceipt {
        HostReleaseReceipt {
            schema_version: 1,
            prepared_hash: request.prepared_hash.clone(),
            grant_id: request.grant_id.clone(),
            host_session_id: request.host_session_id.clone(),
            artifact_digest: request.artifact_digest.clone(),
            status,
            installed,
            running,
        }
    }

    #[test]
    fn release_spec_binds_each_supported_host_target_and_refuses_drift() {
        let project = ProjectIdentity {
            project_id: "fixture".into(),
            canonical_root: "/scratch/project".into(),
        };
        for target in [HostTarget::Vscode, HostTarget::Jetbrains] {
            let draft = release_draft(&project, target.clone());
            let spec = release_spec(&draft, "release");
            validate_spec(&spec, &draft).unwrap();
            let mut swapped = draft.clone();
            swapped.target = match target {
                HostTarget::Vscode => HostTarget::Jetbrains,
                HostTarget::Jetbrains => HostTarget::Vscode,
            };
            assert!(validate_spec(&spec, &swapped).is_err());
            for field in [
                "schema",
                "adapter",
                "kind",
                "resource_kind",
                "resource_id",
                "project",
                "expected_version",
                "resource_version",
                "confirmation",
                "authority",
            ] {
                let mut wrong = spec.clone();
                match field {
                    "schema" => wrong.schema_version = 2,
                    "adapter" => wrong.adapter = "shell".into(),
                    "kind" => wrong.kind = "shell_command".into(),
                    "resource_kind" => wrong.resource.resource_kind = "shell".into(),
                    "resource_id" => wrong.resource.resource_id = "daemon_engine".into(),
                    "project" => wrong.project.project_id = "another-project".into(),
                    "expected_version" => wrong.expected_version = "f".repeat(64),
                    "resource_version" => wrong.resource.version = "f".repeat(64),
                    "confirmation" => wrong.confirmation = ConfirmationClass::None,
                    _ => wrong.authority_class = AuthorityClass::ReadOnly,
                }
                assert!(validate_spec(&wrong, &draft).is_err(), "{field}");
            }
            for field in [
                "empty_session",
                "long_session",
                "candidate",
                "artifact",
                "installed",
                "running",
            ] {
                let mut wrong = draft.clone();
                match field {
                    "empty_session" => wrong.host_session_id = String::new(),
                    "long_session" => wrong.host_session_id = "s".repeat(129),
                    "candidate" => wrong.candidate_digest = "not-a-digest".into(),
                    "artifact" => wrong.artifact_digest = "c".repeat(63),
                    "installed" => wrong.expected_installed_manifest_digest = String::new(),
                    _ => wrong.expected_running_manifest_digest = "z".repeat(64),
                }
                assert!(
                    validate_spec(&release_spec(&wrong, "release"), &wrong).is_err(),
                    "{field}"
                );
            }
        }
    }

    #[test]
    fn release_authorization_requires_exact_preparation_draft_and_grant() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = scratch_store(&dir);
        let project = store.identity().clone();
        let draft = release_draft(&project, HostTarget::Jetbrains);
        let native = seed_authorization(&mut store, &draft, "release");
        let (spec, bound) = authorized(&store, &native).unwrap();
        assert_eq!(bound, draft);
        assert_eq!(spec.resource.resource_id, "jetbrains");
        for (prepared_hash, grant_id) in [
            ("not-a-digest".to_string(), "release"),
            ("f".repeat(64), "release"),
            (native.prepared_hash.clone(), "unknown-grant"),
        ] {
            assert!(authorized(
                &store,
                &NativeRequest {
                    prepared_hash,
                    grant_id: grant_id.into(),
                }
            )
            .is_err());
        }
        let mut changed = draft.clone();
        changed.host_session_id = "new-host".into();
        replace(
            &mut store,
            "resource_draft",
            "release",
            &ResourceDocument {
                schema_version: 1,
                project,
                payload: changed,
            },
        );
        assert!(authorized(&store, &native).is_err());
    }

    #[test]
    fn revoked_expired_or_unowned_grants_cannot_release_a_package() {
        for mutation in [
            "revoked",
            "expired",
            "requester",
            "decision",
            "provenance",
            "preparation_version",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let mut store = scratch_store(&dir);
            let project = store.identity().clone();
            let draft = release_draft(&project, HostTarget::Vscode);
            let native = seed_authorization(&mut store, &draft, "release");
            authorized(&store, &native).unwrap();
            if mutation == "provenance" || mutation == "preparation_version" {
                let mut envelope: serde_json::Value = store
                    .get_document("prepared_action", &native.prepared_hash)
                    .unwrap()
                    .unwrap()
                    .value;
                if mutation == "provenance" {
                    envelope["daemon_prepared_by"] = serde_json::json!("another-user");
                } else {
                    envelope["daemon_preparation_version"] = serde_json::json!(2);
                }
                replace(
                    &mut store,
                    "prepared_action",
                    &native.prepared_hash,
                    &envelope,
                );
            } else {
                let mut document: ResourceDocument<ExactActionGrant> = store
                    .get_document("action_grant", "release")
                    .unwrap()
                    .unwrap()
                    .value;
                match mutation {
                    "revoked" => document.payload.revoked = true,
                    "expired" => {
                        document.payload.expires_at =
                            (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339()
                    }
                    "requester" => {
                        document.payload.requester.source_version = "worker-bearer".into()
                    }
                    _ => document.payload.decision = None,
                }
                replace(&mut store, "action_grant", "release", &document);
            }
            assert!(authorized(&store, &native).is_err(), "{mutation}");
        }
    }

    #[test]
    fn a_terminal_host_receipt_cannot_be_overwritten_or_forged() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = scratch_store(&dir);
        let request = seed_dispatch(&mut store, &"1".repeat(64));
        let receipt = receipt_for(
            &request,
            ReceiptStatus::InstallerReturned,
            Some(InstalledIdentity {
                manifest_digest: request.manifest_digest.clone(),
                version: request.package_version.clone(),
            }),
            None,
        );
        let mut undispatched = receipt.clone();
        undispatched.prepared_hash = "2".repeat(64);
        assert!(record_receipt(&mut store, undispatched).is_err());
        for field in ["schema", "grant", "session", "artifact"] {
            let mut wrong = receipt.clone();
            match field {
                "schema" => wrong.schema_version = 2,
                "grant" => wrong.grant_id = "another-grant".into(),
                "session" => wrong.host_session_id = "another-host".into(),
                _ => wrong.artifact_digest = "f".repeat(64),
            }
            assert!(record_receipt(&mut store, wrong).is_err(), "{field}");
        }
        let recorded = record_receipt(&mut store, receipt.clone()).unwrap();
        assert!(recorded.installed_verified && !recorded.running_verified);
        let retained: Stored<ReleaseRecord> = store
            .get_document(NAMESPACE, &request.prepared_hash)
            .unwrap()
            .unwrap();
        assert_eq!(
            record_receipt(&mut store, receipt.clone()).unwrap().receipt,
            Some(receipt.clone())
        );
        for escalation in ["status", "installed", "running"] {
            let mut wrong = receipt.clone();
            match escalation {
                "status" => wrong.status = ReceiptStatus::Declined,
                "installed" => wrong.installed = None,
                _ => {
                    wrong.running = Some(RunningIdentity {
                        manifest_digest: request.manifest_digest.clone(),
                        version: request.package_version.clone(),
                        host_session_id: "fresh-host".into(),
                    })
                }
            }
            assert!(record_receipt(&mut store, wrong).is_err(), "{escalation}");
        }
        let after: Stored<ReleaseRecord> = store
            .get_document(NAMESPACE, &request.prepared_hash)
            .unwrap()
            .unwrap();
        assert_eq!(after.revision, retained.revision);
        assert_eq!(after.value.receipt, Some(receipt));
        assert!(after.value.installed_verified && !after.value.running_verified);
    }

    #[test]
    fn receipt_verification_requires_exact_installed_and_fresh_running_identity() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = scratch_store(&dir);
        let template = request();
        let installed = InstalledIdentity {
            manifest_digest: template.manifest_digest.clone(),
            version: template.package_version.clone(),
        };
        let running = RunningIdentity {
            manifest_digest: template.manifest_digest.clone(),
            version: template.package_version.clone(),
            host_session_id: "fresh-host".into(),
        };
        let other_manifest = InstalledIdentity {
            manifest_digest: template.expected_installed_manifest_digest.clone(),
            version: template.package_version.clone(),
        };
        let other_version = InstalledIdentity {
            manifest_digest: template.manifest_digest.clone(),
            version: "old".into(),
        };
        let same_session = RunningIdentity {
            host_session_id: template.host_session_id.clone(),
            ..running.clone()
        };
        let empty_session = RunningIdentity {
            host_session_id: String::new(),
            ..running.clone()
        };
        let stale_running = RunningIdentity {
            manifest_digest: template.expected_running_manifest_digest.clone(),
            ..running.clone()
        };
        let cases = [
            (
                "declined",
                ReceiptStatus::Declined,
                Some(installed.clone()),
                Some(running.clone()),
                false,
                false,
            ),
            (
                "unknown",
                ReceiptStatus::OutcomeUnknown,
                Some(installed.clone()),
                Some(running.clone()),
                false,
                false,
            ),
            (
                "no_installed_identity",
                ReceiptStatus::InstallerReturned,
                None,
                None,
                false,
                false,
            ),
            (
                "installed_manifest_differs",
                ReceiptStatus::InstallerReturned,
                Some(other_manifest),
                None,
                false,
                false,
            ),
            (
                "installed_version_differs",
                ReceiptStatus::InstallerReturned,
                Some(other_version),
                None,
                false,
                false,
            ),
            (
                "installed_only",
                ReceiptStatus::InstallerReturned,
                Some(installed.clone()),
                None,
                true,
                false,
            ),
            (
                "running_in_dispatch_session",
                ReceiptStatus::InstallerReturned,
                Some(installed.clone()),
                Some(same_session),
                true,
                false,
            ),
            (
                "running_without_session",
                ReceiptStatus::InstallerReturned,
                Some(installed.clone()),
                Some(empty_session),
                true,
                false,
            ),
            (
                "running_manifest_differs",
                ReceiptStatus::InstallerReturned,
                Some(installed.clone()),
                Some(stale_running),
                true,
                false,
            ),
            (
                "fresh_running_identity",
                ReceiptStatus::InstallerReturned,
                Some(installed),
                Some(running),
                true,
                true,
            ),
        ];
        for (index, (name, status, installed, running, expect_installed, expect_running)) in
            cases.into_iter().enumerate()
        {
            let request = seed_dispatch(&mut store, &format!("{index:x}").repeat(64));
            let record = record_receipt(
                &mut store,
                receipt_for(&request, status, installed, running),
            )
            .unwrap();
            assert_eq!(record.installed_verified, expect_installed, "{name}");
            assert_eq!(record.running_verified, expect_running, "{name}");
        }
    }

    #[test]
    fn running_observation_requires_a_dispatched_release_and_returned_installer() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = scratch_store(&dir);
        let request = seed_dispatch(&mut store, &"3".repeat(64));
        let observation = HostRunningObservation {
            schema_version: 1,
            prepared_hash: request.prepared_hash.clone(),
            grant_id: request.grant_id.clone(),
            host_session_id: request.host_session_id.clone(),
            artifact_digest: request.artifact_digest.clone(),
            installed: InstalledIdentity {
                manifest_digest: request.manifest_digest.clone(),
                version: request.package_version.clone(),
            },
            running: RunningIdentity {
                manifest_digest: request.manifest_digest.clone(),
                version: request.package_version.clone(),
                host_session_id: "fresh-host".into(),
            },
        };
        let mut undispatched = observation.clone();
        undispatched.prepared_hash = "4".repeat(64);
        assert!(record_running(&mut store, undispatched).is_err());
        assert!(record_running(&mut store, observation.clone()).is_err());
        record_receipt(
            &mut store,
            receipt_for(
                &request,
                ReceiptStatus::Declined,
                Some(InstalledIdentity {
                    manifest_digest: request.manifest_digest.clone(),
                    version: request.package_version.clone(),
                }),
                None,
            ),
        )
        .unwrap();
        assert!(record_running(&mut store, observation).is_err());
        let retained: Stored<ReleaseRecord> = store
            .get_document(NAMESPACE, &request.prepared_hash)
            .unwrap()
            .unwrap();
        assert!(!retained.value.installed_verified);
        assert!(!retained.value.running_verified);
        assert!(retained.value.running_observation.is_none());
    }
}

pub struct ReleaseAdapter {
    pub runtime: crate::buddy::outcome_runtime::OutcomeRuntime,
    pub native: NativeRequest,
    pub spec: ActionSpec,
}
#[cfg(target_os = "linux")]
#[async_trait::async_trait]
impl crate::buddy::resources::AsyncResourceAdapter for ReleaseAdapter {
    fn adapter_id(&self) -> &str {
        ADAPTER
    }
    fn capabilities(&self) -> crate::buddy::resources::ResourceCapabilities {
        crate::buddy::resources::ResourceCapabilities {
            preview: true,
            apply: true,
            verify: true,
            recover: false,
        }
    }
    async fn preview(
        &self,
        spec: &ActionSpec,
    ) -> Result<crate::buddy::resources::ResourcePreview, crate::buddy::resources::ResourceError>
    {
        use crate::buddy::resources::*;
        if spec != &self.spec {
            return Err(ResourceError::DraftDoesNotMatchResource);
        }
        Ok(ResourcePreview { resource_id: spec.resource.resource_id.clone(), expected_version: spec.expected_version.clone(), proposed_version: spec.expected_version.clone(), reload_needed: true,
            diff: Some("Stage exact native host install request only. Host confirmation and installer result are separate; no automatic reload, disable or running-version inference.".into()) })
    }
    async fn apply(
        &self,
        spec: &ActionSpec,
    ) -> Result<crate::buddy::resources::ResourceApplyReceipt, crate::buddy::resources::ResourceError>
    {
        use crate::buddy::resources::*;
        if spec != &self.spec {
            return Err(ResourceError::DraftDoesNotMatchResource);
        }
        let native = self.native.clone();
        let post = self
            .runtime
            .with_store(move |store| {
                let (request, _) = prepare_request(store, &native)?;
                if let Some(old) =
                    store.get_document::<ReleaseRecord>(NAMESPACE, &native.prepared_hash)?
                {
                    if old.value.request != request {
                        return Err(conflict("release claim reused"));
                    }
                    return Ok(request.manifest_digest);
                }
                let record = ReleaseRecord {
                    schema_version: 1,
                    project: store.identity().clone(),
                    request,
                    receipt: None,
                    installed_verified: false,
                    running_verified: false,
                    running_observation: None,
                };
                store.reserve_native_documents(
                    "host_release",
                    &native.prepared_hash,
                    &[(NAMESPACE, native.prepared_hash.clone(), 3)],
                )?;
                store.put_reserved_document(
                    "host_release",
                    &native.prepared_hash,
                    NAMESPACE,
                    &native.prepared_hash,
                    &record,
                    None,
                )?;
                Ok(record.request.manifest_digest)
            })
            .await
            .map_err(|e| ResourceError::ValidationFailed(e.to_string()))?;
        Ok(ResourceApplyReceipt {
            resource_id: spec.resource.resource_id.clone(),
            previous_material_reference: String::new(),
            expected_post_version: post,
            state: ResourceApplyState::Applied,
        })
    }
    async fn verify(
        &self,
        spec: &ActionSpec,
        post: &str,
    ) -> Result<bool, crate::buddy::resources::ResourceError> {
        if spec != &self.spec {
            return Ok(false);
        }
        let key = self.native.prepared_hash.clone();
        let post = post.to_string();
        self.runtime
            .with_store(move |store| {
                Ok(store
                    .get_document::<ReleaseRecord>(NAMESPACE, &key)?
                    .is_some_and(|r| {
                        r.value.installed_verified && r.value.request.manifest_digest == post
                    }))
            })
            .await
            .map_err(|e| crate::buddy::resources::ResourceError::ValidationFailed(e.to_string()))
    }
    async fn recover(
        &self,
        _: &ActionSpec,
        _: &crate::buddy::resources::RecoveryDescriptor,
    ) -> Result<crate::buddy::resources::ResourceApplyState, crate::buddy::resources::ResourceError>
    {
        Err(crate::buddy::resources::ResourceError::ValidationFailed("Recovery requires separately evaluated retained package and fresh native exact user grant".into()))
    }
}
