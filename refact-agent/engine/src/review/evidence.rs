//! Genuine execution results, with deliberately conservative input attribution.
//! A post-hoc saved snapshot is never represented as the process's input snapshot.
use std::{path::PathBuf, sync::Arc, collections::BTreeMap};
use tokio::sync::Mutex;
use refact_review_core::*;
use crate::{
    at_commands::at_commands::AtCommandsContext,
    exec::{ExecProcessId, ExecStatus},
};

pub(crate) async fn capture_saved_state(
    root: PathBuf,
    checkout: PathBuf,
) -> std::result::Result<
    (
        Candidate,
        Vec<refact_worktrees::review::SnapshotFile>,
        BlobRef,
    ),
    String,
> {
    super::service::with_store(root, move |store| {
        let repo = git2::Repository::open(&checkout)?;
        let head = repo.head()?.peel_to_commit()?.id().to_string();
        let snapshot = store.capture(
            &checkout,
            CaptureRequest {
                base_oid: head.clone(),
                head_oid: head,
                policy: CapturePolicy {
                    staged: true,
                    working: true,
                    untracked: true,
                },
                origin: Origin::Agent,
            },
        )?;
        let files = snapshot
            .files
            .iter()
            .filter_map(|f| f.head.as_ref().map(|b| (f, b)))
            .map(|(f, b)| {
                Ok(refact_worktrees::review::SnapshotFile {
                    path: f.path.clone(),
                    bytes: store.read_blob(b)?,
                    mode: f
                        .head_mode
                        .ok_or_else(|| Error::Invalid("Missing saved file mode".into()))?,
                })
            })
            .collect::<refact_review_core::Result<Vec<_>>>()?;
        // The manifest retains path/mode/deletion and blob references; bytes live in core CAS.
        let artifact = store.put_blob(&serde_json::to_vec(&snapshot)?)?;
        Ok((snapshot, files, artifact))
    })
    .await
    .map_err(|e| e.to_string())
}

/// Deliberate before-execution capture. This receipt establishes saved local bytes,
/// not complete dependency/environment identity or exclusion of concurrent writers.
pub async fn prepare_execution(
    ccx: Arc<Mutex<AtCommandsContext>>,
    request_key: &str,
) -> std::result::Result<Material, String> {
    if request_key.trim().is_empty() || request_key.len() > 256 {
        return Err("Invalid execution preparation key".into());
    }
    let (root, review) = super::runs::bound_review(ccx.clone(), None).await?;
    let candidate = super::runs::inspected_candidate(ccx.clone(), &review).await?;
    let (app, chat, agent, worktree) = {
        let c = ccx.lock().await;
        (
            c.app.clone(),
            c.chat_id.clone(),
            c.background_agent_id.clone(),
            c.execution_scope_worktree()
                .ok_or("Review checkout missing")?,
        )
    };
    let before_ids = app
        .gcx
        .exec_registry
        .list(crate::exec::ExecProcessFilter {
            chat_id: Some(chat.clone()),
            ..Default::default()
        })
        .await;
    let (snapshot, _, artifact) = capture_saved_state(root.clone(), worktree.root.clone()).await?;
    let mut inputs = BTreeMap::new();
    inputs.insert(
        "prepared_at_ms".into(),
        chrono::Utc::now().timestamp_millis().to_string(),
    );
    inputs.insert("prepared_chat".into(), chat);
    inputs.insert("request_key".into(), request_key.into());
    inputs.insert(
        "processes_already_registered".into(),
        serde_json::to_string(
            &before_ids
                .iter()
                .map(|p| p.meta.process_id.as_str())
                .collect::<Vec<_>>(),
        )
        .map_err(|e| e.to_string())?,
    );
    inputs.insert("saved_state_before".into(), snapshot.content_digest.clone());
    let material = Material { id: uuid::Uuid::new_v4().to_string(), kind: MaterialKind::Investigation, candidate_id: candidate, agent_id: agent, description: "Deliberate pre-execution saved local state. External dependencies, effective environment and concurrent mutation exclusion unknown".into(), content_digest: snapshot.content_digest, input_digests: inputs, environment: None, worktree_id: Some(worktree.id), worktree_root: Some(worktree.root.to_string_lossy().into()), artifacts: vec![artifact] };
    super::runs::apply(
        root,
        &review.id,
        Operation::AddMaterial {
            material: material.clone(),
        },
    )
    .await?;
    Ok(material)
}

