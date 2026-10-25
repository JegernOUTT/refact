//! Native engineering source proof for supervised workers.
//!
//! The goal verifier (`crate::chat::goal_verifier`) judges chat history; it
//! cannot see whether the worker's isolated worktree really holds the change
//! the worker described. This module binds exact source content to the
//! verifier execution itself: the isolated worktree is observed through the
//! live native edit scope immediately BEFORE the verifier stream starts and
//! again immediately AFTER the reply arrives. Only when both observations are
//! byte-identical, the live authority is still valid after the stream, every
//! changed path is one of the exact mandate targets, and the finalized native
//! verdict for that very reply is MET does an immutable `EngineeringProof`
//! become available to the parent supervisor.
//!
//! Nothing here marks a goal MET, edits goal criteria, or trusts worker text
//! (`edited_files`, `diff_summary`). Source is observed with the canonical
//! native `WorktreeService` diff (committed HEAD range + staged + unstaged +
//! untracked, symlink-aware) plus direct target-file content hashing; no
//! shell or arbitrary process is spawned for fingerprinting.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use chrono::Utc;
use refact_buddy_core::operator_types::{ProjectIdentity, WorkerIdentity};
use refact_chat_api::{GoalCriterion, GoalSnapshot, GoalStatus};
use serde::{Deserialize, Serialize};

use super::supervision_authority::{
    is_engineering, native_edit_scope_for_chat, worker_target_files, MandateRecord,
    NativeSnapshotScope, SUPERVISED_WORKTREE_KIND,
};
use crate::worktrees::service::WorktreeService;

pub const SCHEMA_VERSION: u16 = 1;
/// Bounded in-process records: one pending capture per verifier attempt and
/// one authorized proof per child chat.
const MAX_PENDING: usize = 64;
const MAX_AUTHORIZED: usize = 64;
/// A pending capture that never finalizes (verifier abort, crash of the
/// applying task) is pruned after this many milliseconds.
const PENDING_TTL_MS: u64 = 10 * 60 * 1000;
/// Hard cap on target-file bytes hashed per observation.
const MAX_TARGET_BYTES: u64 = 32 * 1024 * 1024;
/// Patch limit passed to the native diff; a truncated patch fails closed.
const MAX_PATCH_BYTES: usize = 8 * 1024 * 1024;

/// Immutable native observation of one isolated worktree at one instant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceObservation {
    pub worktree_id: String,
    pub worktree_root: String,
    pub source_workspace_root: String,
    /// Commit the isolated worktree was created from (mandate baseline).
    pub baseline_head: String,
    /// `HEAD` of the isolated worktree at observation time.
    pub current_head: String,
    /// sha256 of the canonical native diff response (files + patch text).
    pub patch_digest: String,
    /// sha256 over `path\0status\0len\0bytes` of every exact target in
    /// mandate order; missing files hash as absent, symlinks are rejected.
    pub content_digest: String,
    /// Exact mandate target set, in mandate order.
    pub target_files: Vec<String>,
    /// Paths the native diff reports as changed relative to the baseline
    /// (committed, staged, unstaged or untracked). Always ⊆ `target_files`.
    pub changed_files: Vec<String>,
}

/// Serializable, immutable proof that the native verifier's MET verdict was
/// produced while the isolated source held exactly `source`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngineeringProof {
    pub schema_version: u16,
    pub project: ProjectIdentity,
    pub worker: WorkerIdentity,
    pub mandate_id: String,
    pub owner_epoch: u64,
    pub child_chat_id: String,
    pub goal_version: u32,
    pub goal_content: String,
    pub goal_criteria: Vec<GoalCriterion>,
    /// sha256 hex of the exact verifier reply text recorded in the attempt.
    pub verifier_reply_digest: String,
    /// `GoalAttempt::at_ms` of the finalized MET attempt this proof binds.
    pub verifier_attempt_at_ms: u64,
    pub observed_before_at: String,
    pub observed_after_at: String,
    pub source: SourceObservation,
}

/// Comparison of a retained proof against the current isolated source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceComparison {
    pub retained: SourceObservation,
    pub current: SourceObservation,
    pub unchanged: bool,
    pub compared_at: String,
}

/// Native-only capture taken before the verifier stream. Not Clone, not
/// serializable: it must be handed back to `bind_after_stream`.
pub struct PendingCapture {
    scope: NativeSnapshotScope,
    cache_dir: PathBuf,
    goal_version: u32,
    goal_content: String,
    goal_criteria: Vec<GoalCriterion>,
    observed_before_at: String,
    before: SourceObservation,
}

#[derive(Clone)]
struct PendingRecord {
    nonce: String,
    registered_at_ms: u64,
    authority: refact_chat_api::SupervisionCommandGuard,
    goal_version: u32,
    goal_content: String,
    goal_criteria: Vec<GoalCriterion>,
    verifier_reply_digest: String,
    observed_before_at: String,
    observed_after_at: String,
    source: SourceObservation,
}

struct AuthorizedRecord {
    registered_at_ms: u64,
    proof: EngineeringProof,
}

struct Registry {
    pending: Vec<(String, PendingRecord)>,
    authorized: HashMap<String, AuthorizedRecord>,
}

static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();

fn registry() -> &'static Mutex<Registry> {
    REGISTRY.get_or_init(|| {
        Mutex::new(Registry {
            pending: Vec::new(),
            authorized: HashMap::new(),
        })
    })
}

