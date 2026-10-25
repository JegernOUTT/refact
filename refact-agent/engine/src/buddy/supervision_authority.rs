use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{OnceLock, RwLock};
#[cfg(test)]
use std::sync::Arc;

use chrono::{DateTime, Utc};
use refact_buddy_core::operator_types::{
    AuthorityClass, Mandate, NativeSourceIdentity, OperatorContract, ProjectIdentity,
    WorkerIdentity,
};
use refact_chat_api::SupervisionCommandGuard;
use serde::{Deserialize, Serialize};

use super::operator_store::{OperatorStoreError, Stored};
use super::outcome_runtime::OutcomeRuntime;

pub const MANDATES: &str = "supervision_mandates";
pub const MAX_MANDATES: usize = 64;
// The owner permits 100 archived deliveries plus one current delivery.
pub(crate) const MAX_QUESTION_DELIVERIES: usize = 101;
pub(crate) const LINK_RESERVED_WRITES: usize = 3 * MAX_QUESTION_DELIVERIES + 4;
pub(crate) fn reservation_slots(id: &str) -> [(&'static str, String, usize); 2] {
    [
        (MANDATES, id.to_owned(), 2),
        (
            super::supervision::LINKS,
            id.to_owned(),
            LINK_RESERVED_WRITES,
        ),
    ]
}

pub(crate) fn ensure_terminal_reservation(
    store: &mut super::operator_store::OperatorStore,
    id: &str,
) -> Result<(), OperatorStoreError> {
    match store.reserve_native_documents("supervision", id, &reservation_slots(id)) {
        Ok(()) => Ok(()),
        Err(OperatorStoreError::Conflict(reason))
            if matches!(reason, "global document capacity reached; reserved completion retained; explicitly raise policy capacity" | "terminal ledger admission capacity reached" | "terminal document admission capacity reached") =>
        {
            store.reserve_grandfathered_terminal_documents("supervision", id, &reservation_slots(id))
        }
        Err(error) => Err(error),
    }
}

/// Native-only capability. No Deserialize implementation: HTTP bodies and model
/// arguments cannot manufacture a supervised spawn or replace its constraints.
#[derive(Clone)]
pub struct NativeSpawnGuard {
    authority: SupervisionCommandGuard,
    parent_chat_id: String,
    model: String,
    goal: crate::agents::spawn::SpawnGoal,
    tools: Vec<String>,
    target_files: Vec<String>,
    engineering: bool,
    expires_at: i64,
}

impl NativeSpawnGuard {
    pub(crate) fn mint(
        record: &MandateRecord,
        parent_chat_id: String,
        goal: crate::agents::spawn::SpawnGoal,
    ) -> Result<Self, String> {
        with_command_guard(Some(&guard(record)), &parent_chat_id, || Ok(()))?;
        if !record
            .mandate
            .allowed_operations
            .iter()
            .any(|op| op == "spawn_worker")
        {
            return Err("worker spawn not granted".into());
        }
        if goal.budget.as_ref() != Some(&super::supervision::native_budget(record)?) {
            return Err("spawn budget differs from mandate".into());
        }
        let engineering = is_engineering(record);
        Ok(Self {
            authority: guard(record),
            parent_chat_id,
            model: record.mandate.model_constraints[0].clone(),
            goal,
            tools: worker_tools(record)?,
            target_files: if engineering {
                worker_target_files(record)?
            } else {
                vec![]
            },
            engineering,
            expires_at: DateTime::parse_from_rfc3339(
                record
                    .mandate
                    .expires_at
                    .as_deref()
                    .ok_or("expiry required")?,
            )
            .map_err(|_| "invalid expiry")?
            .timestamp(),
        })
    }

    pub(crate) fn is_engineering(&self) -> bool {
        self.engineering
    }

    /// The exact repository-relative files the mandate authorized. Native edit
    /// scope enforcement consumes this; it is not prompt text.
    pub(crate) fn target_files(&self) -> &[String] {
        &self.target_files
    }

    pub(crate) fn validate_request(
        &self,
        req: &crate::agents::spawn::SpawnRequest,
    ) -> Result<(), String> {
        with_command_guard(Some(&self.authority), &self.parent_chat_id, || {
            if req.parent_chat_id != self.parent_chat_id
                || req.model != self.model
                || req.model_type.is_some()
                || req.goal.as_ref() != Some(&self.goal)
                || req.tools.as_ref() != Some(&self.tools)
                || req.max_steps
                    != self
                        .goal
                        .budget
                        .as_ref()
                        .and_then(|b| b.max_turns)
                        .unwrap_or(0) as usize
                || req.plan.is_some()
                || req.target_files != self.target_files
                // An engineering worker without exact declared targets has no
                // enforceable edit scope and must never start.
                || (self.engineering
                    && (self.target_files.is_empty()
                        || !matches!(
                            req.worktree_mode,
                            crate::agents::spawn::SpawnWorktreeMode::Isolated { .. }
                        )))
                || matches!(
                    req.worktree_mode,
                    crate::agents::spawn::SpawnWorktreeMode::Isolated { auto_merge: true }
                )
            {
                return Err("native supervised spawn scope mismatch".into());
            }
            Ok(())
        })
    }

    pub(crate) fn bind_child(&self, child: &str) -> Result<(), String> {
        let mut registry = live()
            .write()
            .map_err(|_| "supervision authority unavailable")?;
        let grant = registry
            .get_mut(&self.authority.mandate_id)
            .ok_or("mandate invalidated")?;
        if grant.guard != self.authority
            || grant.expires_at <= Utc::now().timestamp()
            || grant.chat_ids != vec![self.parent_chat_id.clone()]
        {
            return Err("supervised child capacity consumed or authority changed".into());
        }
        grant.chat_ids.push(child.to_string());
        Ok(())
    }

    /// Bind the exact native edit scope of the already-bound child to the live
    /// grant. `worktree_root` is the isolated worktree the spawn just created;
    /// an engineering worker cannot start without one. A read-only worker gets
    /// a scope with no targets, which rejects every edit. The scope lives only
    /// in the process-local registry, so revocation or expiry removes it.
    pub(crate) fn bind_edit_scope(
        &self,
        child: &str,
        worktree_root: Option<&Path>,
    ) -> Result<(), String> {
        let mut registry = live()
            .write()
            .map_err(|_| "supervision authority unavailable")?;
        let grant = registry
            .get_mut(&self.authority.mandate_id)
            .ok_or("mandate invalidated")?;
        if grant.guard != self.authority
            || grant.expires_at <= Utc::now().timestamp()
            || grant.chat_ids.first() != Some(&self.parent_chat_id)
            || !grant.chat_ids.iter().any(|id| id == child)
        {
            return Err("supervised child not bound or authority changed".into());
        }
        if grant
            .edit_scope
            .as_ref()
            .is_some_and(|scope| scope.chat_id != child)
        {
            return Err("native edit scope already bound to another child".into());
        }
        let (worktree_root, target_files) = if self.engineering {
            let root = worktree_root.ok_or("engineering worker requires an isolated worktree")?;
            if self.target_files.is_empty() {
                return Err("engineering worker requires exact target files".into());
            }
            (canonical_scope_root(root)?, self.target_files.clone())
        } else {
            (PathBuf::new(), vec![])
        };
        grant.edit_scope = Some(NativeEditScope {
            authority: self.authority.clone(),
            chat_id: child.to_string(),
            worktree_root,
            target_files,
        });
        Ok(())
    }

    /// Fence every future poll, not merely the call site. Revocation obtains the
    /// same lock, so a suspended provider/tool future cannot resume useful work.
    pub(crate) async fn run<T>(
        &self,
        child: &str,
        future: impl std::future::Future<Output = Result<T, String>>,
    ) -> Result<T, String> {
        let mut future = std::pin::pin!(future);
        let minutes = self
            .goal
            .budget
            .as_ref()
            .and_then(|b| b.max_minutes)
            .ok_or("finite duration required")?;
        let fenced = std::future::poll_fn(|cx| {
            match with_command_guard(Some(&self.authority), child, || {
                Ok(future.as_mut().poll(cx))
            }) {
                Ok(poll) => poll,
                Err(error) => std::task::Poll::Ready(Err(format!("Aborted: {error}"))),
            }
        });
        let remaining = self
            .expires_at
            .saturating_sub(Utc::now().timestamp())
            .max(0) as u64;
        let mut notifications = changes().subscribe();
        let watched = async {
            tokio::pin!(fenced);
            loop {
                tokio::select! {
                    result = &mut fenced => return result,
                    changed = notifications.changed() => {
                        if changed.is_err() { return Err("Aborted: authority unavailable".into()); }
                        with_command_guard(Some(&self.authority), child, || Ok(()))
                            .map_err(|e| format!("Aborted: {e}"))?;
                    }
                }
            }
        };
        tokio::time::timeout(
            std::time::Duration::from_secs((u64::from(minutes) * 60).min(remaining)),
            watched,
        )
        .await
        .map_err(|_| "Aborted: supervised duration exhausted".to_string())?
    }
}

/// Native-only edit capability of one supervised child chat. No Deserialize
/// implementation and no public constructor: only `NativeSpawnGuard::bind_edit_scope`
/// creates it, from mandate-authorized targets and the spawn-created isolated
/// worktree. Every check revalidates the live authority, so a revoked or
/// expired mandate rejects edits even while a tool call is in flight.
#[derive(Clone, Debug)]
pub struct NativeEditScope {
    authority: SupervisionCommandGuard,
    chat_id: String,
    worktree_root: PathBuf,
    target_files: Vec<String>,
}

impl NativeEditScope {
    pub fn chat_id(&self) -> &str {
        &self.chat_id
    }

    pub fn target_files(&self) -> &[String] {
        &self.target_files
    }

    /// Accept only an edit of exactly one mandate-listed target inside the
    /// isolated worktree, resolved through the real filesystem so symlinks,
    /// `..` segments, and dangling links cannot redirect the write.
    pub fn check_edit(&self, chat_id: &str, edit_path: &Path) -> Result<(), String> {
        with_command_guard(Some(&self.authority), chat_id, || {
            if chat_id != self.chat_id {
                return Err("native edit scope is bound to another chat".into());
            }
            if self.target_files.is_empty() {
                return Err(
                    "supervised worker holds no engineering grant; file edits are rejected".into(),
                );
            }
            let root = canonical_scope_root(&self.worktree_root)?;
            let candidate = canonical_edit_candidate(edit_path)?;
            if !candidate.starts_with(&root) {
                return Err(format!(
                    "Path {} is outside the supervised isolated worktree {}",
                    candidate.display(),
                    root.display()
                ));
            }
            let allowed = self.target_files.iter().any(|target| {
                canonical_edit_candidate(&root.join(target))
                    .is_ok_and(|expected| expected.starts_with(&root) && expected == candidate)
            });
            if allowed {
                Ok(())
            } else {
                Err(format!(
                    "Path {} is not one of the exact files the supervision mandate authorized",
                    candidate.display()
                ))
            }
        })
    }

    /// Native read-side twin of `check_edit` for engineering source proof
    /// (`super::supervision_proof`). Revalidates the private live authority,
    /// the child binding, and the canonical isolated worktree root at call
    /// time, then hands out an immutable copy of the exact target set. Fails
    /// closed for restored, expired, or revoked scopes and for read-only
    /// grants, which hold no targets and therefore never produce a proof.
    pub fn validated_snapshot_scope(&self, chat_id: &str) -> Result<NativeSnapshotScope, String> {
        with_command_guard(Some(&self.authority), chat_id, || {
            if chat_id != self.chat_id {
                return Err("native edit scope is bound to another chat".into());
            }
            if self.target_files.is_empty() {
                return Err(
                    "supervised worker holds no engineering grant; no source proof is possible"
                        .into(),
                );
            }
            Ok(NativeSnapshotScope {
                authority: self.authority.clone(),
                chat_id: self.chat_id.clone(),
                worktree_root: canonical_scope_root(&self.worktree_root)?,
                target_files: self.target_files.clone(),
            })
        })
    }
}

/// Immutable, already-validated view of one live engineering scope. No
/// Deserialize implementation and no public constructor: only
/// `NativeEditScope::validated_snapshot_scope` creates it, so holding one
/// proves the grant was live at observation time. It carries no capability.
#[derive(Clone, Debug)]
pub struct NativeSnapshotScope {
    authority: SupervisionCommandGuard,
    chat_id: String,
    worktree_root: PathBuf,
    target_files: Vec<String>,
}

impl NativeSnapshotScope {
    pub fn authority(&self) -> &SupervisionCommandGuard {
        &self.authority
    }

    pub fn chat_id(&self) -> &str {
        &self.chat_id
    }

    /// Canonical absolute root of the spawn-created isolated worktree.
    pub fn worktree_root(&self) -> &Path {
        &self.worktree_root
    }

    /// Exact repository-relative targets the mandate authorized.
    pub fn target_files(&self) -> &[String] {
        &self.target_files
    }

    /// Re-check after a verifier stream that the same live grant still holds
    /// this chat and the root still resolves to the identity captured before.
    pub fn revalidate(&self) -> Result<(), String> {
        with_command_guard(Some(&self.authority), &self.chat_id, || {
            if canonical_scope_root(&self.worktree_root)? != self.worktree_root {
                return Err("supervised worktree root identity changed".into());
            }
            Ok(())
        })
    }
}

/// Test-only: register a live engineering grant bound to `chat_id` with an
/// exact isolated root and target set, mirroring what `bind_edit_scope` does
/// during a real supervised spawn. Returns the mandate id for invalidation.
#[cfg(test)]
pub(crate) fn bind_test_engineering_scope(
    chat_id: &str,
    worktree_root: &Path,
    target_files: Vec<String>,
) -> Result<String, String> {
    let authority = SupervisionCommandGuard {
        mandate_id: uuid::Uuid::new_v4().to_string(),
        project_id: "p".into(),
        worker_id: "w".into(),
        engine_instance_id: "e".into(),
        worker_epoch: 1,
        owner_epoch: 1,
    };
    let root = canonical_scope_root(worktree_root)?;
    live()
        .write()
        .map_err(|_| "supervision authority unavailable")?
        .insert(
            authority.mandate_id.clone(),
            LiveGrant {
                guard: authority.clone(),
                expires_at: Utc::now().timestamp() + 60,
                chat_ids: vec![format!("parent-of-{chat_id}"), chat_id.to_string()],
                edit_scope: Some(NativeEditScope {
                    authority: authority.clone(),
                    chat_id: chat_id.to_string(),
                    worktree_root: root,
                    target_files,
                }),
            },
        );
    Ok(authority.mandate_id)
}

fn canonical_scope_root(root: &Path) -> Result<PathBuf, String> {
    if !root.is_absolute() {
        return Err("supervised worktree root must be absolute".into());
    }
    let canonical = dunce::canonicalize(root)
        .map(|path| dunce::simplified(&path).to_path_buf())
        .map_err(|error| format!("supervised worktree root unavailable: {error}"))?;
    if canonical != dunce::simplified(root) {
        return Err("supervised worktree root identity changed".into());
    }
    if !canonical.is_dir() {
        return Err("supervised worktree root is not a directory".into());
    }
    Ok(canonical)
}

/// Canonicalize the nearest existing ancestor (following symlinks, so a link
/// pointing outside the worktree resolves outside it) and re-append the
/// not-yet-existing tail. Dangling symlinks fail canonicalization and are
/// rejected instead of being written through.
fn canonical_edit_candidate(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!(
            "edit path {} must be absolute inside the supervised worktree",
            path.display()
        ));
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(format!(
            "edit path {} contains parent directory traversal",
            path.display()
        ));
    }
    for ancestor in path.ancestors() {
        match std::fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("supervised edit paths cannot follow symlinks".into());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("supervised edit path cannot be inspected: {error}")),
        }
    }
    let mut existing = path.to_path_buf();
    let mut missing_tail = Vec::new();
    while std::fs::symlink_metadata(&existing).is_err() {
        let name = existing
            .file_name()
            .ok_or_else(|| format!("edit path {} has no file name", path.display()))?
            .to_os_string();
        missing_tail.push(name);
        existing = existing
            .parent()
            .ok_or_else(|| format!("edit path {} has no existing ancestor", path.display()))?
            .to_path_buf();
    }
    let mut canonical = dunce::canonicalize(&existing)
        .map(|resolved| dunce::simplified(&resolved).to_path_buf())
        .map_err(|error| format!("edit path {} cannot be resolved: {error}", path.display()))?;
    for name in missing_tail.iter().rev() {
        canonical.push(name);
    }
    Ok(canonical)
}