fn validate_process(
    snapshot: &crate::exec::ExecProcessSnapshot,
    chat: &str,
    root: &std::path::Path,
) -> std::result::Result<(), String> {
    if snapshot.meta.owner.chat_id.as_deref() != Some(chat) {
        return Err("Execution evidence requires the exact process owning chat".into());
    }
    if !snapshot.status.is_terminal() || snapshot.meta.started_at_ms.is_none() {
        return Err("Execution has no completed runtime result".into());
    }
    let cwd = snapshot
        .meta
        .cwd
        .as_ref()
        .ok_or("Process workdir unavailable")?;
    let cwd = dunce::canonicalize(cwd).map_err(|e| e.to_string())?;
    let root = dunce::canonicalize(root).map_err(|e| e.to_string())?;
    if !cwd.starts_with(root) {
        return Err("Execution did not run in this retained review checkout".into());
    }
    Ok(())
}

/// Only `process_id` identifies execution: callers cannot supply a command, exit code,
/// environment or passing claim. Current saved state is retained separately and
/// attribution stays Unknown because legacy ExecRegistry has no before-input receipt.
pub async fn record_process_evidence(
    ccx: Arc<Mutex<AtCommandsContext>>,
    review_id: &str,
    process_id: &str,
    summary: &str,
    retained_kind: MaterialKind,
) -> std::result::Result<Review, String> {
    record_prepared_process_evidence(ccx, review_id, process_id, summary, retained_kind, None).await
}