fn now_ms() -> u64 {
    Utc::now().timestamp_millis().max(0) as u64
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

fn prune(registry: &mut Registry, now: u64) {
    registry
        .pending
        .retain(|(_, record)| now.saturating_sub(record.registered_at_ms) < PENDING_TTL_MS);
    registry
        .authorized
        .retain(|_, record| now.saturating_sub(record.registered_at_ms) < PENDING_TTL_MS);
}

/// Hash the exact target set directly from the isolated root. Every target
/// must resolve inside `root` without traversing any symlink; missing files
/// are recorded as absent so a deletion changes the digest deterministically.
fn hash_targets(root: &Path, targets: &[String]) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    let mut budget = MAX_TARGET_BYTES;
    for target in targets {
        if Path::new(target).is_absolute()
            || Path::new(target).components().any(|part| {
                matches!(
                    part,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(format!("target {target} is not repository-relative"));
        }
        let path = root.join(target);
        let mut cursor = root.to_path_buf();
        for part in Path::new(target).components() {
            cursor.push(part);
            match std::fs::symlink_metadata(&cursor) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    return Err(format!("target {target} traverses a symlink"));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                Err(error) => return Err(format!("target {target} cannot be inspected: {error}")),
            }
        }
        hasher.update(target.as_bytes());
        hasher.update([0]);
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_file() => {
                if meta.len() > budget {
                    return Err("target content exceeds proof byte bound".into());
                }
                // Metadata is only a preflight. Read through a hard cap as the
                // file may grow or be replaced after that check.
                let mut file = open_regular_nofollow(&path)?;
                let mut bytes = Vec::with_capacity(meta.len().min(budget) as usize);
                file.by_ref()
                    .take(budget.saturating_add(1))
                    .read_to_end(&mut bytes)
                    .map_err(|e| e.to_string())?;
                let after = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
                if !after.is_file() || after.file_type().is_symlink() || bytes.len() as u64 > budget
                {
                    return Err(
                        "target content exceeds proof byte bound or changed while hashing".into(),
                    );
                }
                if after.len() != bytes.len() as u64 || after.len() != meta.len() {
                    return Err("target changed while hashing".into());
                }
                budget -= bytes.len() as u64;
                hasher.update(b"file");
                hasher.update([0]);
                hasher.update((bytes.len() as u64).to_le_bytes());
                hasher.update([0]);
                hasher.update(&bytes);
            }
            Ok(_) => return Err(format!("target {target} is not a regular file")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                hasher.update(b"absent");
            }
            Err(error) => return Err(format!("target {target} cannot be read: {error}")),
        }
        hasher.update([0]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(unix)]
fn open_regular_nofollow(path: &Path) -> Result<std::fs::File, String> {
    use std::os::unix::fs::OpenOptionsExt;
    // O_NOFOLLOW prevents a replacement between symlink_metadata and open.
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| e.to_string())
}

#[cfg(not(unix))]
fn open_regular_nofollow(path: &Path) -> Result<std::fs::File, String> {
    std::fs::File::open(path).map_err(|e| e.to_string())
}

/// Observe the isolated worktree registered for `scope` through the native
/// worktree service. Fails closed on any drift from the validated scope.
async fn observe(
    cache_dir: &Path,
    scope: &NativeSnapshotScope,
) -> Result<SourceObservation, String> {
    observe_root(
        cache_dir,
        scope.worktree_root(),
        Some(scope.chat_id()),
        scope.target_files(),
    )
    .await
}

async fn observe_root(
    cache_dir: &Path,
    worktree_root: &Path,
    expected_chat: Option<&str>,
    targets: &[String],
) -> Result<SourceObservation, String> {
    let (id, source_workspace_root) = registered_worktree(cache_dir, worktree_root, expected_chat)?;
    let service =
        WorktreeService::new_async(cache_dir.to_path_buf(), source_workspace_root).await?;
    service.validate_registered_worktree_checkout(&id).await?;
    let view = service.get_worktree(&id).await?;
    let root = dunce::canonicalize(&view.meta.root)
        .map(|p| dunce::simplified(&p).to_path_buf())
        .map_err(|e| format!("isolated worktree root unavailable: {e}"))?;
    if root != worktree_root
        || view.meta.kind != SUPERVISED_WORKTREE_KIND
        || view.meta.source_workspace_root != service.source_workspace_root()
        || view.meta.root == service.source_workspace_root()
        || expected_chat.is_some_and(|chat| {
            !view
                .references
                .iter()
                .any(|r| r.chat_id.as_deref() == Some(chat))
        })
        || !view.status.path_exists
        || !view.status.is_git_worktree
        || view.status.conflicted
        || view.status.error.is_some()
    {
        return Err("isolated worktree identity, kind, or health drifted".into());
    }
    let baseline_head = view
        .meta
        .base_commit
        .clone()
        .filter(|c| !c.trim().is_empty())
        .ok_or("isolated worktree has no recorded baseline commit")?;
    let current_head = view
        .status
        .head_commit
        .clone()
        .filter(|c| !c.trim().is_empty())
        .ok_or("isolated worktree HEAD unavailable")?;
    let diff = service
        .diff_worktree_with_limit(&id, MAX_PATCH_BYTES)
        .await?;
    if diff.patch_truncated {
        return Err("isolated worktree diff exceeds the proof patch bound".into());
    }
    let mut changed: Vec<String> = Vec::new();
    for file in &diff.files {
        if !targets.contains(&file.path) {
            return Err(format!(
                "isolated worktree changed {} outside the exact mandate targets",
                file.path
            ));
        }
        if !changed.contains(&file.path) {
            changed.push(file.path.clone());
        }
    }
    let patch_digest = sha256_hex(&serde_json::to_vec(&diff).map_err(|e| e.to_string())?);
    let hash_root = root.clone();
    let hash_targets_owned = targets.to_vec();
    let content_digest =
        tokio::task::spawn_blocking(move || hash_targets(&hash_root, &hash_targets_owned))
            .await
            .map_err(|e| e.to_string())??;
    Ok(SourceObservation {
        worktree_id: id,
        worktree_root: root.to_string_lossy().into_owned(),
        source_workspace_root: service
            .source_workspace_root()
            .to_string_lossy()
            .into_owned(),
        baseline_head,
        current_head,
        patch_digest,
        content_digest,
        target_files: targets.to_vec(),
        changed_files: changed,
    })
}

/// Resolve a registered worktree through the actual WorktreeService registry.
/// Worktree checkouts do not own a `.refact/worktree.json`; the cache registry
/// is the runtime's authoritative metadata store.
fn registered_worktree(
    cache_dir: &Path,
    root: &Path,
    expected_chat: Option<&str>,
) -> Result<(String, PathBuf), String> {
    const MAX_REGISTRY_BYTES: u64 = 4 * 1024 * 1024;
    const MAX_REGISTRIES: usize = 128;
    let canonical_root = dunce::canonicalize(root)
        .map(|p| dunce::simplified(&p).to_path_buf())
        .map_err(|e| format!("isolated worktree root unavailable: {e}"))?;
    let registries = cache_dir.join("worktrees");
    let entries = std::fs::read_dir(&registries)
        .map_err(|e| format!("worktree registry directory unavailable: {e}"))?;
    for (index, entry) in entries.enumerate() {
        if index >= MAX_REGISTRIES {
            return Err("worktree registry directory exceeds proof entry bound".into());
        }
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            continue;
        }
        let index = entry.path().join("index.json");
        let mut file = match std::fs::File::open(&index) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("worktree registry unavailable: {error}")),
        };
        let mut raw = Vec::new();
        file.by_ref()
            .take(MAX_REGISTRY_BYTES + 1)
            .read_to_end(&mut raw)
            .map_err(|e| e.to_string())?;
        if raw.len() as u64 > MAX_REGISTRY_BYTES {
            return Err("worktree registry exceeds proof byte bound".into());
        }
        let registry: crate::worktrees::types::WorktreeRegistry =
            serde_json::from_slice(&raw).map_err(|e| format!("worktree registry invalid: {e}"))?;
        let source_workspace_root = registry.source_workspace_root;
        for record in registry.records {
            let registered_root = dunce::canonicalize(&record.meta.root)
                .map(|p| dunce::simplified(&p).to_path_buf())
                .map_err(|e| format!("registered worktree root unavailable: {e}"))?;
            if registered_root == canonical_root {
                crate::worktrees::service::validate_worktree_id(&record.meta.id)?;
                if expected_chat.is_some_and(|chat| {
                    !record
                        .references
                        .iter()
                        .any(|r| r.chat_id.as_deref() == Some(chat))
                }) {
                    return Err("isolated worktree is not registered to this chat".into());
                }
                return Ok((record.meta.id, source_workspace_root));
            }
        }
    }
    Err("isolated worktree is not registered in the native registry".into())
}

