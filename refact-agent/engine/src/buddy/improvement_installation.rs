//! Linux native installation owner. Worker-side execution is deliberately absent:
//! a separate authenticated daemon request consumes an already prepared exact grant.
use refact_buddy_core::operator_types::{ActionSpec, ProjectIdentity};
use serde::{Deserialize, Serialize};
use crate::buddy::improvement_evaluation::{digest, valid_digest};
use crate::buddy::operator_store::{OperatorStore, OperatorStoreError, Stored};
use crate::buddy::resource_execution::{ResourceDocument, ExactActionGrant};
use crate::buddy::action_execution::{Authorizer, PreparedAction};

#[path = "improvement_release.rs"]
pub mod release;

pub const ADAPTER: &str = "candidate_installation";
pub const ACTION: &str = "native_image_lifecycle";
pub const NAMESPACE: &str = "improvement_installation";
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Canary,
    Install,
    Restart,
    Recover,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InstallationDraft {
    pub project: ProjectIdentity,
    pub candidate_id: String,
    pub candidate_digest: String,
    pub proof_id: String,
    pub artifact_digest: String,
    pub expected_installed_digest: String,
    pub operation: Operation,
    /// Exact successful installation operation for restart/recovery, never a path.
    pub installation_ref: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeRequest {
    pub prepared_hash: String,
    pub grant_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RestartIntent {
    pub from_generation: String,
    pub dispatched_at_ms: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunningObservation {
    pub generation: String,
    pub kernel_digest: String,
    pub observed_at_ms: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InstallationRecord {
    pub schema_version: u32,
    pub project: ProjectIdentity,
    pub draft: InstallationDraft,
    pub grant_id: String,
    pub phase: String,
    pub installed_digest: Option<String>,
    pub running_digest: Option<String>,

    pub previous_digest: String,
    pub detail: String,
    #[serde(default)]
    pub restart_intent_generation: Option<String>,
    #[serde(default)]
    pub observed_daemon_generation: Option<String>,
}
fn conflict<E>(_: E) -> OperatorStoreError {
    OperatorStoreError::Conflict(
        "native image owner rejected binding or effect; inspect retained claim",
    )
}
pub fn validate_spec(
    spec: &ActionSpec,
    draft: &InstallationDraft,
) -> Result<(), OperatorStoreError> {
    use refact_buddy_core::operator_types::{ConfirmationClass, AuthorityClass};
    if spec.schema_version != 1
        || spec.adapter != ADAPTER
        || spec.kind != ACTION
        || spec.project != draft.project
        || spec.resource.resource_kind != ADAPTER
        || spec.resource.resource_id != "daemon_engine"
        || spec.expected_version != digest(draft)?
        || spec.resource.version != spec.expected_version
        || spec.confirmation != ConfirmationClass::UserConfirmationRequired
        || spec.authority_class != AuthorityClass::Mutating
        || [
            &draft.candidate_digest,
            &draft.artifact_digest,
            &draft.expected_installed_digest,
        ]
        .iter()
        .any(|v| !valid_digest(v))
        || draft
            .installation_ref
            .as_ref()
            .is_some_and(|v| !valid_digest(v))
        || matches!(draft.operation, Operation::Install | Operation::Canary)
            != draft.installation_ref.is_none()
    {
        return Err(conflict("invalid installation spec"));
    }
    Ok(())
}
pub(crate) fn daemon_grant(grant: &ExactActionGrant, id: &str) -> bool {
    use refact_buddy_core::operator_types::NativeSourceKind;
    grant.recovery_execution_id.is_none()
        && grant.native_rights.is_none()
        && grant.requester.source_kind == NativeSourceKind::External
        && grant.requester.source_version == "daemon-basic-user-v1"
        && grant.decision.as_ref().is_some_and(|d| {
            d.native.source_kind == NativeSourceKind::External
                && d.native.source_id == id
                && d.native.source_version == "daemon-installation-grant-v1"
        })
}

/// Native compatibility is intentionally narrower than artifact format support:
/// static little-endian ELF64, exact architecture, and independent host-protocol proof.
#[cfg(target_os = "linux")]
fn static_host_image(bytes: &[u8], arch: &str) -> Result<(), OperatorStoreError> {
    let machine = match arch {
        "x86_64" => 62u16,
        "aarch64" => 183,
        _ => return Err(conflict("architecture")),
    };
    if arch != std::env::consts::ARCH
        || bytes.len() < 64
        || &bytes[..7] != b"\x7fELF\x02\x01\x01"
        || !matches!(bytes[7], 0 | 3)
        || u16::from_le_bytes(bytes[16..18].try_into().unwrap()) != 2
        || u16::from_le_bytes(bytes[18..20].try_into().unwrap()) != machine
        || u32::from_le_bytes(bytes[20..24].try_into().unwrap()) != 1
        || u16::from_le_bytes(bytes[52..54].try_into().unwrap()) != 64
        || u16::from_le_bytes(bytes[54..56].try_into().unwrap()) != 56
    {
        return Err(conflict("invalid executable header"));
    }
    let read64 = |at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
    let offset = usize::try_from(read64(32)).map_err(conflict)?;
    let count = u16::from_le_bytes(bytes[56..58].try_into().unwrap()) as usize;
    let end = count
        .checked_mul(56)
        .and_then(|size| offset.checked_add(size))
        .ok_or_else(|| conflict("header overflow"))?;
    if count == 0 || count == 0xffff || offset < 64 || end > bytes.len() {
        return Err(conflict("program headers"));
    }
    let entry = read64(24);
    let mut executable_entry = false;
    for p in (offset..end).step_by(56) {
        let kind = u32::from_le_bytes(bytes[p..p + 4].try_into().unwrap());
        if matches!(kind, 2 | 3) {
            return Err(conflict("dynamic host dependencies unverified"));
        }
        if kind != 1 {
            continue;
        }
        let flags = u32::from_le_bytes(bytes[p + 4..p + 8].try_into().unwrap());
        let file_offset = read64(p + 8);
        let virtual_address = read64(p + 16);
        let file_size = read64(p + 32);
        let memory_size = read64(p + 40);
        let alignment = read64(p + 48);
        if file_size > memory_size
            || file_offset
                .checked_add(file_size)
                .is_none_or(|n| n > bytes.len() as u64)
            || virtual_address.checked_add(memory_size).is_none()
            || (alignment > 1
                && (!alignment.is_power_of_two()
                    || file_offset % alignment != virtual_address % alignment))
        {
            return Err(conflict("invalid load segment"));
        }
        executable_entry |= flags & 1 != 0
            && file_size > 0
            && entry >= virtual_address
            && entry
                < virtual_address
                    .checked_add(file_size)
                    .ok_or_else(|| conflict("entry overflow"))?;
    }
    if !executable_entry {
        return Err(conflict("entry outside executable file segment"));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn preflight(
    store: &OperatorStore,
    spec: &ActionSpec,
    draft: &InstallationDraft,
    executable: &std::path::Path,
) -> Result<Option<InstallationRecord>, OperatorStoreError> {
    preflight_with_inventory(
        store,
        spec,
        draft,
        executable,
        &crate::exec::ExecRegistry::new(),
    )
}

#[cfg(target_os = "linux")]
fn preflight_with_inventory(
    store: &OperatorStore,
    spec: &ActionSpec,
    draft: &InstallationDraft,
    executable: &std::path::Path,
    inventory: &dyn CanaryExecutor,
) -> Result<Option<InstallationRecord>, OperatorStoreError> {
    validate_spec(spec, draft)?;
    let candidate = super::activation_gate(
        store,
        &draft.candidate_id,
        &draft.candidate_digest,
        &draft.proof_id,
    )?;
    let (artifact, bytes) =
        super::artifacts::installation_image(store, &candidate).map_err(conflict)?;
    if artifact.artifact_digest != draft.artifact_digest
        || candidate.engine_build.as_deref() != Some(&draft.artifact_digest)
    {
        return Err(conflict("build changed"));
    }
    let dynamic = static_host_image(&bytes, &artifact.attested_arch).is_err();
    if draft.operation == Operation::Canary || dynamic {
        let snapshot = inventory
            .runtime_snapshot(&bytes, &artifact.attested_arch)
            .map_err(conflict)?;
        if draft.operation != Operation::Canary {
            let key = canary_key(draft, &snapshot, &artifact.attested_version)?;
            let proof: Stored<CanaryProof> = store
                .get_document(CANARY_NAMESPACE, &key)?
                .ok_or_else(|| conflict("native canary missing"))?;
            if !proof.value.passed || proof.value.binding != key || proof.value.runtime != snapshot
            {
                return Err(conflict("native canary changed"));
            }
            let execution = store
                .get_execution_by_idempotency_key(&format!(
                    "native-install:{}",
                    proof.value.prepared_hash
                ))?
                .ok_or_else(|| conflict("canary ledger missing"))?;
            if execution.value.spec_hash != proof.value.prepared_hash
                || execution.value.status
                    != refact_buddy_core::operator_types::ActionExecutionState::Completed
            {
                return Err(conflict("canary ledger not completed"));
            }
            let claim: Stored<InstallationRecord> = store
                .get_document(NAMESPACE, &proof.value.prepared_hash)?
                .ok_or_else(|| conflict("canary claim missing"))?;
            if claim.value.phase != "canary_passed"
                || claim.value.draft.operation != Operation::Canary
                || claim.value.draft.candidate_digest != draft.candidate_digest
                || claim.value.draft.artifact_digest != draft.artifact_digest
                || claim.value.draft.proof_id != draft.proof_id
                || claim.value.project != draft.project
            {
                return Err(conflict("canary claim changed"));
            }
        }
    }
    // ELF headers cannot establish syscall/OS compatibility. Require an exact
    // independent evaluator criterion, not the artifact's operator attestation.
    let proof: Stored<crate::buddy::improvement_evaluation::EvaluationProof> = store
        .get_document(
            crate::buddy::improvement_evaluation::PROOF_NAMESPACE,
            &draft.proof_id,
        )?
        .ok_or_else(|| conflict("proof missing"))?;
    let protocol = if dynamic || draft.operation == Operation::Canary {
        format!(
            "native-dynamic-linux:{}:{}:refact-lsp:{}",
            std::env::consts::ARCH,
            draft.artifact_digest,
            artifact.attested_version
        )
    } else {
        format!(
            "native-static-linux:{}:{}",
            std::env::consts::ARCH,
            draft.artifact_digest
        )
    };
    let evaluation: Stored<crate::buddy::improvement_evaluation::EvaluationSpec> = store
        .get_document(
            crate::buddy::improvement_evaluation::SPEC_NAMESPACE,
            &candidate.candidate.evaluation_spec_version,
        )?
        .ok_or_else(|| conflict("spec missing"))?;
    if !evaluation
        .value
        .criteria
        .iter()
        .any(|c| c.external_review && c.command_or_protocol == protocol)
    {
        return Err(conflict(
            "fixed independent native compatibility criterion missing",
        ));
    }
    if !proof.value.checks.iter().any(|c| {
        c.command_or_protocol == protocol
            && c.result == crate::buddy::improvement_evaluation::CheckResult::Passed
    }) {
        return Err(conflict("independent host compatibility proof unavailable"));
    }
    let owner =
        refact_self_update::candidate_install::Installer::open(executable).map_err(conflict)?;
    if owner.installed_digest().map_err(conflict)? != draft.expected_installed_digest {
        return Err(conflict("preimage changed"));
    }
    let original = if let Some(id) = &draft.installation_ref {
        let record: Stored<InstallationRecord> = store
            .get_document(NAMESPACE, id)?
            .ok_or_else(|| conflict("installation missing"))?;
        if record.value.schema_version != 1
            || record.value.project != draft.project
            || record.value.draft.project != draft.project
            || record.value.draft.candidate_id != draft.candidate_id
            || record.value.draft.proof_id != draft.proof_id
            || !matches!(
                record.value.phase.as_str(),
                "installed" | "recovered_installed"
            )
            || (record.value.phase == "recovered_installed"
                && (draft.operation != Operation::Restart
                    || record.value.draft.operation != Operation::Recover
                    || record.value.installed_digest.as_deref() == Some(&draft.artifact_digest)
                    || candidate.candidate.previous_known_good_identity.as_deref()
                        != record.value.installed_digest.as_deref()))
            || (record.value.phase == "installed"
                && record.value.draft.operation != Operation::Install)
            || record.value.draft.artifact_digest != draft.artifact_digest
            || record.value.draft.candidate_digest != draft.candidate_digest
            || record.value.installed_digest.as_deref() != Some(&draft.expected_installed_digest)
        {
            return Err(conflict("installation changed"));
        }
        Some(record.value)
    } else {
        None
    };
    if draft.operation == Operation::Install
        && candidate.candidate.previous_known_good_identity.as_deref()
            != Some(&draft.expected_installed_digest)
    {
        return Err(conflict("known-good mismatch"));
    }
    Ok(original)
}

#[cfg(target_os = "linux")]
pub(crate) fn consume(
    store: &mut OperatorStore,
    request: &NativeRequest,
    executable: &std::path::Path,
) -> Result<InstallationRecord, OperatorStoreError> {
    consume_with_inventory(
        store,
        request,
        executable,
        &crate::exec::ExecRegistry::new(),
    )
}

#[cfg(target_os = "linux")]
fn consume_with_inventory(
    store: &mut OperatorStore,
    request: &NativeRequest,
    executable: &std::path::Path,
    inventory: &dyn CanaryExecutor,
) -> Result<InstallationRecord, OperatorStoreError> {
    use refact_self_update::candidate_install::{Installer, ImageReceipt, running_digest};
    if !valid_digest(&request.prepared_hash) {
        return Err(conflict("hash"));
    }
    let prepared: Stored<ResourceDocument<PreparedAction>> = store
        .get_document("prepared_action", &request.prepared_hash)?
        .ok_or_else(|| conflict("prepared missing"))?;
    let prepared = prepared.value.payload;
    let spec = &prepared.spec;
    if prepared.spec_hash != request.prepared_hash
        || crate::buddy::action_execution::action_spec_hash(spec, &prepared.bound_parameter_ref)
            != request.prepared_hash
        || prepared.bound_parameter_ref != spec.parameter_ref
        || &spec.project != store.identity()
    {
        return Err(conflict("prepared changed"));
    }
    let reference = spec
        .parameter_ref
        .as_deref()
        .ok_or_else(|| conflict("draft missing"))?;
    let bound: Stored<ResourceDocument<InstallationDraft>> = store
        .get_document("prepared_draft", &request.prepared_hash)?
        .ok_or_else(|| conflict("bound missing"))?;
    let current: Stored<ResourceDocument<InstallationDraft>> = store
        .get_document("resource_draft", reference)?
        .ok_or_else(|| conflict("draft missing"))?;
    let draft = bound.value.payload;
    if draft != current.value.payload {
        return Err(conflict("draft changed"));
    }
    validate_spec(spec, &draft)?;
    let grant: Stored<ResourceDocument<ExactActionGrant>> = store
        .get_document("action_grant", &request.grant_id)?
        .ok_or_else(|| conflict("grant missing"))?;
    let grant = grant.value.payload;
    if !daemon_grant(&grant, &request.grant_id)
        || !grant.authorize(spec, &grant.requester, grant.decision.as_ref())
    {
        return Err(conflict("grant denied"));
    }
    let envelope: Stored<serde_json::Value> = store
        .get_document("prepared_action", &request.prepared_hash)?
        .ok_or_else(|| conflict("owner missing"))?;
    if envelope
        .value
        .get("daemon_prepared_by")
        .and_then(|v| v.as_str())
        != Some(grant.requester.source_id.as_str())
        || envelope
            .value
            .get("daemon_preparation_version")
            .and_then(|v| v.as_u64())
            != Some(1)
    {
        return Err(conflict("daemon preparation provenance missing"));
    }
    // Duplicate/unknown outcomes never replay, including after daemon restart.
    if let Some(previous) =
        store.get_document::<InstallationRecord>(NAMESPACE, &request.prepared_hash)?
    {
        if previous.value.grant_id != request.grant_id || previous.value.draft != draft {
            return Err(conflict("claim conflict"));
        }
        return Ok(previous.value);
    }
    let original = preflight_with_inventory(store, spec, &draft, executable, inventory)?;
    let (_, bytes) = super::artifacts::installation_image(
        store,
        &super::candidate(store, &draft.candidate_id)?.value,
    )
    .map_err(conflict)?;
    let owner = Installer::open(executable).map_err(conflict)?;
    let mut record = InstallationRecord {
        schema_version: 1,
        project: draft.project.clone(),
        draft: draft.clone(),
        grant_id: request.grant_id.clone(),
        phase: "claimed_outcome_unknown".into(),
        installed_digest: None,
        running_digest: Some(running_digest().map_err(conflict)?),
        previous_digest: draft.expected_installed_digest.clone(),
        detail: "Durable native claim; never automatically replay an interrupted effect".into(),
        restart_intent_generation: None,
        observed_daemon_generation: None,
    };
    let mut slots = vec![(NAMESPACE, request.prepared_hash.clone(), 5)];
    if draft.operation == Operation::Canary {
        let (artifact, bytes) = super::artifacts::installation_image(
            store,
            &super::candidate(store, &draft.candidate_id)?.value,
        )
        .map_err(conflict)?;
        let snapshot = inventory
            .runtime_snapshot(&bytes, &artifact.attested_arch)
            .map_err(conflict)?;
        slots.push((
            CANARY_NAMESPACE,
            canary_key(&draft, &snapshot, &artifact.attested_version)?,
            1,
        ));
    }
    store.reserve_native_documents("native_installation", &request.prepared_hash, &slots)?;
    let claim = store.put_reserved_document(
        "native_installation",
        &request.prepared_hash,
        NAMESPACE,
        &request.prepared_hash,
        &record,
        None,
    )?;
    match draft.operation {
        Operation::Canary => return Ok(record), // async owner runs only after this durable claim
        Operation::Install => {
            owner
                .install(
                    &bytes,
                    &draft.expected_installed_digest,
                    &draft.artifact_digest,
                    &request.prepared_hash,
                )
                .map_err(conflict)?;
            record.phase = "installed".into();
        }
        Operation::Recover => {
            let original = original.ok_or_else(|| conflict("recovery missing"))?;
            owner
                .recover(
                    &ImageReceipt {
                        before: original.previous_digest,
                        installed: draft.expected_installed_digest.clone(),
                        operation: draft.installation_ref.clone().unwrap(),
                    },
                    &request.prepared_hash,
                )
                .map_err(conflict)?;
            record.phase = "recovered_installed".into();
        }
        Operation::Restart => {
            record.phase = "restart_requested_running_unverified".into();
        }
    }
    record.installed_digest = Some(owner.installed_digest().map_err(conflict)?);
    record.detail = "Installed pathname and kernel-observed currently running image are distinct; restart request is not running success".into();
    store.put_reserved_document(
        "native_installation",
        &request.prepared_hash,
        NAMESPACE,
        &request.prepared_hash,
        &record,
        Some(claim.revision),
    )?;
    Ok(record)
}

#[cfg(target_os = "linux")]
const CANARY_NAMESPACE: &str = "improvement_native_canary";
#[cfg(target_os = "linux")]
#[derive(Clone, Debug, Serialize, Deserialize)]
struct CanaryProof {
    schema_version: u32,
    project: ProjectIdentity,
    binding: String,
    prepared_hash: String,
    runtime: super::artifacts::LinuxRuntimeSnapshot,
    process_id: String,
    output_digest: String,
    passed: bool,
}
#[cfg(target_os = "linux")]
fn canary_key(
    draft: &InstallationDraft,
    runtime: &super::artifacts::LinuxRuntimeSnapshot,
    version: &str,
) -> Result<String, OperatorStoreError> {
    digest(&(
        1,
        &draft.project,
        &draft.candidate_id,
        &draft.candidate_digest,
        &draft.proof_id,
        &draft.artifact_digest,
        runtime,
        "refact-lsp --version; LD_BIND_NOW=1; isolated-v1",
        version,
    ))
}

/// Only the namespace launcher is executed on the host. Everything visible to
/// the candidate is copied into a private scratch tree or created in its namespace.
#[cfg(target_os = "linux")]
fn canary_command(
    root: &std::path::Path,
    runtime: &super::artifacts::LinuxRuntimeSnapshot,
) -> Result<crate::exec::ExecSpawnRequest, OperatorStoreError> {
    use crate::exec::{ExecSpawnRequest, ExecMode};
    let mut args: Vec<String> = [
        "/usr/bin/bwrap",
        "--unshare-all",
        "--die-with-parent",
        "--new-session",
        "--cap-drop",
        "ALL",
        "--clearenv",
        "--ro-bind",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    args.extend([root.to_string_lossy().into_owned(), "/".into()]);
    args.extend(
        [
            "--proc",
            "/proc",
            "--dev",
            "/dev",
            "--tmpfs",
            "/tmp",
            "--dir",
            "/home",
            "--setenv",
            "HOME",
            "/home",
            "--setenv",
            "LD_BIND_NOW",
            "1",
            "--setenv",
            "LANG",
            "C",
            "--chdir",
            "/",
            "--",
        ]
        .into_iter()
        .map(String::from),
    );
    args.extend([
        runtime.interpreter.clone(),
        "--inhibit-cache".into(),
        "--library-path".into(),
        super::artifacts::linux_library_roots(&runtime.arch)
            .map_err(conflict)?
            .join(":"),
        "/refact-lsp".into(),
        "--version".into(),
    ]);
    Ok(ExecSpawnRequest::argv(ExecMode::Foreground, args)
        .with_cwd(root)
        .with_timeout(std::time::Duration::from_secs(15))
        .with_output_drain_timeout(std::time::Duration::from_secs(2))
        .with_transcript_limit(4096))
}

#[cfg(target_os = "linux")]
type CanaryOutput = (
    crate::exec::ExecProcessId,
    crate::exec::ExecProcessSnapshot,
    crate::exec::ExecReadResult,
);
#[cfg(target_os = "linux")]
#[async_trait::async_trait]
pub(crate) trait CanaryExecutor: Send + Sync {
    fn runtime_snapshot(
        &self,
        bytes: &[u8],
        arch: &str,
    ) -> Result<super::artifacts::LinuxRuntimeSnapshot, crate::buddy::resources::ResourceError>
    {
        super::artifacts::linux_runtime_snapshot(bytes, arch)
    }

    fn runtime_file_bytes(
        &self,
        file: &super::artifacts::LinuxRuntimeFile,
    ) -> Result<Vec<u8>, crate::buddy::resources::ResourceError> {
        super::artifacts::linux_runtime_file_bytes(file)
    }

    async fn execute(
        &self,
        command: crate::exec::ExecSpawnRequest,
        shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
        cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<CanaryOutput, String>;
}
#[cfg(target_os = "linux")]
#[async_trait::async_trait]
impl CanaryExecutor for crate::exec::ExecRegistry {
    async fn execute(
        &self,
        command: crate::exec::ExecSpawnRequest,
        shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
        cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<CanaryOutput, String> {
        use std::sync::atomic::Ordering;
        if shutdown.load(Ordering::SeqCst) || cancelled.load(Ordering::SeqCst) {
            return Err("canary cancelled".into());
        }
        if command
            .argv
            .as_ref()
            .is_none_or(|argv| argv.first().map(String::as_str) != Some("/usr/bin/bwrap"))
        {
            return Err("native canary requires its fixed namespace launcher".into());
        }
        let started = self.spawn(command).await?;
        let id = started.snapshot.meta.process_id;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(18);
        let finished = loop {
            tokio::select! {
                result = self.wait(&id) => break result?,
                _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {
                    if shutdown.load(Ordering::SeqCst) || cancelled.load(Ordering::SeqCst) || tokio::time::Instant::now() >= deadline { break self.kill(&id).await?; }
                }
            }
        };
        let read = self.read(&id, 0, None).await;
        if shutdown.load(Ordering::SeqCst)
            || cancelled.load(Ordering::SeqCst)
            || tokio::time::Instant::now() >= deadline
        {
            return Err("canary cancelled or deadline expired".into());
        }
        Ok((id, finished, read))
    }
}
#[cfg(target_os = "linux")]
fn canary_output_passes(output: &CanaryOutput, version: &str) -> bool {
    let text: String = output
        .2
        .chunks
        .iter()
        .filter(|c| c.stream == crate::exec::ExecOutputStream::Stdout)
        .map(|c| c.text.as_str())
        .collect();
    output.1.status
        == (crate::exec::ExecStatus::Exited {
            exit_code: Some(0),
        })
        && output.2.found
        && !output.2.is_truncated
        && text == format!("refact-lsp {version}\n")
}

#[cfg(target_os = "linux")]
impl DaemonInstallationAdapter {
    async fn run_canary(
        &self,
        record: InstallationRecord,
    ) -> Result<InstallationRecord, OperatorStoreError> {
        let key = self.request.prepared_hash.clone();
        let claim_key = key.clone();
        let draft = record.draft.clone();
        let inventory = self.registry.clone();
        // CAS to executing is irrevocable. Dropping this future cannot authorize replay.
        let (artifact, bytes, snapshot) = self
            .runtime
            .with_store(move |store| {
                let mut claim: Stored<InstallationRecord> = store
                    .get_document(NAMESPACE, &claim_key)?
                    .ok_or_else(|| conflict("claim missing"))?;
                if claim.value.phase != "claimed_outcome_unknown" {
                    return Err(conflict("canary already consumed"));
                }
                claim.value.phase = "canary_executing_outcome_unknown".into();
                store.put_reserved_document(
                    "native_installation",
                    &claim_key,
                    NAMESPACE,
                    &claim_key,
                    &claim.value,
                    Some(claim.revision),
                )?;
                let (artifact, bytes) = super::artifacts::installation_image(
                    store,
                    &super::candidate(store, &draft.candidate_id)?.value,
                )
                .map_err(conflict)?;
                if artifact.artifact_digest != draft.artifact_digest {
                    return Err(conflict("artifact changed"));
                }
                super::activation_gate(
                    store,
                    &draft.candidate_id,
                    &draft.candidate_digest,
                    &draft.proof_id,
                )?;
                let snapshot = inventory
                    .runtime_snapshot(&bytes, &artifact.attested_arch)
                    .map_err(conflict)?;
                Ok((artifact, bytes, snapshot))
            })
            .await?;
        self.execute_claimed_canary(record, artifact, bytes, snapshot)
            .await
    }

    async fn execute_claimed_canary(
        &self,
        mut record: InstallationRecord,
        artifact: super::artifacts::ArtifactDraft,
        bytes: Vec<u8>,
        snapshot: super::artifacts::LinuxRuntimeSnapshot,
    ) -> Result<InstallationRecord, OperatorStoreError> {
        let key = self.request.prepared_hash.clone();
        use std::os::unix::fs::PermissionsExt;
        let scratch = tempfile::Builder::new()
            .prefix("refact-native-canary-")
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .map_err(conflict)?;
        for directory in ["proc", "dev", "tmp", "home"] {
            std::fs::create_dir(scratch.path().join(directory)).map_err(conflict)?;
        }
        std::fs::write(scratch.path().join("refact-lsp"), &bytes).map_err(conflict)?;
        std::fs::set_permissions(
            scratch.path().join("refact-lsp"),
            std::fs::Permissions::from_mode(0o500),
        )
        .map_err(conflict)?;
        for (alias, file) in &snapshot.files {
            let path = scratch.path().join(
                alias
                    .strip_prefix('/')
                    .ok_or_else(|| conflict("runtime alias"))?,
            );
            std::fs::create_dir_all(path.parent().ok_or_else(|| conflict("runtime parent"))?)
                .map_err(conflict)?;
            std::fs::write(
                &path,
                self.registry.runtime_file_bytes(file).map_err(conflict)?,
            )
            .map_err(conflict)?;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o500))
                .map_err(conflict)?;
        }
        let mut command = canary_command(scratch.path(), &snapshot)?;
        command.owner.tool_call_id = Some(key.clone());
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        struct CancelOnDrop(std::sync::Arc<std::sync::atomic::AtomicBool>);
        impl Drop for CancelOnDrop {
            fn drop(&mut self) {
                self.0.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let _cancel = CancelOnDrop(cancelled.clone());
        command.abort_flag = Some(cancelled.clone());
        if self.shutdown.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(conflict("daemon shutdown"));
        }
        let registry = self.registry.clone();
        let shutdown = self.shutdown.clone();
        // The owned task retains scratch until wait/kill drains even if HTTP drops.
        let handle = tokio::spawn(async move {
            let _scratch = scratch;
            registry.execute(command, shutdown, cancelled).await
        });
        let result = handle.await.map_err(conflict)?.map_err(conflict)?;
        let output_passes = canary_output_passes(&result, &artifact.attested_version);
        let (id, _, read) = result;
        let output: String = read
            .chunks
            .iter()
            .filter(|c| c.stream == crate::exec::ExecOutputStream::Stdout)
            .map(|c| c.text.as_str())
            .collect();
        let passed = output_passes
            && !self.shutdown.load(std::sync::atomic::Ordering::SeqCst)
            && self
                .registry
                .runtime_snapshot(&bytes, &artifact.attested_arch)
                .map_err(conflict)?
                == snapshot;
        record.phase = if passed {
            "canary_passed"
        } else {
            "canary_failed"
        }
        .into();
        record.detail = "Confined loader/startup --version observation only; not full engine functionality or install authorization".into();
        let proof = CanaryProof {
            schema_version: 1,
            project: record.project.clone(),
            binding: canary_key(&record.draft, &snapshot, &artifact.attested_version)?,
            prepared_hash: key.clone(),
            runtime: snapshot,
            process_id: id.to_string(),
            output_digest: digest(&output)?,
            passed,
        };
        let retained = record.clone();
        self.runtime
            .with_store(move |store| {
                let claim: Stored<InstallationRecord> = store
                    .get_document(NAMESPACE, &key)?
                    .ok_or_else(|| conflict("claim missing"))?;
                if claim.value.phase != "canary_executing_outcome_unknown" {
                    return Err(conflict("claim changed"));
                }
                store.put_reserved_document(
                    "native_installation",
                    &key,
                    NAMESPACE,
                    &key,
                    &retained,
                    Some(claim.revision),
                )?;
                if passed {
                    let prior =
                        store.get_document::<CanaryProof>(CANARY_NAMESPACE, &proof.binding)?;
                    store.put_reserved_document(
                        "native_installation",
                        &key,
                        CANARY_NAMESPACE,
                        &proof.binding,
                        &proof,
                        prior.map(|p| p.revision),
                    )?;
                }
                Ok(())
            })
            .await?;
        Ok(record)
    }
}

/// Native observation changes only an already durable restart intent. A pathname
/// digest, old generation, or the candidate digest after recovery is insufficient.
#[cfg(target_os = "linux")]
pub(crate) fn reconcile_running(
    record: &mut InstallationRecord,
    generation: &str,
    kernel_digest: &str,
) -> bool {
    if record.schema_version != 1
        || record.draft.operation != Operation::Restart
        || record.phase != "restart_dispatch_outcome_unknown"
        || generation.is_empty()
        || !valid_digest(kernel_digest)
        || record
            .restart_intent_generation
            .as_deref()
            .is_none_or(|old| old.is_empty() || old == generation)
        || record.installed_digest.as_deref() != Some(kernel_digest)
        || record.draft.expected_installed_digest != kernel_digest
    {
        return false;
    }
    record.observed_daemon_generation = Some(generation.into());
    record.running_digest = Some(kernel_digest.into());
    record.phase = "daemon_running_verified_workers_unverified".into();
    record.detail = "Fresh daemon generation and kernel image observed; worker and IDE image reconciliation is not verified".into();
    true
}

/// Only constructed by the daemon after its native drain fence and user auth.
#[cfg(target_os = "linux")]
pub(crate) struct DaemonInstallationAdapter {
    pub registry: std::sync::Arc<dyn CanaryExecutor>,
    pub shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub runtime: crate::buddy::outcome_runtime::OutcomeRuntime,
    pub request: NativeRequest,
    pub executable: std::path::PathBuf,
    pub spec: ActionSpec,
}
#[cfg(target_os = "linux")]
#[async_trait::async_trait]
impl crate::buddy::resources::AsyncResourceAdapter for DaemonInstallationAdapter {
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
        let checked = spec.clone();
        let path = self.executable.clone();
        let inventory = self.registry.clone();
        let proposed = self
            .runtime
            .with_store(move |store| {
                let reference = checked
                    .parameter_ref
                    .as_deref()
                    .ok_or_else(|| conflict("draft missing"))?;
                let draft: Stored<ResourceDocument<InstallationDraft>> = store
                    .get_document("resource_draft", reference)?
                    .ok_or_else(|| conflict("draft missing"))?;
                if draft.value.project != *store.identity() {
                    return Err(conflict("foreign draft"));
                }
                preflight_with_inventory(
                    store,
                    &checked,
                    &draft.value.payload,
                    &path,
                    inventory.as_ref(),
                )?;
                Ok((
                    draft.value.payload.artifact_digest,
                    draft.value.payload.operation,
                ))
            })
            .await
            .map_err(|e| ResourceError::ValidationFailed(e.to_string()))?;
        Ok(ResourcePreview {
            resource_id: spec.resource.resource_id.clone(),
            expected_version: spec.expected_version.clone(),
            proposed_version: proposed.0,
            reload_needed: proposed.1 != Operation::Canary,
            diff: Some(if proposed.1 == Operation::Canary {
                "Execute only the fixed isolated refact-lsp --version loader/startup check. This exact grant cannot install, restart or recover; output grants no authority."
            } else {
                "Native image lifecycle; dynamic images require a retained exact confined loader/startup observation and independent evaluation, plus this fresh grant."
            }.into()),
        })
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
        let request = self.request.clone();
        let path = self.executable.clone();
        let inventory = self.registry.clone();
        let mut record = self
            .runtime
            .with_store(move |store| {
                consume_with_inventory(store, &request, &path, inventory.as_ref())
            })
            .await
            .map_err(|e| ResourceError::ValidationFailed(e.to_string()))?;
        if record.draft.operation == Operation::Canary {
            record = self
                .run_canary(record)
                .await
                .map_err(|e| ResourceError::ValidationFailed(e.to_string()))?;
            return Ok(ResourceApplyReceipt {
                resource_id: spec.resource.resource_id.clone(),
                previous_material_reference: String::new(),
                expected_post_version: record.draft.artifact_digest,
                state: ResourceApplyState::Applied,
            });
        }
        Ok(ResourceApplyReceipt {
            resource_id: spec.resource.resource_id.clone(),
            previous_material_reference: String::new(),
            expected_post_version: record.installed_digest.unwrap_or_default(),
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
        let key = self.request.prepared_hash.clone();
        let record = self
            .runtime
            .with_store(move |s| s.get_document::<InstallationRecord>(NAMESPACE, &key))
            .await
            .map_err(|e| crate::buddy::resources::ResourceError::ValidationFailed(e.to_string()))?;
        if record
            .as_ref()
            .is_some_and(|r| r.value.draft.operation == Operation::Canary)
        {
            return Ok(record.is_some_and(|r| {
                r.value.phase == "canary_passed" && r.value.draft.artifact_digest == post
            }));
        }
        Ok(record.is_some_and(|r| {
            matches!(r.value.phase.as_str(), "installed" | "recovered_installed")
                && r.value.installed_digest.as_deref() == Some(post)
        }) && refact_self_update::candidate_install::Installer::open(&self.executable)
            .and_then(|o| o.installed_digest())
            .as_deref()
            == Ok(post))
    }
    async fn recover(
        &self,
        _: &ActionSpec,
        _: &crate::buddy::resources::RecoveryDescriptor,
    ) -> Result<crate::buddy::resources::ResourceApplyState, crate::buddy::resources::ResourceError>
    {
        Err(crate::buddy::resources::ResourceError::UnsupportedResource)
    }
}

#[cfg(all(test, target_os = "linux"))]
pub(crate) mod tests {
    use super::*;
    fn contract() -> serde_json::Value {
        serde_json::from_str(include_str!(
            "../../crates/refact-buddy-core/tests/fixtures/operator_contract_v1.json"
        ))
        .unwrap()
    }
    pub(crate) async fn seed(
        runtime: &crate::buddy::outcome_runtime::OutcomeRuntime,
        target: &std::path::Path,
    ) -> (InstallationDraft, ActionSpec) {
        seed_image(runtime, target, elf(), "scratch-v1".into(), false).await
    }
    async fn seed_image(
        runtime: &crate::buddy::outcome_runtime::OutcomeRuntime,
        target: &std::path::Path,
        bytes: Vec<u8>,
        version: String,
        dynamic: bool,
    ) -> (InstallationDraft, ActionSpec) {
        use sha2::{Digest, Sha256};
        use std::os::unix::fs::PermissionsExt;
        use crate::buddy::improvement::*;
        use refact_buddy_core::operator_types::{NativeSourceIdentity, ImprovementCandidate};
        use crate::buddy::improvement_evaluation::*;
        let image_digest = hex::encode(Sha256::digest(&bytes));
        std::fs::write(target, b"scratch known-good executable bytes").unwrap();
        std::fs::set_permissions(target, std::fs::Permissions::from_mode(0o700)).unwrap();
        let previous = hex::encode(Sha256::digest(std::fs::read(target).unwrap()));
        let project = runtime.project();
        let staging =
            std::path::Path::new(&project.canonical_root).join(".refact/candidate-artifacts");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::set_permissions(&staging, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(staging.join(&image_digest), &bytes).unwrap();
        std::fs::set_permissions(
            staging.join(&image_digest),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        runtime.with_store(move |store| {
        let mut original: refact_buddy_core::operator_types::RunOutcome =
            serde_json::from_value(contract()["run_outcome"].clone()).unwrap();
        original.project = project.clone();
        original.run_id = "original-problem".into();
        store.append_run_outcome(&original).unwrap();
        let operator = NativeSourceIdentity {
            source_kind: refact_buddy_core::operator_types::NativeSourceKind::Tool,
            source_id: "authenticated-independent-operator".into(),
            source_version: "1".into(),
        };
        let spec = EvaluationSpec {
            schema_version: 1,
            project: project.clone(),
            id: "fixed-evaluation".into(),
            criteria: vec![Criterion {
                id: "review".into(),
                command_or_protocol: if dynamic { format!("native-dynamic-linux:{}:{}:refact-lsp:{}", std::env::consts::ARCH, image_digest, version) } else { format!("native-static-linux:{}:{}", std::env::consts::ARCH, image_digest) },
                expected_result: "all specified checks pass".into(),
                external_review: true,
            }],
            corpus_digest: "a".repeat(64),
            privacy_scope: "isolated fixture only".into(),
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
                candidate_id: "candidate".into(),
                project: project.clone(),
                target: "source_candidate".into(),
                worktree_ref: Some("isolated-worktree".into()),
                config_draft_ref: None,
                patch_digest: Some("b".repeat(64)),
                build_identity: Some(image_digest.clone()),
                evaluation_spec_version: spec.id.clone(),
                proof_refs: vec![],
                activation_decision: None,
                previous_known_good_identity: Some(previous.clone()),
            },
            target: DeploymentTarget::EngineBuild,
            target_files: vec!["src/candidate.rs".into()],
            content_digest: "b".repeat(64),
            evaluation_digest: digest(&spec).unwrap(),
            source_identity: "source-tree".into(),
            commit_identity: Some("commit".into()),
            engine_build: Some(image_digest.clone()),
            gui_build: None,
            plugin_package: None,
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
        evaluation.run_id = "independent-result".into();
        evaluation.input_fingerprint = digest(&candidate).unwrap();
        evaluation.status = refact_buddy_core::operator_types::RunStatus::Completed;
        evaluation.evidence[0].digest = Some("c".repeat(64));
        evaluation.evidence[0].observed_at = chrono::Utc::now().to_rfc3339();
        evaluation.evidence[0].source = operator.clone();
        store.append_run_outcome(&evaluation).unwrap();
        let proof = EvaluationProof {
            schema_version: 1,
            project: project.clone(),
            id: "proof".into(),
            candidate_digest: digest(&candidate).unwrap(),
            evaluation_digest: digest(&spec).unwrap(),
            authority: operator,
            observed_at: chrono::Utc::now().to_rfc3339(),
            checks: vec![CheckObservation {
                criterion_id: "review".into(),
                command_or_protocol: spec.criteria[0].command_or_protocol.clone(),
                result: CheckResult::Passed,
                observed_result: "fixture attestation, not live review".into(),
                evidence: evaluation.evidence[0].clone(),
                run_id: evaluation.run_id,
            }],
        };
        record_proof(store, "candidate", &proof, &proof.authority).unwrap();
        let files = std::collections::BTreeMap::from([("engine".to_string(), image_digest.clone())]);
        let manifest = digest(&files).unwrap();
        let artifact = artifacts::ArtifactDraft {
            project: project.clone(), candidate_id: "candidate".into(), candidate_digest: proof.candidate_digest.clone(), proof_id: proof.id.clone(),
            worktree_id: "scratch-fixture".into(), relative_path: "engine".into(), format: artifacts::ArtifactFormat::LinuxElf64,
            artifact_digest: image_digest.clone(), manifest_digest: manifest.clone(), attested_version: version,
            attested_os: "linux".into(), attested_arch: std::env::consts::ARCH.into(), attested_host_requirement: "independent static Linux fixture".into(), retained_known_good: vec![],
        };
        // Persist exactly the private artifact record wire format. This seeds an
        // already staged artifact; it does not pretend to test artifact staging.
        let inspection = artifacts::ArtifactInspection {
            artifact_digest: image_digest.clone(), manifest_digest: manifest, bytes: bytes.len() as u64, files,
            checked_arch: Some(std::env::consts::ARCH.into()), package_version: None, package_host_requirement: None,
            compatibility_status: "structure_checked; metadata explicitly operator-attested, not build proof; ELF System V/Linux ABI is not OS proof; runtime compatibility and build provenance unverified; not installed or running".into(),
        };
        store.put_document("improvement_artifacts", &image_digest, &serde_json::json!({"schema_version":1,"project":project,"draft":artifact,"inspection":inspection}), None)?;
        let draft = InstallationDraft { project: project.clone(), candidate_id: "candidate".into(), candidate_digest: proof.candidate_digest,
            proof_id: proof.id, artifact_digest: image_digest, expected_installed_digest: previous, operation: Operation::Install, installation_ref: None };
        let spec = fixture_spec(&draft, "install");
        store.put_document("resource_draft", "install", &ResourceDocument { schema_version: 1, project, payload: draft.clone() }, None)?;
        Ok((draft, spec))
        }).await.unwrap()
    }
    pub(crate) fn fixture_spec(draft: &InstallationDraft, reference: &str) -> ActionSpec {
        let mut spec: ActionSpec =
            serde_json::from_value(contract()["action_spec"].clone()).unwrap();
        spec.project = draft.project.clone();
        spec.action_id = reference.into();
        spec.kind = ACTION.into();
        spec.adapter = ADAPTER.into();
        spec.resource.resource_kind = ADAPTER.into();
        spec.resource.resource_id = "daemon_engine".into();
        spec.expected_version = digest(draft).unwrap();
        spec.resource.version = spec.expected_version.clone();
        spec.parameter_ref = Some(reference.into());
        spec.authority_class = refact_buddy_core::operator_types::AuthorityClass::Mutating;
        spec.capabilities.can_cancel = false;
        spec
    }

    async fn prepare_fixture(
        runtime: &crate::buddy::outcome_runtime::OutcomeRuntime,
        target: &std::path::Path,
        draft: &InstallationDraft,
        reference: &str,
    ) -> (
        crate::buddy::action_execution::ActionRequest,
        NativeRequest,
        ResourceCatalogFixture,
    ) {
        use crate::buddy::action_execution::*;
        use crate::buddy::resource_execution::*;
        use refact_buddy_core::operator_types::*;
        let spec = fixture_spec(draft, reference);
        let hash = action_spec_hash(&spec, &spec.parameter_ref);
        let requester = NativeSourceIdentity {
            source_kind: NativeSourceKind::External,
            source_id: "scratch-user".into(),
            source_version: "daemon-basic-user-v1".into(),
        };
        let worker = runtime.worker();
        let decision = DecisionReference {
            project: draft.project.clone(),
            worker: worker.clone(),
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
        let d = ResourceDocument {
            schema_version: 1,
            project: draft.project.clone(),
            payload: draft.clone(),
        };
        let r = reference.to_string();
        let h = hash.clone();
        runtime
            .with_store(move |store| {
                if store
                    .get_document::<serde_json::Value>("resource_draft", &r)?
                    .is_none()
                {
                    store.put_document("resource_draft", &r, &d, None)?;
                }
                store.put_document("prepared_action", &h, &envelope, None)?;
                Ok(())
            })
            .await
            .unwrap();
        bind_prepared_draft(runtime, &prepared).await.unwrap();
        let grant = ExactActionGrant {
            native_rights: None,
            spec_hash: hash.clone(),
            requester: requester.clone(),
            decision: Some(decision.clone()),
            expires_at: (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
            revoked: false,
            recovery_execution_id: None,
        };
        persist_grant(runtime, reference.into(), grant.clone())
            .await
            .unwrap();
        let native = NativeRequest {
            prepared_hash: hash,
            grant_id: reference.into(),
        };
        let mut catalog = ResourceCatalog::default();
        catalog
            .register(
                ACTION.into(),
                std::sync::Arc::new(DaemonInstallationAdapter {
                    registry: std::sync::Arc::new(crate::exec::ExecRegistry::new()),
                    shutdown: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    runtime: runtime.clone(),
                    request: native.clone(),
                    executable: target.into(),
                    spec: spec.clone(),
                }),
            )
            .unwrap();
        let request = ActionRequest {
            execution_id: if draft.operation == Operation::Canary {
                format!("native-install:{}", native.prepared_hash)
            } else {
                reference.into()
            },
            idempotency_key: if draft.operation == Operation::Canary {
                format!("native-install:{}", native.prepared_hash)
            } else {
                reference.into()
            },
            bound_parameter_ref: spec.parameter_ref.clone(),
            spec,
            requester,
            decision: Some(decision),
            worker,
        };
        (
            request,
            native,
            ResourceCatalogFixture {
                catalog,
                grant,
            },
        )
    }
    struct ResourceCatalogFixture {
        catalog: crate::buddy::resource_execution::ResourceCatalog,
        grant: ExactActionGrant,
    }
    impl ResourceCatalogFixture {
        async fn execute(
            &self,
            runtime: &crate::buddy::outcome_runtime::OutcomeRuntime,
            request: &crate::buddy::action_execution::ActionRequest,
        ) -> crate::buddy::action_execution::ActionResult {
            self.catalog
                .service(runtime.clone(), std::sync::Arc::new(self.grant.clone()))
                .unwrap()
                .execute_async(request)
                .await
        }
    }
    async fn retained_installation(
        runtime: &crate::buddy::outcome_runtime::OutcomeRuntime,
        native: &NativeRequest,
    ) -> Option<Stored<InstallationRecord>> {
        let key = native.prepared_hash.clone();
        runtime
            .with_store(move |store| store.get_document(NAMESPACE, &key))
            .await
            .unwrap()
    }

    #[test]
    fn canary_argv_is_fixed_namespace_only_and_binding_changes_with_runtime() {
        let project = ProjectIdentity {
            project_id: "fixture".into(),
            canonical_root: "/not-mounted".into(),
        };
        let draft = InstallationDraft {
            project,
            candidate_id: "candidate".into(),
            candidate_digest: "a".repeat(64),
            proof_id: "proof".into(),
            artifact_digest: "b".repeat(64),
            expected_installed_digest: "c".repeat(64),
            operation: Operation::Canary,
            installation_ref: None,
        };
        let mut runtime = super::super::artifacts::LinuxRuntimeSnapshot {
            arch: std::env::consts::ARCH.into(),
            interpreter: "/lib64/ld-linux-x86-64.so.2".into(),
            kernel_release: "fixture".into(),
            files: Default::default(),
        };
        let original = canary_key(&draft, &runtime, "1").unwrap();
        let request = canary_command(std::path::Path::new("/private-scratch"), &runtime).unwrap();
        let argv = request.argv.unwrap();
        assert_eq!(argv[0], "/usr/bin/bwrap");
        assert!(argv.iter().any(|a| a == "--unshare-all"));
        assert!(argv.iter().any(|a| a == "--clearenv"));
        assert!(argv.iter().any(|a| a == "--inhibit-cache"));
        assert!(argv.ends_with(&["/refact-lsp".into(), "--version".into()]));
        assert_eq!(argv.iter().filter(|a| a.as_str() == "--ro-bind").count(), 1);
        assert!(!argv.iter().any(|a| a == "--bind" || a == "/not-mounted"));
        assert!(request.timeout.unwrap() <= std::time::Duration::from_secs(15));
        runtime.kernel_release.push('2');
        assert_ne!(original, canary_key(&draft, &runtime, "1").unwrap());
        runtime.files.insert(
            "/lib/libc.so.6".into(),
            super::super::artifacts::LinuxRuntimeFile {
                canonical_path: "/usr/lib/libc.so.6".into(),
                digest: "d".repeat(64),
            },
        );
        let with_file = canary_key(&draft, &runtime, "1").unwrap();
        runtime.files.get_mut("/lib/libc.so.6").unwrap().digest = "e".repeat(64);
        assert_ne!(with_file, canary_key(&draft, &runtime, "1").unwrap());
        assert_ne!(with_file, canary_key(&draft, &runtime, "2").unwrap());
        let canary = fixture_spec(&draft, "same-reference");
        let mut install = draft.clone();
        install.operation = Operation::Install;
        assert!(validate_spec(&canary, &install).is_err());
        assert_ne!(
            crate::buddy::action_execution::action_spec_hash(&canary, &canary.parameter_ref),
            crate::buddy::action_execution::action_spec_hash(
                &fixture_spec(&install, "same-reference"),
                &canary.parameter_ref
            )
        );
    }

    /// Run only with parent-authorized explicit ignored-test invocation. The
    /// evaluator fixtures are isolated, not production evidence. Never installs.
    #[tokio::test]
    #[ignore = "explicit confined actual-built-artifact acceptance only"]
    async fn actual_built_engine_confined_canary_acceptance() {
        use crate::buddy::outcome_runtime::OutcomeRuntime;
        use refact_buddy_core::operator_types::{WorkerIdentity, ActionExecutionState};
        use crate::buddy::action_execution::ActionResult;
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::env::var("REFACT_NATIVE_CANARY_ACCEPTANCE").as_deref(),
            Ok("authorized")
        );
        let path = std::path::PathBuf::from(
            std::env::var("REFACT_NATIVE_CANARY_IMAGE")
                .expect("explicit current built refact-lsp path"),
        );
        assert!(path.is_absolute());
        let bytes = std::fs::read(path).unwrap();
        assert!(
            bytes.len() as u64 <= super::super::artifacts::MAX_BYTES,
            "canary image is {} bytes, above the production staged-artifact cap {}; supply a stripped copy of the actual built refact-lsp",
            bytes.len(),
            super::super::artifacts::MAX_BYTES
        );
        super::super::artifacts::linux_runtime_snapshot(&bytes, std::env::consts::ARCH)
            .expect("actual engine runtime inventory must pass before preparing a canary grant");
        let dir = tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let target = dir.path().join("never-installed");
        let project = ProjectIdentity {
            project_id: "confined-canary-acceptance".into(),
            canonical_root: dir.path().to_string_lossy().into_owned(),
        };
        let runtime = OutcomeRuntime::open(
            dir.path(),
            project,
            WorkerIdentity {
                worker_id: "scratch".into(),
                engine_instance_id: "scratch".into(),
                epoch: 1,
            },
        )
        .unwrap();
        let (install, install_spec) = seed_image(
            &runtime,
            &target,
            bytes,
            env!("CARGO_PKG_VERSION").into(),
            true,
        )
        .await;
        let d = install.clone();
        let s = install_spec.clone();
        let p = target.clone();
        assert!(
            runtime
                .with_store(move |store| preflight(store, &s, &d, &p))
                .await
                .is_err(),
            "independent fixture alone cannot authorize dynamic Install"
        );
        let mut draft = install.clone();
        draft.operation = Operation::Canary;
        let (request, native, fixture) = prepare_fixture(&runtime, &target, &draft, "canary").await;
        let result = fixture.execute(&runtime, &request).await;
        assert!(
            matches!(result, ActionResult::Finished(ref r) if r.execution.status == ActionExecutionState::Completed),
            "{result:?}"
        );
        assert_eq!(
            retained_installation(&runtime, &native)
                .await
                .unwrap()
                .value
                .phase,
            "canary_passed"
        );
        assert!(matches!(
            fixture.execute(&runtime, &request).await,
            ActionResult::Duplicate(_)
        ));
        let p = target.clone();
        runtime
            .with_store(move |store| preflight(store, &install_spec, &install, &p))
            .await
            .unwrap();
        assert_eq!(
            std::fs::read(target).unwrap(),
            b"scratch known-good executable bytes"
        );
    }

    fn fake_canary_output(
        status: crate::exec::ExecStatus,
        text: &str,
        found: bool,
        truncated: bool,
    ) -> CanaryOutput {
        use crate::exec::*;
        let meta = ExecProcessMeta::new(ExecMode::Foreground, "confined fixture".into());
        let id = meta.process_id.clone();
        let mut read = ExecReadResult::not_found(id.clone(), 0);
        read.found = found;
        read.is_truncated = truncated;
        read.chunks.push(ExecOutputChunk {
            process_id: id.clone(),
            seq: 1,
            stream: ExecOutputStream::Stdout,
            text: text.into(),
            timestamp_ms: 1,
        });
        (id, ExecProcessSnapshot::new(meta).with_status(status), read)
    }

    #[test]
    fn canary_output_requires_exact_complete_success() {
        use crate::exec::ExecStatus;
        let success = ExecStatus::Exited {
            exit_code: Some(0),
        };
        assert!(canary_output_passes(
            &fake_canary_output(success.clone(), "refact-lsp 1\n", true, false),
            "1"
        ));
        for status in [
            ExecStatus::TimedOut,
            ExecStatus::Killed,
            ExecStatus::Running,
            ExecStatus::Failed {
                message: "spawn".into(),
            },
            ExecStatus::Exited {
                exit_code: Some(1),
            },
        ] {
            assert!(!canary_output_passes(
                &fake_canary_output(status, "refact-lsp 1\n", true, false),
                "1"
            ));
        }
        for (text, found, truncated) in [
            ("refact-lsp 1\n", false, false),
            ("refact-lsp 1\n", true, true),
            ("refact-lsp 2\n", true, false),
            ("refact-lsp 1\n\n", true, false),
            ("", true, false),
        ] {
            assert!(!canary_output_passes(
                &fake_canary_output(success.clone(), text, found, truncated),
                "1"
            ));
        }
    }

    struct FailedCanaryExecutor {
        result: std::sync::Mutex<Option<Result<CanaryOutput, String>>>,
        shutdown_on_spawn: bool,
        cancel_on_drop: bool,
        started: tokio::sync::Notify,
        drained: tokio::sync::Notify,
        scratch: std::sync::Mutex<Option<std::path::PathBuf>>,
    }
    #[async_trait::async_trait]
    impl CanaryExecutor for FailedCanaryExecutor {
        async fn execute(
            &self,
            command: crate::exec::ExecSpawnRequest,
            shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
            cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
        ) -> Result<CanaryOutput, String> {
            use std::sync::atomic::Ordering;
            let root = command.cwd.unwrap();
            assert!(root.join("refact-lsp").is_file());
            assert_eq!(command.argv.unwrap()[0], "/usr/bin/bwrap");
            *self.scratch.lock().unwrap() = Some(root.clone());
            self.started.notify_one();
            if self.shutdown_on_spawn {
                shutdown.store(true, Ordering::SeqCst);
            }
            if self.cancel_on_drop {
                while !cancelled.load(Ordering::SeqCst) {
                    tokio::task::yield_now().await;
                }
                assert!(
                    root.exists(),
                    "owned executor keeps scratch until cancellation drains"
                );
                self.drained.notify_one();
                return Err("cancelled and drained".into());
            }
            self.result.lock().unwrap().take().unwrap()
        }
    }

    #[tokio::test]
    async fn claimed_canary_failures_never_save_proof_or_allow_replay() {
        use crate::buddy::outcome_runtime::OutcomeRuntime;
        use refact_buddy_core::operator_types::WorkerIdentity;
        use crate::exec::ExecStatus;
        use std::sync::{Arc, Mutex, atomic::AtomicBool};
        for case in [
            "timeout",
            "killed",
            "missing-output",
            "truncated",
            "wrong-version",
            "spawn-error",
            "shutdown-race",
            "cancel-drop",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let project = ProjectIdentity {
                project_id: "fake-claimed-canary".into(),
                canonical_root: dir.path().to_string_lossy().into_owned(),
            };
            let runtime = OutcomeRuntime::open(
                dir.path(),
                project.clone(),
                WorkerIdentity {
                    worker_id: "scratch".into(),
                    engine_instance_id: "scratch".into(),
                    epoch: 1,
                },
            )
            .unwrap();
            let target = dir.path().join("never-installed");
            let (mut draft, _) = seed(&runtime, &target).await;
            draft.operation = Operation::Canary;
            let spec = fixture_spec(&draft, "canary");
            let key = crate::buddy::action_execution::action_spec_hash(&spec, &spec.parameter_ref);
            let record = InstallationRecord {
                schema_version: 1,
                project: project.clone(),
                draft: draft.clone(),
                grant_id: "canary".into(),
                phase: "canary_executing_outcome_unknown".into(),
                installed_digest: None,
                running_digest: None,
                previous_digest: draft.expected_installed_digest.clone(),
                detail: "fixture: begins after durable claim, never executes artifact".into(),
                restart_intent_generation: None,
                observed_daemon_generation: None,
            };
            let artifact = runtime
                .with_store(move |s| {
                    super::super::artifacts::installation_image(
                        s,
                        &super::super::candidate(s, "candidate")?.value,
                    )
                    .map(|x| x.0)
                    .map_err(conflict)
                })
                .await
                .unwrap();
            let snapshot = super::super::artifacts::LinuxRuntimeSnapshot {
                arch: std::env::consts::ARCH.into(),
                interpreter: if std::env::consts::ARCH == "aarch64" {
                    "/lib/ld-linux-aarch64.so.1"
                } else {
                    "/lib64/ld-linux-x86-64.so.2"
                }
                .into(),
                kernel_release: "fake-no-host-inventory".into(),
                files: Default::default(),
            };
            let proof_key = canary_key(&draft, &snapshot, &artifact.attested_version).unwrap();
            let claim = record.clone();
            let k = key.clone();
            let reserved_proof_key = proof_key.clone();
            runtime
                .with_store(move |s| {
                    // Mirror the native claim's capacity ownership, including its
                    // proof slot, before entering the already-claimed executor.
                    s.reserve_native_documents(
                        "native_installation",
                        &k,
                        &[
                            (NAMESPACE, k.clone(), 5),
                            (CANARY_NAMESPACE, reserved_proof_key, 1),
                        ],
                    )?;
                    s.put_reserved_document("native_installation", &k, NAMESPACE, &k, &claim, None)
                })
                .await
                .unwrap();
            let status = match case {
                "timeout" => ExecStatus::TimedOut,
                "killed" => ExecStatus::Killed,
                _ => ExecStatus::Exited {
                    exit_code: Some(0),
                },
            };
            let text = if case == "wrong-version" {
                "refact-lsp wrong\n"
            } else {
                "refact-lsp scratch-v1\n"
            };
            let result = if case == "spawn-error" {
                Err("fixture spawn rejected".into())
            } else {
                Ok(fake_canary_output(
                    status,
                    text,
                    case != "missing-output",
                    case == "truncated",
                ))
            };
            let executor = Arc::new(FailedCanaryExecutor {
                result: Mutex::new(Some(result)),
                shutdown_on_spawn: case == "shutdown-race",
                cancel_on_drop: case == "cancel-drop",
                started: Default::default(),
                drained: Default::default(),
                scratch: Mutex::new(None),
            });
            let native = NativeRequest {
                prepared_hash: key,
                grant_id: "canary".into(),
            };
            let adapter = Arc::new(DaemonInstallationAdapter {
                registry: executor.clone(),
                shutdown: Arc::new(AtomicBool::new(false)),
                runtime: runtime.clone(),
                request: native.clone(),
                executable: target.clone(),
                spec,
            });
            let runner = adapter.clone();
            let original = record.clone();
            let task = tokio::spawn(async move {
                runner
                    .execute_claimed_canary(original, artifact, elf(), snapshot)
                    .await
            });
            if case == "cancel-drop" {
                executor.started.notified().await;
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
                tokio::time::timeout(
                    std::time::Duration::from_secs(2),
                    executor.drained.notified(),
                )
                .await
                .unwrap();
            } else {
                let result = task.await.unwrap();
                if case == "spawn-error" {
                    assert!(result.is_err());
                } else {
                    assert_eq!(result.unwrap().phase, "canary_failed", "{case}");
                }
            }
            // No failure may mint compatibility proof or permit a second execution.
            assert!(runtime.with_store(move |s| s.get_document::<CanaryProof>(CANARY_NAMESPACE, &proof_key)).await.unwrap().is_none(), "{case}");
            let retained = retained_installation(&runtime, &native)
                .await
                .unwrap()
                .value;
            assert_ne!(retained.phase, "canary_passed");
            assert!(adapter.run_canary(retained).await.is_err(), "{case}");
            assert_eq!(
                std::fs::read(&target).unwrap(),
                b"scratch known-good executable bytes"
            );
            let scratch = executor.scratch.lock().unwrap().clone().unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                while scratch.exists() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
        }
    }

    const FIXTURE_LOADER_BYTES: &[u8] = b"fixture loader bytes, never executable";
    const CHANGED_FIXTURE_LOADER_BYTES: &[u8] = b"changed fixture loader bytes, never executable";

    #[derive(Debug, PartialEq, Eq)]
    enum CanaryIoEvent {
        Snapshot(super::super::artifacts::LinuxRuntimeSnapshot),
        Copy(super::super::artifacts::LinuxRuntimeFile),
        Executed,
    }

    struct FixtureCanaryExecutor {
        image: Vec<u8>,
        snapshot: super::super::artifacts::LinuxRuntimeSnapshot,
        events: std::sync::Mutex<Vec<CanaryIoEvent>>,
        result: std::sync::Mutex<Option<Result<CanaryOutput, String>>>,
        change_after_execution: bool,
        completed: std::sync::atomic::AtomicBool,
        calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait::async_trait]
    impl CanaryExecutor for FixtureCanaryExecutor {
        fn runtime_snapshot(
            &self,
            bytes: &[u8],
            arch: &str,
        ) -> Result<
            super::super::artifacts::LinuxRuntimeSnapshot,
            crate::buddy::resources::ResourceError,
        > {
            use sha2::{Digest, Sha256};
            assert_eq!(bytes, self.image);
            assert_eq!(arch, self.snapshot.arch);
            let mut snapshot = self.snapshot.clone();
            if self.completed.load(std::sync::atomic::Ordering::SeqCst)
                && self.change_after_execution
            {
                snapshot
                    .files
                    .get_mut(&snapshot.interpreter)
                    .unwrap()
                    .digest = hex::encode(Sha256::digest(CHANGED_FIXTURE_LOADER_BYTES));
            }
            self.events
                .lock()
                .unwrap()
                .push(CanaryIoEvent::Snapshot(snapshot.clone()));
            Ok(snapshot)
        }

        fn runtime_file_bytes(
            &self,
            file: &super::super::artifacts::LinuxRuntimeFile,
        ) -> Result<Vec<u8>, crate::buddy::resources::ResourceError> {
            if Some(file) != self.snapshot.files.get(&self.snapshot.interpreter) {
                return Err(crate::buddy::resources::ResourceError::ValidationFailed(
                    "fixture runtime fingerprint changed".into(),
                ));
            }
            self.events
                .lock()
                .unwrap()
                .push(CanaryIoEvent::Copy(file.clone()));
            Ok(FIXTURE_LOADER_BYTES.to_vec())
        }

        async fn execute(
            &self,
            command: crate::exec::ExecSpawnRequest,
            shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
            cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
        ) -> Result<CanaryOutput, String> {
            use std::sync::atomic::Ordering;
            self.calls.fetch_add(1, Ordering::SeqCst);
            let root = command.cwd.as_ref().unwrap();
            assert_eq!(std::fs::read(root.join("refact-lsp")).unwrap(), self.image);
            assert_eq!(
                std::fs::read(root.join(self.snapshot.interpreter.strip_prefix('/').unwrap()))
                    .unwrap(),
                FIXTURE_LOADER_BYTES
            );
            let roots = super::super::artifacts::linux_library_roots(&self.snapshot.arch)
                .unwrap()
                .join(":");
            assert_eq!(
                command.argv.unwrap(),
                [
                    "/usr/bin/bwrap",
                    "--unshare-all",
                    "--die-with-parent",
                    "--new-session",
                    "--cap-drop",
                    "ALL",
                    "--clearenv",
                    "--ro-bind",
                    root.to_str().unwrap(),
                    "/",
                    "--proc",
                    "/proc",
                    "--dev",
                    "/dev",
                    "--tmpfs",
                    "/tmp",
                    "--dir",
                    "/home",
                    "--setenv",
                    "HOME",
                    "/home",
                    "--setenv",
                    "LD_BIND_NOW",
                    "1",
                    "--setenv",
                    "LANG",
                    "C",
                    "--chdir",
                    "/",
                    "--",
                    &self.snapshot.interpreter,
                    "--inhibit-cache",
                    "--library-path",
                    &roots,
                    "/refact-lsp",
                    "--version",
                ]
                .map(String::from)
            );
            assert!(!shutdown.load(Ordering::SeqCst));
            assert!(!cancelled.load(Ordering::SeqCst));
            self.events.lock().unwrap().push(CanaryIoEvent::Executed);
            self.completed.store(true, Ordering::SeqCst);
            self.result.lock().unwrap().take().unwrap()
        }
    }

    fn dynamic_elf() -> Vec<u8> {
        let mut bytes = elf();
        bytes.resize(512, 0);
        bytes[16] = 3;
        bytes[56] = 3;
        bytes[96..104].copy_from_slice(&512u64.to_le_bytes());
        bytes[104..112].copy_from_slice(&512u64.to_le_bytes());
        bytes[120..176].fill(0);
        bytes[120] = 2;
        bytes[128..136].copy_from_slice(&256u64.to_le_bytes());
        bytes[152..160].copy_from_slice(&48u64.to_le_bytes());
        bytes[256..264].copy_from_slice(&5u64.to_le_bytes());
        bytes[264..272].copy_from_slice(&(0x400000u64 + 352).to_le_bytes());
        bytes[272..280].copy_from_slice(&10u64.to_le_bytes());
        bytes[280..288].copy_from_slice(&1u64.to_le_bytes());
        let loader = if std::env::consts::ARCH == "aarch64" {
            "/lib/ld-linux-aarch64.so.1\0"
        } else {
            "/lib64/ld-linux-x86-64.so.2\0"
        };
        bytes[176] = 3;
        bytes[184..192].copy_from_slice(&400u64.to_le_bytes());
        bytes[208..216].copy_from_slice(&(loader.len() as u64).to_le_bytes());
        bytes[400..400 + loader.len()].copy_from_slice(loader.as_bytes());
        bytes
    }

    struct FullCanaryFixture {
        _directory: tempfile::TempDir,
        draft: InstallationDraft,
        request: crate::buddy::action_execution::ActionRequest,
        executor: std::sync::Arc<FixtureCanaryExecutor>,
        adapter: DaemonInstallationAdapter,
    }

    async fn full_canary_fixture(
        result: Result<CanaryOutput, String>,
        change_after_execution: bool,
    ) -> FullCanaryFixture {
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicUsize},
        };
        use crate::buddy::outcome_runtime::OutcomeRuntime;
        use refact_buddy_core::operator_types::WorkerIdentity;
        use std::os::unix::fs::PermissionsExt;
        use sha2::{Digest, Sha256};
        let dir = tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let project = ProjectIdentity {
            project_id: "failed-canary".into(),
            canonical_root: dir.path().to_string_lossy().into_owned(),
        };
        let runtime = OutcomeRuntime::open(
            dir.path(),
            project,
            WorkerIdentity {
                worker_id: "scratch".into(),
                engine_instance_id: "scratch".into(),
                epoch: 1,
            },
        )
        .unwrap();
        let target = dir.path().join("never-installed");
        let bytes = dynamic_elf();
        let (mut draft, _) = seed_image(&runtime, &target, bytes.clone(), "1".into(), true).await;
        draft.operation = Operation::Canary;
        let (request, native, _) =
            prepare_fixture(&runtime, &target, &draft, "failed-canary").await;
        let interpreter = if std::env::consts::ARCH == "aarch64" {
            "/lib/ld-linux-aarch64.so.1"
        } else {
            "/lib64/ld-linux-x86-64.so.2"
        };
        let executor = Arc::new(FixtureCanaryExecutor {
            image: bytes,
            snapshot: super::super::artifacts::LinuxRuntimeSnapshot {
                arch: std::env::consts::ARCH.into(),
                interpreter: interpreter.into(),
                kernel_release: "fixture-only-no-host-inventory".into(),
                files: std::collections::BTreeMap::from([(
                    interpreter.into(),
                    super::super::artifacts::LinuxRuntimeFile {
                        canonical_path: "/usr/lib/refact-fixture-loader".into(),
                        digest: hex::encode(Sha256::digest(FIXTURE_LOADER_BYTES)),
                    },
                )]),
            },
            events: Mutex::new(Vec::new()),
            result: Mutex::new(Some(result)),
            change_after_execution,
            completed: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
        });
        let adapter = DaemonInstallationAdapter {
            registry: executor.clone(),
            shutdown: Arc::new(AtomicBool::new(false)),
            runtime: runtime.clone(),
            request: native.clone(),
            executable: target.clone(),
            spec: request.spec.clone(),
        };
        FullCanaryFixture {
            _directory: dir,
            draft,
            request,
            executor,
            adapter,
        }
    }

    #[tokio::test]
    async fn canary_failed_executor_never_publishes_proof_or_replays() {
        use std::sync::atomic::Ordering;
        use crate::buddy::resources::AsyncResourceAdapter;
        let FullCanaryFixture {
            _directory,
            draft,
            request,
            executor,
            adapter,
        } = full_canary_fixture(Err("deterministic spawn failure".into()), false).await;
        let runtime = &adapter.runtime;
        let native = &adapter.request;
        let result = adapter.apply(&request.spec).await;
        assert!(result.is_err(), "{result:?}");
        assert_eq!(
            executor.calls.load(Ordering::SeqCst),
            1,
            "apply={result:?}; (claim_phase, any_canary_proof)={:?}; fixture IO prerequisite recheck (not real host trust or original error chain)={:?}",
            {
                let key = native.prepared_hash.clone();
                runtime
                    .with_store(move |store| {
                        Ok((
                            store
                                .get_document::<InstallationRecord>(NAMESPACE, &key)
                                .map(|claim| claim.map(|claim| claim.value.phase)),
                            store
                                .list_documents::<CanaryProof>(CANARY_NAMESPACE, None, 1)
                                .map(|proofs| !proofs.entries.is_empty()),
                        ))
                    })
                    .await
            },
            executor.runtime_snapshot(&executor.image, std::env::consts::ARCH)
                .map(|_| ())
        );
        assert_eq!(
            *executor.events.lock().unwrap(),
            [
                CanaryIoEvent::Snapshot(executor.snapshot.clone()),
                CanaryIoEvent::Snapshot(executor.snapshot.clone()),
                CanaryIoEvent::Snapshot(executor.snapshot.clone()),
                CanaryIoEvent::Copy(
                    executor.snapshot.files[&executor.snapshot.interpreter].clone()
                ),
                CanaryIoEvent::Executed,
            ]
        );
        assert_eq!(
            retained_installation(runtime, native)
                .await
                .unwrap()
                .value
                .phase,
            "canary_executing_outcome_unknown"
        );
        assert!(runtime
            .with_store(|store| store.list_documents::<CanaryProof>(CANARY_NAMESPACE, None, 1))
            .await
            .unwrap()
            .entries
            .is_empty());
        assert!(!adapter
            .verify(&request.spec, &draft.artifact_digest)
            .await
            .unwrap());
        assert!(adapter.apply(&request.spec).await.is_err());
        assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            std::fs::read(&adapter.executable).unwrap(),
            b"scratch known-good executable bytes"
        );
    }

    #[tokio::test]
    async fn canary_full_apply_rechecks_inventory_before_publishing_proof() {
        use crate::buddy::resources::AsyncResourceAdapter;
        use std::sync::atomic::Ordering;
        use sha2::{Digest, Sha256};
        for changed in [false, true] {
            let output = fake_canary_output(
                crate::exec::ExecStatus::Exited {
                    exit_code: Some(0),
                },
                "refact-lsp 1\n",
                true,
                false,
            );
            assert!(canary_output_passes(&output, "1"));
            let process_id = output.0.to_string();
            let FullCanaryFixture {
                _directory,
                draft,
                request,
                executor,
                adapter,
            } = full_canary_fixture(Ok(output), changed).await;
            let snapshot = executor.snapshot.clone();
            let proof_key = canary_key(&draft, &snapshot, "1").unwrap();
            let receipt = adapter.apply(&request.spec).await.unwrap();
            assert_eq!(receipt.expected_post_version, draft.artifact_digest);
            let key = proof_key.clone();
            let proof = adapter
                .runtime
                .with_store(move |s| s.get_document::<CanaryProof>(CANARY_NAMESPACE, &key))
                .await
                .unwrap();
            if changed {
                assert!(
                    proof.is_none(),
                    "changed runtime must not publish a successful canary proof"
                );
                assert!(adapter
                    .runtime
                    .with_store(|s| s.list_documents::<CanaryProof>(CANARY_NAMESPACE, None, 1))
                    .await
                    .unwrap()
                    .entries
                    .is_empty());
            } else {
                let proof = proof.unwrap().value;
                assert_eq!(proof.schema_version, 1);
                assert_eq!(proof.project, draft.project);
                assert_eq!(proof.binding, proof_key);
                assert_eq!(proof.prepared_hash, adapter.request.prepared_hash);
                assert_eq!(proof.runtime, snapshot);
                assert_eq!(proof.process_id, process_id);
                assert_eq!(proof.output_digest, digest(&"refact-lsp 1\n").unwrap());
                assert!(proof.passed);
            }
            let retained = retained_installation(&adapter.runtime, &adapter.request)
                .await
                .unwrap();
            assert_eq!(
                retained.value.phase,
                if changed {
                    "canary_failed"
                } else {
                    "canary_passed"
                }
            );
            assert_eq!(retained.value.draft, draft);
            assert_eq!(retained.value.grant_id, adapter.request.grant_id);
            assert_eq!(
                adapter
                    .verify(&request.spec, &draft.artifact_digest)
                    .await
                    .unwrap(),
                !changed
            );
            let mut final_snapshot = snapshot.clone();
            if changed {
                final_snapshot
                    .files
                    .get_mut(&snapshot.interpreter)
                    .unwrap()
                    .digest = hex::encode(Sha256::digest(CHANGED_FIXTURE_LOADER_BYTES));
                assert_ne!(final_snapshot, snapshot);
            }
            assert_eq!(
                *executor.events.lock().unwrap(),
                [
                    CanaryIoEvent::Snapshot(snapshot.clone()),
                    CanaryIoEvent::Snapshot(snapshot.clone()),
                    CanaryIoEvent::Snapshot(snapshot.clone()),
                    CanaryIoEvent::Copy(snapshot.files[&snapshot.interpreter].clone()),
                    CanaryIoEvent::Executed,
                    CanaryIoEvent::Snapshot(final_snapshot),
                ]
            );
            assert!(adapter.apply(&request.spec).await.is_err());
            assert_eq!(executor.calls.load(Ordering::SeqCst), 1);
            assert_eq!(
                std::fs::read(&adapter.executable).unwrap(),
                b"scratch known-good executable bytes"
            );
        }
    }

    #[test]
    fn installation_record_strict_store_contract_and_phase_cas() {
        let dir = tempfile::tempdir().unwrap();
        let project = ProjectIdentity {
            project_id: "installation-record-contract".into(),
            canonical_root: dir.path().to_string_lossy().into_owned(),
        };
        let mut store = OperatorStore::open(dir.path(), &project).unwrap();
        let mut record = InstallationRecord {
            schema_version: 1,
            project: project.clone(),
            draft: InstallationDraft {
                project,
                candidate_id: "candidate".into(),
                candidate_digest: "a".repeat(64),
                proof_id: "proof".into(),
                artifact_digest: "b".repeat(64),
                expected_installed_digest: "c".repeat(64),
                operation: Operation::Install,
                installation_ref: None,
            },
            grant_id: "grant".into(),
            phase: "claimed_outcome_unknown".into(),
            installed_digest: None,
            running_digest: None,
            previous_digest: "c".repeat(64),
            detail: "scratch record contract; no native effect".into(),
            restart_intent_generation: None,
            observed_daemon_generation: None,
        };
        let wire = serde_json::to_value(&record).unwrap();
        assert_eq!(wire["schema_version"], 1);
        assert_eq!(
            wire["project"],
            serde_json::to_value(store.identity()).unwrap()
        );
        let mut missing = wire.clone();
        missing.as_object_mut().unwrap().remove("schema_version");
        assert!(serde_json::from_value::<InstallationRecord>(missing.clone()).is_err());
        assert!(matches!(
            store.put_document(NAMESPACE, "missing-version", &missing, None),
            Err(OperatorStoreError::InvalidInput(
                "unsupported document schema version"
            ))
        ));
        let mut invalid = wire.clone();
        invalid["schema_version"] = serde_json::json!(2);
        assert!(store
            .put_document(NAMESPACE, "future-version", &invalid, None)
            .is_err());
        invalid = wire;
        invalid["project"]["project_id"] = serde_json::json!("foreign");
        assert!(matches!(
            store.put_document(NAMESPACE, "foreign", &invalid, None),
            Err(OperatorStoreError::ProjectMismatch)
        ));
        // Every phase written by consume or the daemon retains the strict envelope.
        let claim = store
            .put_document(NAMESPACE, "record", &record, None)
            .unwrap();
        let mut revision = claim.revision;
        for (operation, phase) in [
            (Operation::Install, "installed"),
            (Operation::Recover, "recovered_installed"),
            (Operation::Restart, "restart_requested_running_unverified"),
            (Operation::Restart, "restart_dispatch_outcome_unknown"),
            (
                Operation::Restart,
                "daemon_running_verified_workers_unverified",
            ),
        ] {
            record.draft.operation = operation;
            record.phase = phase.into();
            let next = store
                .put_document(NAMESPACE, "record", &record, Some(revision))
                .unwrap();
            assert!(matches!(
                store.put_document(NAMESPACE, "record", &claim.value, Some(revision)),
                Err(OperatorStoreError::StaleRevision { .. })
            ));
            revision = next.revision;
            let observed = store
                .get_document::<InstallationRecord>(NAMESPACE, "record")
                .unwrap()
                .unwrap();
            assert_eq!(observed.value.schema_version, 1);
            assert_eq!(observed.value.phase, phase);
        }
    }

    #[tokio::test]
    async fn scratch_action_service_installs_recovers_and_never_replays_after_reopen() {
        use crate::buddy::action_execution::ActionResult;
        use crate::buddy::outcome_runtime::OutcomeRuntime;
        use refact_buddy_core::operator_types::{WorkerIdentity, ActionExecutionState};
        use std::os::unix::fs::PermissionsExt;
        // Installation requires a non-shared parent even with a group-writable umask.
        let dir = tempfile::Builder::new()
            .permissions(std::fs::Permissions::from_mode(0o700))
            .tempdir()
            .unwrap();
        let target = dir.path().canonicalize().unwrap().join("scratch-engine");
        let project = ProjectIdentity {
            project_id: "native-install-fixture".into(),
            canonical_root: dir.path().to_string_lossy().into_owned(),
        };
        let worker = WorkerIdentity {
            worker_id: "scratch-daemon".into(),
            engine_instance_id: "scratch-generation".into(),
            epoch: 1,
        };
        let runtime = OutcomeRuntime::open(dir.path(), project.clone(), worker.clone()).unwrap();
        let (draft, _) = seed(&runtime, &target).await;
        let (request, native, fixture) =
            prepare_fixture(&runtime, &target, &draft, "install").await;
        let result = fixture.execute(&runtime, &request).await;
        let retained = retained_installation(&runtime, &native).await;
        assert!(
            matches!(result, ActionResult::Finished(ref r) if r.execution.status == ActionExecutionState::Completed),
            "{result:?}; retained installation: {retained:?}"
        );
        let retained = retained.unwrap();
        assert_eq!(retained.revision, 2, "claim precedes terminal observation");
        assert_eq!(retained.value.schema_version, 1);
        assert_eq!(retained.value.phase, "installed");
        assert_eq!(
            retained.value.installed_digest.as_deref(),
            Some(draft.artifact_digest.as_str())
        );
        assert_eq!(std::fs::read(&target).unwrap(), elf());
        assert!(matches!(
            fixture.execute(&runtime, &request).await,
            ActionResult::Duplicate(_)
        ));
        let mut recovery = draft.clone();
        recovery.operation = Operation::Recover;
        recovery.expected_installed_digest = draft.artifact_digest.clone();
        recovery.installation_ref = Some(native.prepared_hash.clone());
        let (recover_request, recover_native, recover_fixture) =
            prepare_fixture(&runtime, &target, &recovery, "recover").await;
        let result = recover_fixture.execute(&runtime, &recover_request).await;
        let retained = retained_installation(&runtime, &recover_native).await;
        assert!(
            matches!(result, ActionResult::Finished(ref r) if r.execution.status == ActionExecutionState::Completed),
            "{result:?}; retained recovery: {retained:?}"
        );
        let retained = retained.unwrap();
        assert_eq!(retained.revision, 2);
        assert_eq!(retained.value.schema_version, 1);
        assert_eq!(retained.value.phase, "recovered_installed");
        assert_eq!(
            retained.value.installed_digest.as_deref(),
            Some(draft.expected_installed_digest.as_str())
        );
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"scratch known-good executable bytes"
        );
        // A new exact user grant may request the recovered known-good image,
        // never the still-staged candidate. No daemon is launched by this test.
        let mut restart = draft.clone();
        restart.operation = Operation::Restart;
        restart.installation_ref = Some(recover_native.prepared_hash.clone());
        let (restart_request, restart_native, restart_fixture) =
            prepare_fixture(&runtime, &target, &restart, "restart-recovered").await;
        let result = restart_fixture.execute(&runtime, &restart_request).await;
        assert!(
            matches!(result, ActionResult::Finished(ref r) if r.execution.status == ActionExecutionState::Partial),
            "{result:?}"
        );
        let mut observation = retained_installation(&runtime, &restart_native)
            .await
            .unwrap()
            .value;
        assert_eq!(
            observation.installed_digest.as_deref(),
            Some(draft.expected_installed_digest.as_str())
        );
        assert!(!reconcile_running(
            &mut observation,
            "new-generation",
            &draft.expected_installed_digest
        ));
        observation.phase = "restart_dispatch_outcome_unknown".into();
        observation.restart_intent_generation = Some("old-generation".into());
        assert!(!reconcile_running(
            &mut observation,
            "old-generation",
            &draft.expected_installed_digest
        ));
        assert!(!reconcile_running(
            &mut observation,
            "new-generation",
            &draft.artifact_digest
        ));
        assert!(!reconcile_running(
            &mut observation,
            "new-generation",
            &"f".repeat(64)
        ));
        assert!(reconcile_running(
            &mut observation,
            "new-generation",
            &draft.expected_installed_digest
        ));
        assert!(!reconcile_running(
            &mut observation,
            "another-generation",
            &draft.expected_installed_digest
        ));
        assert_eq!(
            observation.phase,
            "daemon_running_verified_workers_unverified"
        );
        assert!(matches!(
            restart_fixture.execute(&runtime, &restart_request).await,
            ActionResult::Duplicate(_)
        ));
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"scratch known-good executable bytes"
        );
        drop(restart_fixture);
        drop(fixture);
        drop(recover_fixture);
        drop(runtime);
        let runtime = OutcomeRuntime::open(dir.path(), project, worker).unwrap();
        let path = target.clone();
        let n = native.clone();
        let record = runtime
            .with_store(move |store| consume(store, &n, &path))
            .await
            .unwrap();
        assert_eq!(record.phase, "installed");
        // The historical install outcome is returned but the recovered file is not overwritten.
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"scratch known-good executable bytes"
        );
        let mut catalog = crate::buddy::resource_execution::ResourceCatalog::default();
        catalog
            .register(
                ACTION.into(),
                std::sync::Arc::new(DaemonInstallationAdapter {
                    registry: std::sync::Arc::new(crate::exec::ExecRegistry::new()),
                    shutdown: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    runtime: runtime.clone(),
                    request: native,
                    executable: target.clone(),
                    spec: request.spec.clone(),
                }),
            )
            .unwrap();
        let grant = crate::buddy::resource_execution::load_grant(&runtime, "install".into())
            .await
            .unwrap()
            .unwrap();
        let service = catalog
            .service(runtime.clone(), std::sync::Arc::new(grant))
            .unwrap();
        assert!(matches!(
            service.execute_async(&request).await,
            ActionResult::Duplicate(_)
        ));
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"scratch known-good executable bytes"
        );
    }

    pub(crate) fn elf() -> Vec<u8> {
        // Complete ET_EXEC image with one executable PT_LOAD and an architecture
        // specific exit(0) entry. The fixture is installed as bytes, NEVER executed.
        let code: &[u8] = if std::env::consts::ARCH == "aarch64" {
            &[
                0x00, 0x00, 0x80, 0xd2, 0xa8, 0x0b, 0x80, 0xd2, 0x01, 0x00, 0x00, 0xd4,
            ]
        } else {
            &[0xb8, 60, 0, 0, 0, 0x31, 0xff, 0x0f, 0x05]
        };
        let mut bytes = vec![0u8; 120];
        bytes.extend_from_slice(code);
        let size = bytes.len() as u64;
        bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        bytes[16] = 2;
        bytes[18] = if std::env::consts::ARCH == "aarch64" {
            183
        } else {
            62
        };
        bytes[20] = 1;
        bytes[24..32].copy_from_slice(&(0x400000u64 + 120).to_le_bytes());
        bytes[32] = 64;
        bytes[52] = 64;
        bytes[54] = 56;
        bytes[56] = 1;
        bytes[64] = 1;
        bytes[68] = 5;
        bytes[80..88].copy_from_slice(&0x400000u64.to_le_bytes());
        bytes[96..104].copy_from_slice(&size.to_le_bytes());
        bytes[104..112].copy_from_slice(&size.to_le_bytes());
        bytes[112..120].copy_from_slice(&4096u64.to_le_bytes());
        bytes
    }

    #[test]
    fn static_gate_rejects_malformed_headers_segments_and_entry() {
        for (offset, replacement) in [
            (0, vec![0]),
            (4, vec![1]),
            (5, vec![2]),
            (16, vec![3]),
            (18, vec![0]),
            (52, vec![0]),
            (54, vec![0]),
            (56, vec![0, 0]),
            (32, u64::MAX.to_le_bytes().to_vec()),
            (68, vec![4]),
            (24, 0u64.to_le_bytes().to_vec()),
            (96, u64::MAX.to_le_bytes().to_vec()),
            (104, 0u64.to_le_bytes().to_vec()),
            (112, 3u64.to_le_bytes().to_vec()),
        ] {
            let mut bytes = elf();
            bytes[offset..offset + replacement.len()].copy_from_slice(&replacement);
            assert!(
                static_host_image(&bytes, std::env::consts::ARCH).is_err(),
                "offset {offset}"
            );
        }
        for length in 0..elf().len() {
            assert!(static_host_image(&elf()[..length], std::env::consts::ARCH).is_err());
        }
    }
    #[test]
    fn transport_cannot_supply_executable_version_or_authority() {
        for field in [
            "path",
            "bytes",
            "version",
            "running_digest",
            "user_confirmed",
        ] {
            let mut request =
                serde_json::json!({"prepared_hash":"a".repeat(64), "grant_id":"grant"});
            request[field] = serde_json::json!("untrusted");
            assert!(serde_json::from_value::<NativeRequest>(request).is_err());
        }
    }
}

pub struct InstallationAdapter {
    runtime: crate::buddy::outcome_runtime::OutcomeRuntime,
    drafts: std::sync::Arc<dyn crate::buddy::resources::DraftRegistry<InstallationDraft>>,
}
impl InstallationAdapter {
    pub fn new(
        runtime: crate::buddy::outcome_runtime::OutcomeRuntime,
        drafts: std::sync::Arc<dyn crate::buddy::resources::DraftRegistry<InstallationDraft>>,
    ) -> Self {
        Self {
            runtime,
            drafts,
        }
    }
}
#[async_trait::async_trait]
impl crate::buddy::resources::AsyncResourceAdapter for InstallationAdapter {
    fn adapter_id(&self) -> &str {
        ADAPTER
    }
    fn capabilities(&self) -> crate::buddy::resources::ResourceCapabilities {
        crate::buddy::resources::ResourceCapabilities {
            preview: true,
            apply: false,
            verify: false,
            recover: false,
        }
    }
    async fn preview(
        &self,
        spec: &ActionSpec,
    ) -> Result<crate::buddy::resources::ResourcePreview, crate::buddy::resources::ResourceError>
    {
        use crate::buddy::resources::*;
        if spec.project != self.runtime.project() {
            return Err(ResourceError::ProjectScopeMismatch);
        }
        let draft = self.drafts.get(
            spec.parameter_ref
                .as_deref()
                .ok_or(ResourceError::ParameterReferenceMissing)?,
        )?;
        validate_spec(spec, &draft).map_err(|e| ResourceError::ValidationFailed(e.to_string()))?;
        Ok(ResourcePreview { resource_id: spec.resource.resource_id.clone(), expected_version: spec.expected_version.clone(), proposed_version: draft.artifact_digest, reload_needed: draft.operation != Operation::Canary, diff: Some(if draft.operation == Operation::Canary { "Execute-only confined refact-lsp --version loader/startup check. Separate exact Basic-user grant required; cannot install, restart or recover." } else { "Daemon-native Linux image lifecycle. Dynamic image requires exact retained native canary and independent evaluation plus a fresh install grant; no worker install or self-restart." }.into()) })
    }
    async fn apply(
        &self,
        _: &ActionSpec,
    ) -> Result<crate::buddy::resources::ResourceApplyReceipt, crate::buddy::resources::ResourceError>
    {
        Err(crate::buddy::resources::ResourceError::UnsupportedResource)
    }
    async fn verify(
        &self,
        _: &ActionSpec,
        _: &str,
    ) -> Result<bool, crate::buddy::resources::ResourceError> {
        Ok(false)
    }
    async fn recover(
        &self,
        _: &ActionSpec,
        _: &crate::buddy::resources::RecoveryDescriptor,
    ) -> Result<crate::buddy::resources::ResourceApplyState, crate::buddy::resources::ResourceError>
    {
        Err(crate::buddy::resources::ResourceError::UnsupportedResource)
    }
}