pub async fn record_prepared_process_evidence(
    ccx: Arc<Mutex<AtCommandsContext>>,
    review_id: &str,
    process_id: &str,
    summary: &str,
    retained_kind: MaterialKind,
    prepared_material_id: Option<&str>,
) -> std::result::Result<Review, String> {
    if retained_kind == MaterialKind::Imported {
        return Err("Executed investigations cannot become immutable imported evidence".into());
    }
    if summary.trim().is_empty() || summary.len() > 32768 {
        return Err("Evidence summary required (maximum 32768 bytes)".into());
    }
    let (root, review) = super::runs::bound_review(ccx.clone(), Some(review_id)).await?;
    let candidate = super::runs::inspected_candidate(ccx.clone(), &review).await?;
    let (app, chat, agent, worktree) = {
        let c = ccx.lock().await;
        (
            c.app.clone(),
            c.chat_id.clone(),
            c.background_agent_id.clone(),
            c.execution_scope_worktree()
                .ok_or("Review checkout unavailable")?,
        )
    };
    let id = ExecProcessId(process_id.into());
    let process = app
        .gcx
        .exec_registry
        .get(&id)
        .await
        .ok_or("Registry-owned process not found")?;
    validate_process(&process, &chat, &worktree.root)?;
    let output = app.gcx.exec_registry.read(&id, 0, None).await;
    let (snapshot, _, manifest) = capture_saved_state(root.clone(), worktree.root.clone()).await?;
    let mut inputs = BTreeMap::new();
    inputs.insert(
        "saved_state_after_recording".into(),
        snapshot.content_digest.clone(),
    );
    inputs.insert("process_id".into(), process_id.into());
    inputs.insert(
        "process_input_before".into(),
        "unknown: not captured by execution registry".into(),
    );
    inputs.insert(
        "process_input_after".into(),
        "unknown: saved state captured at recording, not process completion".into(),
    );
    let mut applicability = Applicability::Unknown;
    if let Some(prepared_id) = prepared_material_id {
        let prepared = review
            .materials
            .iter()
            .find(|m| m.id == prepared_id)
            .ok_or("Prepared execution material not found")?;
        let prepared_at = prepared
            .input_digests
            .get("prepared_at_ms")
            .and_then(|t| t.parse::<u64>().ok())
            .ok_or("Not an execution preparation receipt")?;
        let old_ids: Vec<String> = serde_json::from_str(
            prepared
                .input_digests
                .get("processes_already_registered")
                .ok_or("Preparation process fence missing")?,
        )
        .map_err(|e| e.to_string())?;
        if prepared.candidate_id != candidate
            || prepared.agent_id != agent
            || prepared.worktree_id.as_deref() != Some(&worktree.id)
            || prepared
                .input_digests
                .get("prepared_chat")
                .map(String::as_str)
                != Some(&chat)
            || prepared_at > process.meta.created_at_ms
            || old_ids.iter().any(|p| p == process_id)
        {
            return Err("Process does not match prepared review input identity/time fence".into());
        }
        inputs.insert("prepared_material_id".into(), prepared.id.clone());
        inputs.insert("saved_state_before".into(), prepared.content_digest.clone());
        // Equality proves only endpoints. Unobserved transient changes and unknown
        // external inputs remain unknown, never promote this to Current.
        if prepared.content_digest != snapshot.content_digest {
            applicability = Applicability::NeedsRevalidation;
        }
    }
    let stdout_bytes = output
        .chunks
        .iter()
        .filter(|c| c.stream != crate::exec::ExecOutputStream::Stderr)
        .map(|c| c.text.as_str())
        .collect::<String>()
        .into_bytes();
    let stderr_bytes = output
        .chunks
        .iter()
        .filter(|c| c.stream == crate::exec::ExecOutputStream::Stderr)
        .map(|c| c.text.as_str())
        .collect::<String>()
        .into_bytes();
    let (stdout, stderr) = super::service::with_store(root.clone(), move |store| {
        Ok((
            store.put_blob(&stdout_bytes)?,
            store.put_blob(&stderr_bytes)?,
        ))
    })
    .await
    .map_err(|e| e.to_string())?;
    let receipt_bytes = serde_json::to_vec(&serde_json::json!({"process":process,"output":output,"basis":"registry result; before/after input and effective environment unknown"})).map_err(|e| e.to_string())?;
    let receipt =
        super::service::with_store(root.clone(), move |store| store.put_blob(&receipt_bytes))
            .await
            .map_err(|e| e.to_string())?;
    let material = Material { id: uuid::Uuid::new_v4().to_string(), kind: retained_kind, candidate_id: candidate.clone(), agent_id: agent.clone(), description: "Saved local investigation/repair at evidence recording; not proven process input and not immutable imported comparison".into(), content_digest: snapshot.content_digest, input_digests: inputs, environment: None, worktree_id: Some(worktree.id), worktree_root: Some(worktree.root.to_string_lossy().into()), artifacts: vec![manifest, receipt.clone()] };
    let material_id = material.id.clone();
    super::runs::apply(
        root.clone(),
        review_id,
        Operation::AddMaterial {
            material,
        },
    )
    .await?;
    let evidence = Evidence { id: uuid::Uuid::new_v4().to_string(), candidate_id: candidate, material_id: Some(material_id), run_id: agent.and_then(|a| review.runs.iter().find(|r| r.agent_id == a).map(|r| r.id.clone())), source: EvidenceSource::Executed { command: vec![process.meta.command], workdir: process.meta.cwd.unwrap().to_string_lossy().into(), exit_code: match process.status { ExecStatus::Exited { exit_code } => exit_code, _ => None }, stdout: Some(stdout), stderr: Some(stderr) }, summary: format!("{}\nAttribution: runtime-owned result only. Required inputs and effective environment are unknown; a passing experiment does not prove the original candidate. Output may be truncated; inspect retained receipt.", summary.trim()), applicability, created_at_ms: chrono::Utc::now().timestamp_millis() };
    super::runs::apply(
        root,
        review_id,
        Operation::AddEvidence {
            evidence,
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn execution_attribution_requires_exact_owner_and_checkout() {
        let dir = tempfile::tempdir().unwrap();
        let foreign = tempfile::tempdir().unwrap();
        let mut meta =
            crate::exec::ExecProcessMeta::new(crate::exec::ExecMode::Foreground, "true".into())
                .with_chat_id("owner")
                .with_cwd(dir.path().into());
        meta.started_at_ms = Some(1);
        let process = crate::exec::ExecProcessSnapshot::new(meta).with_status(ExecStatus::Exited {
            exit_code: Some(0),
        });
        assert!(validate_process(&process, "owner", dir.path()).is_ok());
        assert!(validate_process(&process, "foreign", dir.path()).is_err());
        assert!(validate_process(&process, "owner", foreign.path()).is_err());
    }
}