/// Snapshot of the session fields the capture needs, taken under the session
/// lock by the caller and released before any awaiting.
pub struct SessionFacts {
    pub chat_id: String,
    pub goal_version: u32,
    pub goal_content: String,
    pub goal_criteria: Vec<GoalCriterion>,
}

/// Native capture immediately BEFORE the verifier stream. Returns `None`
/// when `chat_id` has no live engineering scope (unsupervised chats and
/// read-only supervised workers are unchanged). Returns `Err` when a live
/// engineering scope exists but the source cannot be observed; the caller
/// must then run the verifier without a proof, never fabricate one.
pub async fn capture_before_stream(
    cache_dir: PathBuf,
    facts: &SessionFacts,
) -> Result<Option<PendingCapture>, String> {
    let Some(edit_scope) = native_edit_scope_for_chat(&facts.chat_id) else {
        return Ok(None);
    };
    if edit_scope.target_files().is_empty() {
        return Ok(None);
    }
    // A fresh verifier run supersedes every undelivered capture for this
    // child. This prevents a delayed/replayed identical reply from promoting
    // evidence captured for an earlier stream after an abort or retry.
    if let Ok(mut registry) = registry().lock() {
        prune(&mut registry, now_ms());
        registry.pending.retain(|(chat, _)| chat != &facts.chat_id);
        registry.authorized.remove(&facts.chat_id);
    }
    let scope = edit_scope.validated_snapshot_scope(&facts.chat_id)?;
    let observed_before_at = Utc::now().to_rfc3339();
    let before = observe(&cache_dir, &scope).await?;
    Ok(Some(PendingCapture {
        scope,
        cache_dir,
        goal_version: facts.goal_version,
        goal_content: facts.goal_content.clone(),
        goal_criteria: facts.goal_criteria.clone(),
        observed_before_at,
        before,
    }))
}