/// Live native edit scope of a supervised child chat, if the mandate is still
/// registered and unexpired. Consumed by the file-edit tool runtime.
pub fn native_edit_scope_for_chat(chat_id: &str) -> Option<NativeEditScope> {
    let registry = live().read().ok()?;
    let now = Utc::now().timestamp();
    registry.values().find_map(|grant| {
        grant
            .edit_scope
            .as_ref()
            .filter(|scope| scope.chat_id == chat_id && grant.expires_at > now)
            .cloned()
    })
}

/// Tools a `read_project` grant may use. Unchanged historical subset.
pub(crate) const SUPERVISED_WORKTREE_KIND: &str = "supervised";

pub(crate) const SUPERVISED_READ_TOOLS: [&str; 4] = ["cat", "tree", "search_pattern", "glob"];

/// The only additional tools an explicit `engineer_project` grant may use.
/// Every one of them resolves an exact repository path that native edit-scope
/// enforcement can check. Shell, process, browser, MCP, network, settings,
/// deployment and delegation tools are deliberately absent: accepting their
/// names would hand a supervised worker unbounded side effects.
pub(crate) const SUPERVISED_ENGINEERING_TOOLS: [&str; 6] = [
    "create_textdoc",
    "update_textdoc",
    "update_textdoc_regex",
    "update_textdoc_by_lines",
    "update_textdoc_anchored",
    "apply_patch",
];