/// Native binding immediately AFTER the verifier reply. Re-validates the live
/// authority, re-observes the source, and registers a private pending record
/// keyed by the exact reply only when nothing changed. Any failure simply
/// leaves no pending record: the verdict is untouched but no proof can exist.
pub async fn bind_after_stream(
    capture: PendingCapture,
    verifier_reply: &str,
) -> Result<(), String> {
    capture.scope.revalidate()?;
    let after = observe(&capture.cache_dir, &capture.scope).await?;
    if after != capture.before {
        return Err("isolated source changed during native verification".into());
    }
    capture.scope.revalidate()?;
    let record = PendingRecord {
        nonce: uuid::Uuid::new_v4().to_string(),
        registered_at_ms: now_ms(),
        authority: capture.scope.authority().clone(),
        goal_version: capture.goal_version,
        goal_content: capture.goal_content,
        goal_criteria: capture.goal_criteria,
        verifier_reply_digest: sha256_hex(verifier_reply.as_bytes()),
        observed_before_at: capture.observed_before_at,
        observed_after_at: Utc::now().to_rfc3339(),
        source: after,
    };
    let mut registry = registry()
        .lock()
        .map_err(|_| "proof registry unavailable")?;
    prune(&mut registry, record.registered_at_ms);
    registry.pending.retain(|(chat, existing)| {
        chat != capture.scope.chat_id()
            || existing.goal_version != record.goal_version
            || existing.goal_content != record.goal_content
            || existing.goal_criteria != record.goal_criteria
            || existing.verifier_reply_digest != record.verifier_reply_digest
    });
    if registry.pending.len() >= MAX_PENDING {
        registry.pending.remove(0);
    }
    registry
        .pending
        .push((capture.scope.chat_id().to_string(), record));
    Ok(())
}

/// Consume the pending record for the exact reply the native verdict just
/// finalized as MET and promote it to the authorized proof for `chat_id`.
/// Returns whether a proof was authorized. The goal must already carry the
/// attempt: `verifier_reply` and `attempt_at_ms` are taken from that attempt.
pub fn finalize_met(
    chat_id: &str,
    goal_version: u32,
    goal_content: &str,
    goal_criteria: &[GoalCriterion],
    verifier_reply: &str,
    attempt_at_ms: u64,
) -> bool {
    let digest = sha256_hex(verifier_reply.as_bytes());
    let Ok(mut registry) = registry().lock() else {
        return false;
    };
    prune(&mut registry, now_ms());
    let Some(index) = registry.pending.iter().position(|(chat, record)| {
        chat == chat_id
            && record.goal_version == goal_version
            && record.goal_content == goal_content
            && record.goal_criteria == goal_criteria
            && record.verifier_reply_digest == digest
    }) else {
        return false;
    };
    let (_, record) = registry.pending.remove(index);
    // The authority must still be live at finalization: a proof is never
    // minted for a revoked or expired mandate even if the stream succeeded.
    if super::supervision_authority::with_command_guard(Some(&record.authority), chat_id, || Ok(()))
        .is_err()
    {
        return false;
    }
    let proof = EngineeringProof {
        schema_version: SCHEMA_VERSION,
        project: ProjectIdentity {
            project_id: record.authority.project_id.clone(),
            canonical_root: record.source.source_workspace_root.clone(),
        },
        worker: WorkerIdentity {
            worker_id: record.authority.worker_id.clone(),
            engine_instance_id: record.authority.engine_instance_id.clone(),
            epoch: record.authority.worker_epoch,
        },
        mandate_id: record.authority.mandate_id.clone(),
        owner_epoch: record.authority.owner_epoch,
        child_chat_id: chat_id.to_string(),
        goal_version,
        goal_content: record.goal_content,
        goal_criteria: record.goal_criteria,
        verifier_reply_digest: digest,
        verifier_attempt_at_ms: attempt_at_ms,
        observed_before_at: record.observed_before_at,
        observed_after_at: record.observed_after_at,
        source: record.source,
    };
    let _ = record.nonce;
    if registry.authorized.len() >= MAX_AUTHORIZED && !registry.authorized.contains_key(chat_id) {
        if let Some(oldest) = registry
            .authorized
            .iter()
            .min_by_key(|(_, record)| record.registered_at_ms)
            .map(|(chat, _)| chat.clone())
        {
            registry.authorized.remove(&oldest);
        }
    }
    registry.authorized.insert(
        chat_id.to_string(),
        AuthorizedRecord {
            registered_at_ms: now_ms(),
            proof,
        },
    );
    true
}

/// Drop any pending record for `chat_id` matching `verifier_reply` (UNMET,
/// superseded, or discarded verdicts). A previously authorized proof for an
/// earlier goal version is also cleared: the newer verdict supersedes it.
pub fn discard_pending(chat_id: &str, verifier_reply: &str) {
    let digest = sha256_hex(verifier_reply.as_bytes());
    if let Ok(mut registry) = registry().lock() {
        registry
            .pending
            .retain(|(chat, record)| !(chat == chat_id && record.verifier_reply_digest == digest));
        registry.authorized.remove(chat_id);
    }
}

/// Cloned authorized proof for `child_chat_id`, if the native verifier
/// authorized one. Never consumes: the parent acknowledges after durable
/// persistence with `acknowledge_authorized`.
pub fn get_authorized_proof(child_chat_id: &str) -> Option<EngineeringProof> {
    let mut registry = registry().lock().ok()?;
    prune(&mut registry, now_ms());
    registry
        .authorized
        .get(child_chat_id)
        .map(|record| record.proof.clone())
}

/// Remove the authorized proof once the parent has durably retained it.
pub fn acknowledge_authorized(child_chat_id: &str) {
    if let Ok(mut registry) = registry().lock() {
        registry.authorized.remove(child_chat_id);
    }
}

/// Store-side validation of a retained proof against the exact admitted
/// mandate, child chat, and native goal snapshot. Pure and deterministic;
/// never consults live authority so it works after stop/reopen.
pub fn validate_retained(
    proof: &EngineeringProof,
    record: &MandateRecord,
    child_chat_id: &str,
    goal: &GoalSnapshot,
) -> Result<(), String> {
    if proof.schema_version != SCHEMA_VERSION {
        return Err("engineering proof schema unsupported".into());
    }
    if !is_engineering(record) {
        return Err("mandate holds no engineering grant".into());
    }
    let targets = worker_target_files(record)?;
    let admitted_epoch = record.mandate.owner_epoch;
    let epoch_ok = proof.owner_epoch == admitted_epoch
        || (record.mandate.revoked_at.is_some()
            && proof.owner_epoch.checked_add(1) == Some(admitted_epoch));
    if proof.mandate_id != record.mandate.mandate_id
        || proof.project != record.project
        || proof.source.source_workspace_root != record.project.canonical_root
        || proof.worker != record.worker
        || !epoch_ok
        || proof.child_chat_id != child_chat_id
    {
        return Err("engineering proof identity does not match the admitted mandate".into());
    }
    let hex = |value: &str, length: usize| {
        value.len() == length && value.bytes().all(|b| b.is_ascii_hexdigit())
    };
    let before = chrono::DateTime::parse_from_rfc3339(&proof.observed_before_at)
        .map_err(|_| "invalid source observation time")?;
    let after = chrono::DateTime::parse_from_rfc3339(&proof.observed_after_at)
        .map_err(|_| "invalid source observation time")?;
    if before > after
        || after.timestamp_millis() < 0
        || after.timestamp_millis() as u64 > proof.verifier_attempt_at_ms
    {
        return Err("invalid native verification observation order".into());
    }
    if proof.source.target_files != targets
        || proof
            .source
            .changed_files
            .iter()
            .any(|path| !targets.contains(path))
        || !hex(&proof.source.baseline_head, 40)
        || !hex(&proof.source.current_head, 40)
        || !hex(&proof.source.patch_digest, 64)
        || !hex(&proof.source.content_digest, 64)
        || proof.source.worktree_id.trim().is_empty()
        || !Path::new(&proof.source.worktree_root).is_absolute()
        || proof.source.worktree_root == proof.source.source_workspace_root
    {
        return Err("engineering proof source observation is incomplete or escapes targets".into());
    }
    if proof.goal_version != goal.version
        || proof.goal_content != goal.content
        || proof.goal_criteria != goal.criteria
        || goal.status != GoalStatus::Completed
    {
        return Err("engineering proof does not bind the native goal revision".into());
    }
    let attempt = goal
        .attempts
        .last()
        .ok_or("native verifier attempt unavailable")?;
    if !attempt.verdict.eq_ignore_ascii_case("met")
        || attempt.at_ms != proof.verifier_attempt_at_ms
        || sha256_hex(attempt.verifier_reply.as_bytes()) != proof.verifier_reply_digest
    {
        return Err("engineering proof does not bind the finalized native verdict".into());
    }
    Ok(())
}