pub(crate) fn is_engineering(record: &MandateRecord) -> bool {
    record
        .mandate
        .allowed_operations
        .iter()
        .any(|op| op == "engineer_project")
}

pub(crate) fn read_only_tools(record: &MandateRecord) -> Vec<String> {
    if record
        .mandate
        .allowed_operations
        .iter()
        .any(|op| op == "read_project")
    {
        SUPERVISED_READ_TOOLS
            .into_iter()
            .map(str::to_string)
            .collect()
    } else {
        vec![]
    }
}

/// Exact tool grant. Nothing outside the two native catalogs is reachable, and
/// an engineering tool is only admitted when the mandate carries the explicit
/// mutating authority class, the explicit operation, and exact target files.
pub(crate) fn worker_tools(record: &MandateRecord) -> Result<Vec<String>, String> {
    let mut tools = read_only_tools(record);
    if !is_engineering(record) {
        if !record.worker_tools.is_empty() {
            // Declared engineering tools without the operation are not a grant.
            return Err("declared worker tools require the engineer_project operation".into());
        }
        return Ok(tools);
    }
    if !record
        .mandate
        .allowed_authority_classes
        .contains(&AuthorityClass::Mutating)
        || tools.is_empty()
    {
        return Err(
            "engineering supervision requires explicit mutating authority and read_project".into(),
        );
    }
    if record.worker_tools.is_empty()
        || record.worker_tools.len() > SUPERVISED_ENGINEERING_TOOLS.len()
    {
        return Err("engineering supervision requires explicit bounded worker tools".into());
    }
    for (index, tool) in record.worker_tools.iter().enumerate() {
        if !SUPERVISED_ENGINEERING_TOOLS.contains(&tool.as_str())
            || record.worker_tools[..index].contains(tool)
        {
            return Err("worker tool is outside the native supervised engineering catalog".into());
        }
        tools.push(tool.clone());
    }
    worker_target_files(record)?;
    Ok(tools)
}

/// Repository-relative, non-escaping, unique, explicitly granted edit targets.
pub(crate) fn worker_target_files(record: &MandateRecord) -> Result<Vec<String>, String> {
    if !is_engineering(record) {
        if !record.worker_target_files.is_empty() {
            return Err("declared target files require the engineer_project operation".into());
        }
        return Ok(vec![]);
    }
    if record.worker_target_files.is_empty() || record.worker_target_files.len() > 24 {
        return Err("engineering supervision requires 1..=24 explicit target files".into());
    }
    for (index, path) in record.worker_target_files.iter().enumerate() {
        let trimmed = path.trim();
        if trimmed != path
            || trimmed.is_empty()
            || trimmed.len() > 512
            || trimmed.starts_with('/')
            || trimmed.starts_with('\\')
            || trimmed.contains('\\')
            || trimmed.contains(':')
            || trimmed.contains('*')
            || trimmed
                .split('/')
                .any(|part| part.is_empty() || part == "..")
            || record.worker_target_files[..index].contains(path)
        {
            return Err("target files must be unique, relative, non-escaping paths".into());
        }
    }
    Ok(record.worker_target_files.clone())
}

/// Exact steering note lookup. A supervisor may only deliver text the user
/// already wrote into the mandate; nothing is composed at delivery time.
pub(crate) fn steering_note(record: &MandateRecord, note_id: &str) -> Result<String, String> {
    record
        .steering_notes
        .iter()
        .find(|note| note.note_id == note_id)
        .map(|note| note.text.clone())
        .ok_or_else(|| "no exact mandate-listed steering note for this id".into())
}

/// Bounded, unique, user-written steering notes. Validated once at grant time
/// so delivery never has to interpret free text.
pub(crate) fn validate_steering_notes(notes: &[SteeringNote]) -> Result<(), String> {
    if notes.len() > 12 {
        return Err("at most 12 steering notes are supported".into());
    }
    for (index, note) in notes.iter().enumerate() {
        if note.note_id.trim() != note.note_id
            || note.note_id.is_empty()
            || note.note_id.len() > 64
            || note.text.trim().is_empty()
            || note.text.len() > 4_000
            || notes[..index]
                .iter()
                .any(|other| other.note_id == note.note_id)
        {
            return Err("steering notes must have unique bounded ids and non-empty text".into());
        }
    }
    Ok(())
}

/// Native-only queued steering capability. No Deserialize implementation and
/// no public constructor: only `mint` creates it from a LIVE mandate and a
/// mandate-listed note id. The text is the user's own note, copied at mint
/// time; `consume` re-runs the live-grant, chat-binding, model and expiry
/// checks at the moment the worker loop actually reads the note, so a note
/// queued before revocation, expiry, restart, or a model change is dropped.
#[derive(Clone)]
pub struct NativeSteeringCapability {
    authority: SupervisionCommandGuard,
    child_chat_id: String,
    model: String,
    note_id: String,
    text: String,
    expires_at: i64,
}

impl NativeSteeringCapability {
    pub(crate) fn mint(
        record: &MandateRecord,
        child_chat_id: String,
        note_id: &str,
    ) -> Result<Self, String> {
        if !record
            .mandate
            .allowed_operations
            .iter()
            .any(|op| op == "steer_worker")
        {
            return Err("steer_worker not granted by mandate".into());
        }
        let text = steering_note(record, note_id)?;
        with_command_guard(Some(&guard(record)), &child_chat_id, || Ok(()))?;
        Ok(Self {
            authority: guard(record),
            child_chat_id,
            model: record.mandate.model_constraints[0].clone(),
            note_id: note_id.to_string(),
            text,
            expires_at: DateTime::parse_from_rfc3339(
                record
                    .mandate
                    .expires_at
                    .as_deref()
                    .ok_or("expiry required")?,
            )
            .map_err(|_| "invalid expiry")?
            .timestamp(),
        })
    }