/// Compare a retained proof against the current isolated source without any
/// live grant. Historical proof stays valid as history; `unchanged` tells the
/// parent whether the source still matches it exactly.
pub async fn compare_current_source(
    cache_dir: PathBuf,
    proof: &EngineeringProof,
) -> Result<SourceComparison, String> {
    let root = PathBuf::from(&proof.source.worktree_root);
    if !root.is_absolute() {
        return Err("retained proof root is not absolute".into());
    }
    let current = observe_root(
        &cache_dir,
        &root,
        Some(&proof.child_chat_id),
        &proof.source.target_files,
    )
    .await?;
    let unchanged = current == proof.source;
    Ok(SourceComparison {
        retained: proof.source.clone(),
        current,
        unchanged,
        compared_at: Utc::now().to_rfc3339(),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::worktrees::types::CreateWorktreeRequest;

    fn git(cwd: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    pub(crate) struct Fixture {
        pub temp: tempfile::TempDir,
        pub cache_dir: PathBuf,
        pub source: PathBuf,
        pub root: PathBuf,
        pub chat_id: String,
        pub mandate_id: String,
        pub targets: Vec<String>,
    }

    pub(crate) async fn fixture(targets: &[&str]) -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(source.join("src")).unwrap();
        git(&source, &["init"]);
        git(&source, &["checkout", "-b", "main"]);
        git(&source, &["config", "user.email", "t@example.com"]);
        git(&source, &["config", "user.name", "T"]);
        git(&source, &["config", "core.autocrlf", "false"]);
        std::fs::write(source.join("src/a.rs"), "fn a() {}\n").unwrap();
        std::fs::write(source.join("README.md"), "readme\n").unwrap();
        git(&source, &["add", "."]);
        git(&source, &["commit", "-m", "init"]);
        let cache_dir = temp.path().join("cache");
        let source = dunce::canonicalize(&source).unwrap();
        let chat_id = format!("subchat-supervised-{}", uuid::Uuid::new_v4());
        let service = WorktreeService::new(cache_dir.clone(), source.clone()).unwrap();
        let head = crate::worktrees::git::head_rev(&source).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some(format!("refact/subagent/{}", &chat_id[19..27])),
                base_branch: Some("main".into()),
                base_commit: Some(head),
                chat_id: Some(chat_id.clone()),
                kind: Some(SUPERVISED_WORKTREE_KIND.into()),
                ..Default::default()
            })
            .await
            .unwrap();
        let root = dunce::canonicalize(&created.worktree.meta.root).unwrap();
        let targets: Vec<String> = targets.iter().map(|t| t.to_string()).collect();
        let mandate_id = super::super::supervision_authority::bind_test_engineering_scope(
            &chat_id,
            &root,
            targets.clone(),
        )
        .unwrap();
        Fixture {
            temp,
            cache_dir,
            source,
            root,
            chat_id,
            mandate_id,
            targets,
        }
    }

    fn facts(fx: &Fixture) -> SessionFacts {
        SessionFacts {
            chat_id: fx.chat_id.clone(),
            goal_version: 1,
            goal_content: "edit a".into(),
            goal_criteria: vec![GoalCriterion {
                id: "C1".into(),
                text: "a edited".into(),
                verify_hint: None,
                verify_command: None,
            }],
        }
    }

    async fn observe_fx(fx: &Fixture) -> SourceObservation {
        let scope = native_edit_scope_for_chat(&fx.chat_id)
            .unwrap()
            .validated_snapshot_scope(&fx.chat_id)
            .unwrap();
        observe(&fx.cache_dir, &scope).await.unwrap()
    }

    #[tokio::test]
    async fn observation_is_deterministic_and_binds_heads() {
        let fx = fixture(&["src/a.rs"]).await;
        let first = observe_fx(&fx).await;
        let second = observe_fx(&fx).await;
        assert_eq!(first, second);
        assert_eq!(first.baseline_head, first.current_head);
        assert!(first.changed_files.is_empty());
        assert_eq!(first.worktree_root, fx.root.to_string_lossy());
        let _ = &fx.temp;
    }

    #[tokio::test]
    async fn same_length_edit_changes_content_digest() {
        let fx = fixture(&["src/a.rs"]).await;
        let before = observe_fx(&fx).await;
        std::fs::write(fx.root.join("src/a.rs"), "fn b() {}\n").unwrap();
        let after = observe_fx(&fx).await;
        assert_ne!(before.content_digest, after.content_digest);
        assert_ne!(before.patch_digest, after.patch_digest);
        assert_eq!(after.changed_files, vec!["src/a.rs".to_string()]);
        // Untracked new target and deleted target are both observable.
        let fx2 = fixture(&["src/a.rs", "src/new.rs"]).await;
        let base = observe_fx(&fx2).await;
        std::fs::write(fx2.root.join("src/new.rs"), "new\n").unwrap();
        let with_new = observe_fx(&fx2).await;
        assert_eq!(with_new.changed_files, vec!["src/new.rs".to_string()]);
        assert_ne!(base.content_digest, with_new.content_digest);
        std::fs::remove_file(fx2.root.join("src/a.rs")).unwrap();
        let deleted = observe_fx(&fx2).await;
        assert!(deleted.changed_files.contains(&"src/a.rs".to_string()));
        assert_ne!(with_new.content_digest, deleted.content_digest);
    }

    #[tokio::test]
    async fn target_escape_symlink_and_head_drift_fail_closed() {
        let fx = fixture(&["src/a.rs"]).await;
        std::fs::write(fx.root.join("README.md"), "changed\n").unwrap();
        let scope = native_edit_scope_for_chat(&fx.chat_id)
            .unwrap()
            .validated_snapshot_scope(&fx.chat_id)
            .unwrap();
        assert!(observe(&fx.cache_dir, &scope).await.is_err());
        git(&fx.root, &["checkout", "--", "README.md"]);
        assert!(observe(&fx.cache_dir, &scope).await.is_ok());
        #[cfg(unix)]
        {
            std::fs::remove_file(fx.root.join("src/a.rs")).unwrap();
            std::os::unix::fs::symlink(fx.source.join("README.md"), fx.root.join("src/a.rs"))
                .unwrap();
            assert!(observe(&fx.cache_dir, &scope).await.is_err());
            std::fs::remove_file(fx.root.join("src/a.rs")).unwrap();
            git(&fx.root, &["checkout", "--", "src/a.rs"]);
        }
        let before = observe(&fx.cache_dir, &scope).await.unwrap();
        std::fs::write(fx.root.join("src/a.rs"), "fn c() {}\n").unwrap();
        git(&fx.root, &["commit", "-am", "edit"]);
        let after = observe(&fx.cache_dir, &scope).await.unwrap();
        assert_ne!(before.current_head, after.current_head);
        assert_eq!(before.baseline_head, after.baseline_head);
        // Root drift: a different directory with the same meta is rejected.
        let other = fx.temp.path().join("other");
        std::fs::create_dir_all(&other).unwrap();
        let other = dunce::canonicalize(&other).unwrap();
        assert!(
            observe_root(&fx.cache_dir, &other, Some(&fx.chat_id), &fx.targets)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn revoked_or_readonly_scope_yields_no_capture() {
        let fx = fixture(&["src/a.rs"]).await;
        assert!(capture_before_stream(fx.cache_dir.clone(), &facts(&fx))
            .await
            .unwrap()
            .is_some());
        super::super::supervision_authority::invalidate(&fx.mandate_id).unwrap();
        assert!(capture_before_stream(fx.cache_dir.clone(), &facts(&fx))
            .await
            .unwrap()
            .is_none());
        assert!(capture_before_stream(
            fx.cache_dir.clone(),
            &SessionFacts {
                chat_id: "unsupervised".into(),
                ..facts(&fx)
            }
        )
        .await
        .unwrap()
        .is_none());
    }

    #[tokio::test]
    async fn change_during_stream_or_revocation_prevents_proof() {
        let fx = fixture(&["src/a.rs"]).await;
        let capture = capture_before_stream(fx.cache_dir.clone(), &facts(&fx))
            .await
            .unwrap()
            .unwrap();
        std::fs::write(fx.root.join("src/a.rs"), "fn z() {}\n").unwrap();
        assert!(bind_after_stream(capture, "GOAL: MET").await.is_err());
        assert!(!finalize_met(
            &fx.chat_id,
            1,
            "edit a",
            &facts(&fx).goal_criteria,
            "GOAL: MET",
            1
        ));
        let capture = capture_before_stream(fx.cache_dir.clone(), &facts(&fx))
            .await
            .unwrap()
            .unwrap();
        super::super::supervision_authority::invalidate(&fx.mandate_id).unwrap();
        assert!(bind_after_stream(capture, "GOAL: MET").await.is_err());
        assert!(get_authorized_proof(&fx.chat_id).is_none());
    }

    #[tokio::test]
    async fn met_finalization_authorizes_exact_reply_and_unmet_discards() {
        let fx = fixture(&["src/a.rs"]).await;
        std::fs::write(fx.root.join("src/a.rs"), "fn edited() {}\n").unwrap();
        let f = facts(&fx);
        let capture = capture_before_stream(fx.cache_dir.clone(), &f)
            .await
            .unwrap()
            .unwrap();
        bind_after_stream(capture, "GOAL: MET\nCRITERION C1: MET")
            .await
            .unwrap();
        // Wrong reply, wrong version, or altered goal content never finalizes.
        assert!(!finalize_met(
            &fx.chat_id,
            1,
            "edit a",
            &f.goal_criteria,
            "GOAL: MET",
            5
        ));
        assert!(!finalize_met(
            &fx.chat_id,
            2,
            "edit a",
            &f.goal_criteria,
            "GOAL: MET\nCRITERION C1: MET",
            5
        ));
        assert!(finalize_met(
            &fx.chat_id,
            1,
            "edit a",
            &f.goal_criteria,
            "GOAL: MET\nCRITERION C1: MET",
            5
        ));
        let proof = get_authorized_proof(&fx.chat_id).unwrap();
        assert_eq!(proof.verifier_attempt_at_ms, 5);
        assert_eq!(proof.source.changed_files, vec!["src/a.rs".to_string()]);
        assert_eq!(proof.mandate_id, fx.mandate_id);
        let json = serde_json::to_string(&proof).unwrap();
        assert_eq!(
            serde_json::from_str::<EngineeringProof>(&json).unwrap(),
            proof
        );
        // Peek does not consume; acknowledge does.
        assert!(get_authorized_proof(&fx.chat_id).is_some());
        let comparison = compare_current_source(fx.cache_dir.clone(), &proof)
            .await
            .unwrap();
        assert!(comparison.unchanged);
        std::fs::write(fx.root.join("src/a.rs"), "fn drift() {}\n").unwrap();
        let comparison = compare_current_source(fx.cache_dir.clone(), &proof)
            .await
            .unwrap();
        assert!(!comparison.unchanged);
        acknowledge_authorized(&fx.chat_id);
        assert!(get_authorized_proof(&fx.chat_id).is_none());
        // UNMET path: pending is discarded, nothing authorized.
        let capture = capture_before_stream(fx.cache_dir.clone(), &f)
            .await
            .unwrap()
            .unwrap();
        bind_after_stream(capture, "GOAL: UNMET\n- more")
            .await
            .unwrap();
        discard_pending(&fx.chat_id, "GOAL: UNMET\n- more");
        assert!(!finalize_met(
            &fx.chat_id,
            1,
            "edit a",
            &f.goal_criteria,
            "GOAL: UNMET\n- more",
            6
        ));
        assert!(get_authorized_proof(&fx.chat_id).is_none());
    }

    #[tokio::test]
    async fn native_verdict_application_promotes_only_the_recorded_met_attempt() {
        let fx = fixture(&["src/a.rs"]).await;
        std::fs::write(fx.root.join("src/a.rs"), "fn edited() {}\n").unwrap();
        let facts = facts(&fx);
        let reply = "GOAL: MET\nCRITERION C1: MET";
        let capture = capture_before_stream(fx.cache_dir.clone(), &facts)
            .await
            .unwrap()
            .unwrap();
        bind_after_stream(capture, reply).await.unwrap();

        let mut session = crate::chat::types::ChatSession::new(fx.chat_id.clone());
        session.install_goal_with_criteria(
            "agent",
            &facts.goal_content,
            true,
            Default::default(),
            facts.goal_criteria.clone(),
        );
        session.goal_set_status(GoalStatus::Verifying);
        let outcome = crate::chat::goal_verifier::apply_goal_verdict(
            &mut session,
            "finish",
            crate::chat::goal_verifier::GoalVerifierReply {
                verdict: crate::chat::goal_verifier::GoalVerdict::Met,
                verifier_reply: reply.into(),
                tokens: 1,
                criteria_verdicts: Vec::new(),
            },
            None,
        );
        assert!(matches!(
            outcome,
            crate::chat::goal_verifier::GoalVerificationApplyOutcome::Finalized
        ));
        assert!(get_authorized_proof(&fx.chat_id).is_some());
        acknowledge_authorized(&fx.chat_id);
    }

    #[test]
    fn validate_retained_binds_mandate_goal_and_attempt() {
        use refact_buddy_core::operator_types::*;
        let project = ProjectIdentity {
            project_id: "p".into(),
            canonical_root: "/r".into(),
        };
        let worker = WorkerIdentity {
            worker_id: "w".into(),
            engine_instance_id: "e".into(),
            epoch: 1,
        };
        let owner = NativeSourceIdentity {
            source_kind: NativeSourceKind::Chat,
            source_id: "u".into(),
            source_version: "1".into(),
        };
        let mandate = Mandate {
            schema_version: 1,
            mandate_id: "m1".into(),
            project: project.clone(),
            owner: owner.clone(),
            owner_epoch: 3,
            resource_scopes: vec![ResourceRef {
                resource_kind: "project".into(),
                resource_id: "p".into(),
                version: "1".into(),
            }],
            allowed_authority_classes: vec![AuthorityClass::ReadOnly, AuthorityClass::Mutating],
            allowed_operations: vec![
                "read_project".into(),
                "spawn_worker".into(),
                "engineer_project".into(),
            ],
            model_constraints: vec!["m".into()],
            budget_limit: Some(BudgetLimit {
                max_turns: Some(2),
                max_tokens: Some(100),
                max_cost_micros: None,
                max_duration_seconds: Some(60),
            }),
            expires_at: Some("2099-01-01T00:00:00Z".into()),
            revoked_at: None,
        };
        let record = MandateRecord {
            schema_version: 1,
            project: project.clone(),
            worker: worker.clone(),
            granted_by: owner,
            mandate,
            max_workers: 1,
            routine_choices: vec![],
            worker_tools: vec!["create_textdoc".into()],
            worker_target_files: vec!["src/a.rs".into()],
            steering_notes: vec![],
        };
        let criteria = vec![GoalCriterion {
            id: "C1".into(),
            text: "a".into(),
            verify_hint: None,
            verify_command: None,
        }];
        let goal = GoalSnapshot {
            content: "edit a".into(),
            owner: None,
            revision: 1,
            plan: None,
            version: 2,
            active: false,
            status: GoalStatus::Completed,
            budget: Default::default(),
            progress: Default::default(),
            attempts: vec![refact_chat_api::GoalAttempt {
                at_ms: 9,
                trigger: "finish".into(),
                verdict: "met".into(),
                gaps: vec![],
                verifier_reply: "GOAL: MET".into(),
                criteria_verdicts: vec![],
            }],
            events: vec![],
            criteria: criteria.clone(),
            snoozed_until_ms: None,
            wait: None,
            stop_reason: None,
            transferred_from: None,
            transferred_to: None,
            sub_goals: Default::default(),
        };
        let proof = EngineeringProof {
            schema_version: SCHEMA_VERSION,
            project: ProjectIdentity {
                project_id: "p".into(),
                canonical_root: "/r".into(),
            },
            worker,
            mandate_id: "m1".into(),
            owner_epoch: 3,
            child_chat_id: "child".into(),
            goal_version: 2,
            goal_content: "edit a".into(),
            goal_criteria: criteria,
            verifier_reply_digest: sha256_hex(b"GOAL: MET"),
            verifier_attempt_at_ms: 9,
            observed_before_at: "1970-01-01T00:00:00Z".into(),
            observed_after_at: "1970-01-01T00:00:00Z".into(),
            source: SourceObservation {
                worktree_id: "wt".into(),
                worktree_root: "/x/worktree".into(),
                source_workspace_root: "/r".into(),
                baseline_head: "a".repeat(40),
                current_head: "b".repeat(40),
                patch_digest: "c".repeat(64),
                content_digest: "d".repeat(64),
                target_files: vec!["src/a.rs".into()],
                changed_files: vec!["src/a.rs".into()],
            },
        };
        assert!(validate_retained(&proof, &record, "child", &goal).is_ok());
        assert!(validate_retained(&proof, &record, "other", &goal).is_err());
        let mut bad = proof.clone();
        bad.source.changed_files.push("README.md".into());
        assert!(validate_retained(&bad, &record, "child", &goal).is_err());
        let mut bad = proof.clone();
        bad.verifier_reply_digest = sha256_hex(b"GOAL: MET ");
        assert!(validate_retained(&bad, &record, "child", &goal).is_err());
        let mut stale_goal = goal.clone();
        stale_goal.version = 3;
        assert!(validate_retained(&proof, &record, "child", &stale_goal).is_err());
        let mut unmet = goal.clone();
        unmet.attempts[0].verdict = "unmet".into();
        assert!(validate_retained(&proof, &record, "child", &unmet).is_err());
        let mut revoked = record.clone();
        revoked.mandate.revoked_at = Some("now".into());
        revoked.mandate.owner_epoch = 4;
        assert!(validate_retained(&proof, &revoked, "child", &goal).is_ok());
        revoked.mandate.owner_epoch = 5;
        assert!(validate_retained(&proof, &revoked, "child", &goal).is_err());
        let mut read_only = record.clone();
        read_only.mandate.allowed_operations = vec!["read_project".into()];
        read_only.worker_tools.clear();
        read_only.worker_target_files.clear();
        assert!(validate_retained(&proof, &read_only, "child", &goal).is_err());
    }
}