    pub(crate) fn note_id(&self) -> &str {
        &self.note_id
    }

    /// Consume-time authorization. Runs under the shared authority lock so a
    /// concurrent revocation cannot interleave with delivery.
    pub(crate) fn consume(&self, chat_id: &str, model: &str) -> Result<String, String> {
        with_command_guard(Some(&self.authority), chat_id, || {
            if chat_id != self.child_chat_id {
                return Err("steering note is bound to another chat".into());
            }
            if model != self.model {
                return Err("worker model differs from the mandate constraint".into());
            }
            if self.expires_at <= Utc::now().timestamp() {
                return Err("mandate expired before delivery".into());
            }
            Ok(self.text.clone())
        })
    }

    pub(crate) fn into_native_note(
        self,
        control_id: String,
    ) -> crate::agents::registry::NativeNote {
        let note_id = self.note_id.clone();
        crate::agents::registry::NativeNote::new(control_id, note_id, move |chat, model| {
            self.consume(chat, model)
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MandateRecord {
    pub schema_version: u16,
    pub project: ProjectIdentity,
    pub worker: WorkerIdentity,
    pub granted_by: NativeSourceIdentity,
    pub mandate: Mandate,
    pub max_workers: u32,
    pub routine_choices: Vec<RoutineChoice>,
    /// Engineering tool names the user explicitly granted. Absent/empty keeps
    /// the historical read-only behaviour byte for byte.
    #[serde(default)]
    pub worker_tools: Vec<String>,
    #[serde(default)]
    pub worker_target_files: Vec<String>,
    #[serde(default)]
    pub steering_notes: Vec<SteeringNote>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MandateRequest {
    pub mandate: Mandate,
    pub max_workers: u32,
    pub routine_choices: Vec<RoutineChoice>,
    #[serde(default)]
    pub worker_tools: Vec<String>,
    #[serde(default)]
    pub worker_target_files: Vec<String>,
    #[serde(default)]
    pub steering_notes: Vec<SteeringNote>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SteeringNote {
    pub note_id: String,
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutineChoice {
    pub question_id: String,
    pub question_text: String,
    pub options: Vec<String>,
    pub selected: String,
}

struct LiveGrant {
    guard: SupervisionCommandGuard,
    expires_at: i64,
    chat_ids: Vec<String>,
    edit_scope: Option<NativeEditScope>,
}

static LIVE: OnceLock<RwLock<HashMap<String, LiveGrant>>> = OnceLock::new();
static CHANGES: OnceLock<tokio::sync::watch::Sender<u64>> = OnceLock::new();

fn changes() -> &'static tokio::sync::watch::Sender<u64> {
    CHANGES.get_or_init(|| tokio::sync::watch::channel(0).0)
}

fn live() -> &'static RwLock<HashMap<String, LiveGrant>> {
    LIVE.get_or_init(|| RwLock::new(HashMap::new()))
}

pub fn guard(record: &MandateRecord) -> SupervisionCommandGuard {
    SupervisionCommandGuard {
        mandate_id: record.mandate.mandate_id.clone(),
        project_id: record.project.project_id.clone(),
        worker_id: record.worker.worker_id.clone(),
        engine_instance_id: record.worker.engine_instance_id.clone(),
        worker_epoch: record.worker.epoch,
        owner_epoch: record.mandate.owner_epoch,
    }
}

pub fn validate_command_guard(candidate: &SupervisionCommandGuard) -> Result<(), String> {
    let registry = live()
        .read()
        .map_err(|_| "supervision authority unavailable")?;
    match registry.get(&candidate.mandate_id) {
        Some(current)
            if current.guard == *candidate && current.expires_at > Utc::now().timestamp() =>
        {
            Ok(())
        }
        _ => Err("supervision revoked, stale, or not restored by explicit user grant".into()),
    }
}

pub fn validate_chat_command(
    candidate: &SupervisionCommandGuard,
    chat_id: &str,
    command: &refact_chat_api::ChatCommand,
) -> Result<(), String> {
    match command {
        refact_chat_api::ChatCommand::SetGoal { .. }
        | refact_chat_api::ChatCommand::AnswerQuestions { .. } => {}
        refact_chat_api::ChatCommand::GoalControl { action }
            if action == "pause" || action == "stop" => {}
        _ => return Err("command is not supported by supervision".into()),
    }
    with_command_guard(Some(candidate), chat_id, || Ok(()))
}

pub fn with_command_guard<T>(
    candidate: Option<&SupervisionCommandGuard>,
    chat_id: &str,
    consume: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let Some(candidate) = candidate else {
        return consume();
    };
    let registry = live()
        .read()
        .map_err(|_| "supervision authority unavailable")?;
    match registry.get(&candidate.mandate_id) {
        Some(current)
            if current.guard == *candidate
                && current.expires_at > Utc::now().timestamp()
                && current.chat_ids.iter().any(|id| id == chat_id) =>
        {
            consume()
        }
        _ => Err("supervision revoked, stale, or bound to another chat".into()),
    }
}

pub fn bind_chat(mandate_id: &str, chat_id: String) -> Result<(), String> {
    let mut registry = live()
        .write()
        .map_err(|_| "supervision authority unavailable")?;
    let grant = registry.get_mut(mandate_id).ok_or("mandate invalidated")?;
    if grant.expires_at <= Utc::now().timestamp() {
        return Err("mandate expired".into());
    }
    if grant.chat_ids.iter().any(|id| id == &chat_id) {
        return Ok(());
    }
    if grant.chat_ids.len() >= 2 {
        return Err("mandate chat capacity reached".into());
    }
    grant.chat_ids.push(chat_id);
    Ok(())
}

pub fn invalidate_chat(chat_id: &str) -> Result<(), String> {
    live()
        .write()
        .map_err(|_| "supervision authority unavailable")?
        .retain(|_, grant| !grant.chat_ids.iter().any(|id| id == chat_id));
    changes().send_modify(|version| *version = version.wrapping_add(1));
    Ok(())
}

pub fn invalidate(mandate_id: &str) -> Result<(), String> {
    live()
        .write()
        .map_err(|_| "supervision authority unavailable")?
        .remove(mandate_id);
    changes().send_modify(|version| *version = version.wrapping_add(1));
    Ok(())
}

pub fn validate_record(record: &MandateRecord, runtime: &OutcomeRuntime) -> Result<(), String> {
    if record.project != runtime.project() || record.worker != runtime.worker() {
        return Err("mandate project or worker epoch changed; explicit grant required".into());
    }
    validate_mandate(
        &record.mandate,
        record.max_workers,
        &record.project,
        Utc::now(),
    )?;
    validate_command_guard(&guard(record))
}

fn validate_mandate(
    mandate: &Mandate,
    workers: u32,
    project: &ProjectIdentity,
    now: DateTime<Utc>,
) -> Result<(), String> {
    mandate.validate().map_err(|e| e.to_string())?;
    if mandate.project != *project
        || mandate.owner_epoch == 0
        || mandate.revoked_at.is_some()
        || !(1..=2).contains(&workers)
    {
        return Err("invalid mandate identity, revocation, or concurrency bound".into());
    }
    let expiry = mandate
        .expires_at
        .as_deref()
        .ok_or("mandate expiry required")?;
    let expiry = DateTime::parse_from_rfc3339(expiry).map_err(|_| "invalid mandate expiry")?;
    if expiry <= now || expiry.timestamp() - now.timestamp() > 86_400 {
        return Err("mandate must expire within 24 hours".into());
    }
    // Read-only remains the only default. Mutating is admitted exclusively as
    // the exact pair [ReadOnly, Mutating] together with engineer_project; no
    // egress, credential, publication or deployment class is representable.
    let engineering = mandate
        .allowed_operations
        .iter()
        .any(|op| op == "engineer_project");
    let classes_ok = if engineering {
        mandate.allowed_authority_classes
            == vec![AuthorityClass::ReadOnly, AuthorityClass::Mutating]
            && mandate
                .allowed_operations
                .iter()
                .any(|op| op == "read_project")
            && mandate
                .allowed_operations
                .iter()
                .any(|op| op == "spawn_worker")
    } else {
        mandate.allowed_authority_classes == vec![AuthorityClass::ReadOnly]
    };
    if !classes_ok
        || mandate.allowed_operations.is_empty()
        || mandate.allowed_operations.iter().any(|op| {
            ![
                "create_goal",
                "spawn_worker",
                "answer_question",
                "read_project",
                "engineer_project",
                "steer_worker",
            ]
            .contains(&op.as_str())
        })
        || mandate
            .allowed_operations
            .iter()
            .enumerate()
            .any(|(i, op)| mandate.allowed_operations[..i].contains(op))
        || mandate.resource_scopes.len() != 1
        || mandate.resource_scopes[0].resource_kind != "project"
        || mandate.resource_scopes[0].resource_id != project.project_id
        || mandate.model_constraints.len() != 1
        || mandate.model_constraints[0].trim().is_empty()
    {
        return Err("only explicit single-project read-only supervision with one selected model is supported".into());
    }
    let budget = mandate
        .budget_limit
        .as_ref()
        .ok_or("finite budget required")?;
    if !matches!(budget.max_turns, Some(1..=100))
        || !matches!(budget.max_tokens, Some(1..=1_000_000))
        || !matches!(budget.max_duration_seconds, Some(60..=86_400))
        || budget.max_duration_seconds.unwrap() % 60 != 0
        || budget
            .max_cost_micros
            .is_some_and(|v| v == 0 || v % 10_000 != 0)
    {
        return Err(
            "unsupported budget: finite turns/tokens/whole minutes and exact cent cost required"
                .into(),
        );
    }
    Ok(())
}

pub(crate) async fn create(
    runtime: &OutcomeRuntime,
    requester: NativeSourceIdentity,
    request: MandateRequest,
) -> Result<Stored<MandateRecord>, String> {
    validate_mandate(
        &request.mandate,
        request.max_workers,
        &runtime.project(),
        Utc::now(),
    )?;
    if request.mandate.owner != requester {
        return Err("mandate owner must match authenticated HTTP origin".into());
    }
    if request.routine_choices.len() > 12
        || request.routine_choices.iter().any(|choice| {
            choice.question_id.is_empty()
                || choice.question_text.is_empty()
                || choice.options.is_empty()
                || choice.options.len() > 12
                || !choice.options.contains(&choice.selected)
        })
    {
        return Err("routine choices must be explicit bounded single-select choices".into());
    }
    validate_steering_notes(&request.steering_notes)?;
    let record = MandateRecord {
        schema_version: 1,
        project: runtime.project(),
        worker: runtime.worker(),
        granted_by: requester,
        mandate: request.mandate,
        max_workers: request.max_workers,
        routine_choices: request.routine_choices,
        worker_tools: request.worker_tools,
        worker_target_files: request.worker_target_files,
        steering_notes: request.steering_notes,
    };
    // The exact tool/target grant is validated at grant time, so a stored
    // engineering mandate can never exist without an enforceable edit scope.
    worker_tools(&record)?;
    worker_target_files(&record)?;
    let stored = runtime
        .with_store(move |store| {
            if store
                .get_document::<MandateRecord>(MANDATES, &record.mandate.mandate_id)?
                .is_some()
            {
                return Err(OperatorStoreError::Conflict(
                    "mandate identity already consumed; create a new explicit grant",
                ));
            }
            if store
                .list_documents::<MandateRecord>(MANDATES, None, MAX_MANDATES)?
                .entries
                .len()
                >= MAX_MANDATES
            {
                return Err(OperatorStoreError::Conflict("mandate capacity reached"));
            }
            let id = &record.mandate.mandate_id;
            store.reserve_native_documents("supervision", id, &reservation_slots(id))?;
            store.put_native_ordinary_document("supervision", id, MANDATES, id, &record, None)
        })
        .await
        .map_err(|e| e.to_string())?;
    let expires_at = DateTime::parse_from_rfc3339(
        stored
            .value
            .mandate
            .expires_at
            .as_deref()
            .ok_or("expiry missing")?,
    )
    .map_err(|e| e.to_string())?
    .timestamp();
    live()
        .write()
        .map_err(|_| "supervision authority unavailable")?
        .insert(
            stored.value.mandate.mandate_id.clone(),
            LiveGrant {
                guard: guard(&stored.value),
                expires_at,
                chat_ids: vec![],
                edit_scope: None,
            },
        );
    Ok(stored)
}

pub async fn load(runtime: &OutcomeRuntime, id: String) -> Result<Stored<MandateRecord>, String> {
    let stored = runtime
        .with_store(move |store| store.get_document::<MandateRecord>(MANDATES, &id))
        .await
        .map_err(|e| e.to_string())?
        .ok_or("mandate not found")?;
    if stored.value.schema_version != 1
        || stored.value.project != runtime.project()
        || stored.value.worker != runtime.worker()
    {
        return Err("mandate source project or worker binding mismatch".into());
    }
    Ok(stored)
}

/// Read only a durably retained mandate for historical completion publication.
/// Unlike `load`, this deliberately preserves the admitted worker identity
/// instead of accepting a new runtime epoch. It is not an authority loader:
/// callers must additionally bind a retained link and proof, and must never
/// use this record to mint a guard, dispatch, or control a worker.
pub(crate) async fn load_historical(
    runtime: &OutcomeRuntime,
    id: String,
) -> Result<Stored<MandateRecord>, String> {
    let expected_id = id.clone();
    let stored = runtime
        .with_store(move |store| store.get_document::<MandateRecord>(MANDATES, &id))
        .await
        .map_err(|e| e.to_string())?
        .ok_or("mandate not found")?;
    if stored.value.schema_version != 1
        || stored.value.project != runtime.project()
        || stored.value.mandate.schema_version != 1
        || stored.value.mandate.mandate_id != expected_id
        || stored.value.mandate.project != stored.value.project
        || stored.value.worker.worker_id.trim().is_empty()
        || stored.value.worker.engine_instance_id.trim().is_empty()
        || stored.value.worker.epoch == 0
    {
        return Err("historical mandate project, schema, or worker binding mismatch".into());
    }
    Ok(stored)
}

pub(crate) async fn revoke(
    runtime: &OutcomeRuntime,
    id: String,
) -> Result<Stored<MandateRecord>, String> {
    load(runtime, id.clone()).await?;
    invalidate(&id)?;
    runtime
        .with_store(move |store| {
            let mut record = store
                .get_document::<MandateRecord>(MANDATES, &id)?
                .ok_or(OperatorStoreError::Conflict("mandate not found"))?;
            if record.value.mandate.revoked_at.is_some() {
                return Ok(record);
            }
            ensure_terminal_reservation(store, &id)?;
            record.value.mandate.revoked_at = Some(Utc::now().to_rfc3339());
            record.value.mandate.owner_epoch = record
                .value
                .mandate
                .owner_epoch
                .checked_add(1)
                .ok_or(OperatorStoreError::Conflict("owner epoch exhausted"))?;
            store.put_reserved_document(
                "supervision",
                &id,
                MANDATES,
                &id,
                &record.value,
                Some(record.revision),
            )
        })
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spawn_fixture() -> (NativeSpawnGuard, crate::agents::spawn::SpawnRequest) {
        use crate::agents::spawn::{NotifyParent, SpawnGoal, SpawnRequest, SpawnWorktreeMode};
        let authority = SupervisionCommandGuard {
            mandate_id: uuid::Uuid::new_v4().to_string(),
            project_id: "p".into(),
            worker_id: "w".into(),
            engine_instance_id: "e".into(),
            worker_epoch: 1,
            owner_epoch: 1,
        };
        let parent = uuid::Uuid::new_v4().to_string();
        live().write().unwrap().insert(
            authority.mandate_id.clone(),
            LiveGrant {
                guard: authority.clone(),
                expires_at: Utc::now().timestamp() + 60,
                chat_ids: vec![parent.clone()],
                edit_scope: None,
            },
        );
        let goal = SpawnGoal {
            content: "inspect only".into(),
            criteria: vec![refact_chat_api::GoalCriterion {
                id: "C1".into(),
                text: "report evidence".into(),
                verify_hint: None,
                verify_command: None,
            }],
            budget: Some(refact_chat_api::GoalBudget {
                max_turns: Some(2),
                max_tokens: Some(8192),
                max_minutes: Some(1),
                explicit: true,
                ..Default::default()
            }),
        };
        let guard = NativeSpawnGuard {
            authority,
            parent_chat_id: parent.clone(),
            model: "test-model".into(),
            goal: goal.clone(),
            tools: vec!["cat".into(), "tree".into()],
            target_files: vec![],
            engineering: false,
            expires_at: Utc::now().timestamp() + 60,
        };
        let request = SpawnRequest {
            completion_push: Default::default(),
            native_supervision_guard: Some(guard.clone()),
            kind: crate::agents::types::BgAgentKind::Subagent,
            parent_chat_id: parent.clone(),
            parent_root_chat_id: Some(parent),
            parent_tool_call_id: None,
            config_name: "test_spawn".into(),
            title: "test".into(),
            prompt: goal.content.clone(),
            initial_context_messages: Vec::new(),
            history_source_model: None,
            custom_instructions: None,
            deliverable: None,
            profile: None,
            tools: Some(guard.tools.clone()),
            target_files: vec![],
            max_steps: 2,
            model: "test-model".into(),
            model_type: None,
            goal: Some(goal),
            plan: None,
            worktree_mode: SpawnWorktreeMode::Inherit,
            parent_subchat_tx: None,
            parent_worktree: None,
            parent_task_meta: None,
            subchat_depth: 0,
            notify_parent: NotifyParent::Silent,
        };
        (guard, request)
    }

    #[tokio::test]
    async fn native_edit_scope_enforces_actual_write_and_revocation_without_board_metadata() {
        use crate::at_commands::at_commands::AtCommandsContext;
        use crate::tools::file_edit::tool_create_textdoc::tool_create_text_doc_exec;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("src")).unwrap();
        let (mut guard, _) = spawn_fixture();
        guard.engineering = true;
        guard.target_files = vec!["src/allowed.rs".into()];
        let child = format!("subchat-supervised-{}", uuid::Uuid::new_v4());
        guard.bind_child(&child).unwrap();
        guard.bind_edit_scope(&child, Some(&root)).unwrap();
        let gcx = crate::global_context::tests::make_test_gcx_with_dirs(
            root.join("cache"),
            root.join("config"),
        )
        .await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.clone()];
        let ccx = Arc::new(tokio::sync::Mutex::new(
            AtCommandsContext::new(
                gcx.clone(),
                4096,
                1,
                false,
                vec![],
                child.clone(),
                None,
                "test-model".into(),
                None,
                None,
            )
            .await,
        ));
        let args = |path: &Path| {
            std::collections::HashMap::from([
                ("path".into(), serde_json::json!(path)),
                ("content".into(), serde_json::json!("fn retained() {}")),
            ])
        };
        let allowed = root.join("src/allowed.rs");
        tool_create_text_doc_exec(gcx.clone(), &args(&allowed), false, None, Some(&ccx))
            .await
            .unwrap();
        let bytes = std::fs::read(&allowed).unwrap();
        let outside = root.join("src/outside.rs");
        assert!(
            tool_create_text_doc_exec(gcx.clone(), &args(&outside), false, None, Some(&ccx))
                .await
                .is_err()
        );
        assert!(!outside.exists());
        assert!(native_edit_scope_for_chat(&child)
            .unwrap()
            .check_edit(&child, &root.join("src/../src/allowed.rs"))
            .is_err());
        #[cfg(unix)]
        {
            let external = tempfile::tempdir().unwrap();
            let target = external.path().join("target.rs");
            std::fs::write(&target, "outside bytes").unwrap();
            std::fs::remove_file(&allowed).unwrap();
            std::os::unix::fs::symlink(&target, &allowed).unwrap();
            assert!(tool_create_text_doc_exec(
                gcx.clone(),
                &args(&allowed),
                false,
                None,
                Some(&ccx)
            )
            .await
            .is_err());
            assert_eq!(std::fs::read_to_string(&target).unwrap(), "outside bytes");
            std::fs::remove_file(&allowed).unwrap();
            std::fs::write(&allowed, &bytes).unwrap();
        }
        let partial_patch = format!(
            "*** Begin Patch\n*** Update File: {}\n@@\n-fn retained() {{}}\n+fn changed() {{}}\n*** Add File: {}\n+forbidden\n*** End Patch",
            allowed.display(), outside.display(),
        );
        let patch_args =
            std::collections::HashMap::from([("patch".into(), serde_json::json!(partial_patch))]);
        assert!(
            crate::tools::file_edit::tool_apply_patch::tool_apply_patch_exec(
                gcx.clone(),
                &patch_args,
                false,
                None,
                Some(&ccx),
            )
            .await
            .is_err()
        );
        assert_eq!(std::fs::read(&allowed).unwrap(), bytes);
        assert!(!outside.exists());
        let rename_patch = format!(
            "*** Begin Patch\n*** Update File: {}\n*** Move to: {}\n@@\n-fn retained() {{}}\n+fn changed() {{}}\n*** End Patch",
            allowed.display(), outside.display(),
        );
        let patch_args =
            std::collections::HashMap::from([("patch".into(), serde_json::json!(rename_patch))]);
        assert!(
            crate::tools::file_edit::tool_apply_patch::tool_apply_patch_exec(
                gcx.clone(),
                &patch_args,
                false,
                None,
                Some(&ccx),
            )
            .await
            .is_err()
        );
        assert_eq!(std::fs::read(&allowed).unwrap(), bytes);
        assert!(!outside.exists());
        invalidate(&guard.authority.mandate_id).unwrap();
        assert!(
            tool_create_text_doc_exec(gcx.clone(), &args(&allowed), false, None, Some(&ccx))
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(&allowed).unwrap(), bytes);
        let restored = Arc::new(tokio::sync::Mutex::new(
            AtCommandsContext::new(
                gcx.clone(),
                4096,
                1,
                false,
                vec![],
                child,
                None,
                "test-model".into(),
                None,
                None,
            )
            .await,
        ));
        assert!(
            tool_create_text_doc_exec(gcx, &args(&allowed), false, None, Some(&restored))
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(&allowed).unwrap(), bytes);
    }

    #[test]
    fn supervised_spawn_rejects_model_budget_tools_and_owner_drift() {
        let (guard, request) = spawn_fixture();
        assert!(guard.validate_request(&request).is_ok());
        let mut changed = request.clone();
        changed.model = "other".into();
        assert!(guard.validate_request(&changed).is_err());
        let mut changed = request.clone();
        changed
            .goal
            .as_mut()
            .unwrap()
            .budget
            .as_mut()
            .unwrap()
            .max_turns = Some(3);
        assert!(guard.validate_request(&changed).is_err());
        let mut changed = request.clone();
        changed.tools = Some(vec!["shell".into()]);
        assert!(guard.validate_request(&changed).is_err());
        let mut changed = request.clone();
        changed.parent_chat_id = "imposter".into();
        assert!(guard.validate_request(&changed).is_err());
        guard.bind_child("child").unwrap();
        assert!(guard.bind_child("second-child").is_err());
        invalidate(&guard.authority.mandate_id).unwrap();
        assert!(guard.validate_request(&request).is_err());
    }

    #[tokio::test]
    async fn revoked_spawn_never_creates_registry_record_or_provider_work() {
        let (guard, request) = spawn_fixture();
        let app = crate::app_state::AppState::from_gcx(
            crate::global_context::tests::make_test_gcx().await,
        )
        .await;
        invalidate(&guard.authority.mandate_id).unwrap();
        assert!(
            crate::agents::spawn::spawn_background_agent(app.clone(), request)
                .await
                .is_err()
        );
        assert!(app.agents.list_all().await.is_empty());
        assert!(app.chat.sessions.read().await.is_empty());
    }

    #[tokio::test]
    async fn revoked_suspended_provider_is_never_polled_again() {
        let (guard, _) = spawn_fixture();
        guard.bind_child("child").unwrap();
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let useful = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let observed = useful.clone();
        let running_guard = guard.clone();
        let worker = tokio::spawn(async move {
            running_guard
                .run("child", async move {
                    entered_tx.send(()).unwrap();
                    release_rx.await.map_err(|e| e.to_string())?;
                    observed.store(true, std::sync::atomic::Ordering::SeqCst);
                    Ok(())
                })
                .await
        });
        entered_rx.await.unwrap();
        invalidate(&guard.authority.mandate_id).unwrap();
        let _ = release_tx.send(());
        assert!(worker.await.unwrap().is_err());
        assert!(!useful.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[serial_test::serial(test_runner)]
    #[tokio::test]
    async fn guarded_native_goal_is_installed_before_fake_provider_and_revoke_stops_it() {
        let (guard, request) = spawn_fixture();
        let expected = request.goal.clone().unwrap();
        // Every launch now resolves and canonicalizes an active project through
        // `validate_launch_request`, so a supervised spawn needs a real workspace
        // root the way the edit-scope test above already provides one.
        let workspace = tempfile::tempdir().unwrap();
        let workspace_root = workspace.path().canonicalize().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![workspace_root];
        let (defaults_root, _) = crate::caps::project_model_defaults_state(gcx.clone()).await;
        {
            let mut caps = crate::caps::CodeAssistantCaps::default();
            caps.chat_models.insert(
                "test-model".into(),
                std::sync::Arc::new(crate::caps::ChatModelRecord {
                    base: crate::caps::BaseModelRecord {
                        id: "test-model".into(),
                        ..Default::default()
                    },
                    supports_agent: true,
                    supports_tools: true,
                    ..Default::default()
                }),
            );
            let mut state = gcx.caps_state.write().await;
            state.caps = Some(std::sync::Arc::new(caps));
            state.loaded_project_root = Some(defaults_root);
            state.last_attempted_ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();
        }
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        let entered = std::sync::Arc::new(tokio::sync::Notify::new());
        let useful = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let release = std::sync::Arc::new(tokio::sync::Notify::new());
        let runner_entered = entered.clone();
        let runner_release = release.clone();
        let runner_useful = useful.clone();
        let _runner = crate::agents::spawn::install_test_runner(std::sync::Arc::new(
            move |gcx, messages, config| {
                let entered = runner_entered.clone();
                let release = runner_release.clone();
                let useful = runner_useful.clone();
                let expected = expected.clone();
                Box::pin(async move {
                    let app = crate::app_state::AppState::from_gcx(gcx).await;
                    let snapshot = app
                        .chat
                        .facade
                        .session_snapshot(config.chat_id.as_deref().unwrap())
                        .await?;
                    let goal = snapshot.goal.unwrap();
                    assert_eq!(goal.version, 1);
                    assert_eq!(goal.content, expected.content);
                    assert_eq!(goal.criteria, expected.criteria);
                    assert_eq!(Some(goal.budget), expected.budget);
                    assert_eq!(snapshot.thread.model, "test-model");
                    assert!(!config.final_step_force_answer);
                    assert!(!config.auto_approve_editing_tools);
                    assert!(!config.auto_approve_dangerous_commands);
                    assert!(
                        matches!(&config.tools, crate::subchat::ToolsPolicy::Supervised { tools, .. } if tools == &vec!["cat".to_string(), "tree".to_string()])
                    );
                    entered.notify_one();
                    release.notified().await;
                    useful.store(true, std::sync::atomic::Ordering::SeqCst);
                    Ok(crate::subchat::SubchatResult {
                        messages,
                        metering: serde_json::Map::new(),
                        chat_id: config.chat_id,
                        aborted: false,
                    })
                })
            },
        ));
        let handle = crate::agents::spawn::spawn_background_agent(app, request)
            .await
            .unwrap();
        entered.notified().await;
        invalidate(&guard.authority.mandate_id).unwrap();
        release.notify_one();
        let record = tokio::time::timeout(std::time::Duration::from_secs(5), handle.completion_rx)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            record.status,
            crate::agents::types::BgAgentStatus::Cancelled
        );
        assert!(!useful.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn queued_goal_is_rejected_after_user_takeover() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        let chat_id = uuid::Uuid::new_v4().to_string();
        let candidate = SupervisionCommandGuard {
            mandate_id: uuid::Uuid::new_v4().to_string(),
            project_id: "p".into(),
            worker_id: "w".into(),
            engine_instance_id: "e".into(),
            worker_epoch: 1,
            owner_epoch: 1,
        };
        live().write().unwrap().insert(
            candidate.mandate_id.clone(),
            LiveGrant {
                guard: candidate.clone(),
                expires_at: Utc::now().timestamp() + 60,
                chat_ids: vec![chat_id.clone()],
                edit_scope: None,
            },
        );
        let session = std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::chat::types::ChatSession::new(chat_id.clone()),
        ));
        let mut locked = session.lock().await;
        let command = refact_chat_api::ChatCommand::SetGoal {
            content: "must not install".into(),
            budget: None,
            criteria: None,
        };
        assert!(validate_chat_command(&candidate, "another-chat", &command).is_err());
        let request = refact_chat_api::CommandRequest {
            client_request_id: "supervised-goal".into(),
            priority: false,
            command,
            native_approval_guard: None,
            supervision_guard: Some(candidate.clone()),
        };
        assert_eq!(
            locked.enqueue_command(request),
            crate::chat::types::EnqueueCommandOutcome::Accepted
        );
        let takeover = refact_chat_api::CommandRequest {
            client_request_id: "takeover".into(),
            priority: false,
            command: refact_chat_api::ChatCommand::GoalControl {
                action: "stop".into(),
            },
            native_approval_guard: None,
            supervision_guard: None,
        };
        assert_eq!(
            locked.enqueue_command(takeover),
            crate::chat::types::EnqueueCommandOutcome::Accepted
        );
        assert!(validate_command_guard(&candidate).is_err());
        let mut events = locked.subscribe();
        let running = locked.queue_processor_running.clone();
        running.store(true, std::sync::atomic::Ordering::SeqCst);
        drop(locked);
        let worker = tokio::spawn(crate::chat::process_command_queue(
            app,
            session.clone(),
            running,
        ));
        loop {
            let event: serde_json::Value =
                serde_json::from_str(&events.recv().await.unwrap()).unwrap();
            if event["client_request_id"] == "supervised-goal" {
                assert_eq!(event["accepted"], false);
                break;
            }
        }
        let mut locked = session.lock().await;
        assert!(locked.goal.is_none());
        assert!(!locked.messages.iter().any(|message| message.role == "goal"));
        locked.close_event_channel();
        drop(locked);
        worker.await.unwrap();
    }

    #[tokio::test]
    async fn revoked_queued_answer_keeps_native_question_pending() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = crate::app_state::AppState::from_gcx(gcx).await;
        let chat_id = uuid::Uuid::new_v4().to_string();
        let candidate = SupervisionCommandGuard {
            mandate_id: uuid::Uuid::new_v4().to_string(),
            project_id: "p".into(),
            worker_id: "w".into(),
            engine_instance_id: "e".into(),
            worker_epoch: 1,
            owner_epoch: 1,
        };
        live().write().unwrap().insert(
            candidate.mandate_id.clone(),
            LiveGrant {
                guard: candidate.clone(),
                expires_at: Utc::now().timestamp() + 60,
                chat_ids: vec![chat_id.clone()],
                edit_scope: None,
            },
        );
        let session = std::sync::Arc::new(tokio::sync::Mutex::new(
            crate::chat::types::ChatSession::new(chat_id),
        ));
        let mut locked = session.lock().await;
        locked.runtime.state = refact_chat_api::SessionState::WaitingUserInput;
        locked
            .pending_questions
            .push(refact_chat_api::PendingQuestionSet {
                tool_call_id: "question-tool".into(),
                asked_at_ms: 1,
                status: refact_chat_api::PendingQuestionStatus::Pending,
                questions: vec![refact_chat_api::PendingQuestion {
                    id: "q".into(),
                    question_type: "single_select".into(),
                    text: "Choose".into(),
                    options: Some(vec!["a".into()]),
                }],
                answers: vec![],
                resolution_id: None,
                response_protocol_version: refact_chat_api::QUESTION_RESPONSE_PROTOCOL_VERSION,
                response: None,
            });
        let request = refact_chat_api::CommandRequest {
            client_request_id: "supervised-answer".into(),
            priority: false,
            command: refact_chat_api::ChatCommand::AnswerQuestions {
                tool_call_id: "question-tool".into(),
                asked_at_ms: Some(1),
                answers: vec![refact_chat_api::QuestionAnswer {
                    id: "q".into(),
                    answer: serde_json::json!("a"),
                }],
            },
            native_approval_guard: None,
            supervision_guard: Some(candidate.clone()),
        };
        assert_eq!(
            locked.enqueue_command(request),
            crate::chat::types::EnqueueCommandOutcome::Accepted
        );
        invalidate(&candidate.mandate_id).unwrap();
        let mut events = locked.subscribe();
        let running = locked.queue_processor_running.clone();
        running.store(true, std::sync::atomic::Ordering::SeqCst);
        drop(locked);
        let worker = tokio::spawn(crate::chat::process_command_queue(
            app,
            session.clone(),
            running,
        ));
        loop {
            let event: serde_json::Value =
                serde_json::from_str(&events.recv().await.unwrap()).unwrap();
            if event["client_request_id"] == "supervised-answer" {
                assert_eq!(event["accepted"], false);
                break;
            }
        }
        let mut locked = session.lock().await;
        assert_eq!(
            locked.pending_questions[0].status,
            refact_chat_api::PendingQuestionStatus::Pending
        );
        assert!(locked.pending_questions[0].answers.is_empty());
        assert!(locked.messages.is_empty());
        locked.close_event_channel();
        drop(locked);
        worker.await.unwrap();
    }

    #[test]
    fn consumption_and_revocation_share_one_authority_lock() {
        let candidate = SupervisionCommandGuard {
            mandate_id: uuid::Uuid::new_v4().to_string(),
            project_id: "p".into(),
            worker_id: "w".into(),
            engine_instance_id: "e".into(),
            worker_epoch: 1,
            owner_epoch: 1,
        };
        live().write().unwrap().insert(
            candidate.mandate_id.clone(),
            LiveGrant {
                guard: candidate.clone(),
                expires_at: Utc::now().timestamp() + 60,
                chat_ids: vec![candidate.mandate_id.clone()],
                edit_scope: None,
            },
        );
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let consuming = candidate.clone();
        let consumer = std::thread::spawn(move || {
            with_command_guard(Some(&consuming), &consuming.mandate_id, || {
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(())
            })
        });
        entered_rx.recv().unwrap();
        assert!(live().try_write().is_err());
        release_tx.send(()).unwrap();
        consumer.join().unwrap().unwrap();
        invalidate(&candidate.mandate_id).unwrap();
        let mut consumed = false;
        assert!(
            with_command_guard(Some(&candidate), &candidate.mandate_id, || {
                consumed = true;
                Ok(())
            })
            .is_err()
        );
        assert!(!consumed);
    }

    #[test]
    fn unregistered_epoch_never_restores_authority() {
        let candidate = SupervisionCommandGuard {
            mandate_id: uuid::Uuid::new_v4().to_string(),
            project_id: "p".into(),
            worker_id: "w".into(),
            engine_instance_id: "e".into(),
            worker_epoch: 1,
            owner_epoch: 1,
        };
        assert!(validate_command_guard(&candidate).is_err());
        live().write().unwrap().insert(
            candidate.mandate_id.clone(),
            LiveGrant {
                guard: candidate.clone(),
                expires_at: Utc::now().timestamp() + 60,
                chat_ids: vec![],
                edit_scope: None,
            },
        );
        assert!(validate_command_guard(&candidate).is_ok());
        let mut stale = candidate.clone();
        stale.worker_epoch += 1;
        assert!(validate_command_guard(&stale).is_err());
        invalidate(&candidate.mandate_id).unwrap();
        assert!(validate_command_guard(&candidate).is_err());
    }

    fn steering_record(child: &str) -> MandateRecord {
        let project = ProjectIdentity {
            project_id: "p".into(),
            canonical_root: "/p".into(),
        };
        let owner = NativeSourceIdentity {
            source_kind: refact_buddy_core::operator_types::NativeSourceKind::Chat,
            source_id: "user".into(),
            source_version: "1".into(),
        };
        let record = MandateRecord {
            schema_version: 1,
            project: project.clone(),
            worker: WorkerIdentity {
                worker_id: "w".into(),
                engine_instance_id: "e".into(),
                epoch: 1,
            },
            granted_by: owner.clone(),
            max_workers: 1,
            routine_choices: vec![],
            worker_tools: vec![],
            worker_target_files: vec![],
            steering_notes: vec![SteeringNote {
                note_id: "focus".into(),
                text: "Prefer the smallest diff.".into(),
            }],
            mandate: Mandate {
                schema_version: 1,
                mandate_id: uuid::Uuid::new_v4().to_string(),
                project,
                owner,
                owner_epoch: 1,
                resource_scopes: vec![],
                allowed_operations: vec!["read_project".into(), "steer_worker".into()],
                allowed_authority_classes: vec![AuthorityClass::ReadOnly],
                model_constraints: vec!["test-model".into()],
                budget_limit: None,
                expires_at: Some((Utc::now() + chrono::Duration::seconds(60)).to_rfc3339()),
                revoked_at: None,
            },
        };
        live().write().unwrap().insert(
            record.mandate.mandate_id.clone(),
            LiveGrant {
                guard: guard(&record),
                expires_at: Utc::now().timestamp() + 60,
                chat_ids: vec!["parent".into(), child.to_string()],
                edit_scope: None,
            },
        );
        record
    }

    #[test]
    fn steering_capability_requires_exact_mandate_note_and_live_grant_at_consume() {
        let child = format!("subchat-supervised-{}", uuid::Uuid::new_v4());
        let record = steering_record(&child);
        assert!(NativeSteeringCapability::mint(&record, child.clone(), "unknown").is_err());
        assert!(NativeSteeringCapability::mint(
            &record,
            child.clone(),
            "Prefer the smallest diff."
        )
        .is_err());
        assert!(NativeSteeringCapability::mint(&record, "unbound-chat".into(), "focus").is_err());
        let mut ungranted = record.clone();
        ungranted.mandate.allowed_operations = vec!["read_project".into()];
        assert!(NativeSteeringCapability::mint(&ungranted, child.clone(), "focus").is_err());
        let capability = NativeSteeringCapability::mint(&record, child.clone(), "focus").unwrap();
        assert_eq!(capability.note_id(), "focus");
        assert_eq!(
            capability.consume(&child, "test-model").unwrap(),
            "Prefer the smallest diff."
        );
        assert!(capability.consume("parent", "test-model").is_err());
        assert!(capability.consume(&child, "other-model").is_err());
        let note = capability.clone().into_native_note("control-1".into());
        assert_eq!(
            note.consume(&child, "test-model").unwrap(),
            "Prefer the smallest diff."
        );
        invalidate(&record.mandate.mandate_id).unwrap();
        assert!(note.consume(&child, "test-model").is_err());
        assert!(capability.consume(&child, "test-model").is_err());
        assert!(NativeSteeringCapability::mint(&record, child, "focus").is_err());
    }
}
