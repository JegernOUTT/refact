use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use chrono::Utc;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex as AMutex;
use uuid::Uuid;

use crate::git;
use crate::locking::{MergeAuthority, MergePermit, PhysicalGuard};
use crate::types::{
    CreateWorktreeRequest, CreateWorktreeResponse, DeleteWorktreeResponse, MergeWorktreeRequest,
    MergeWorktreeResponse, OpenWorktreeResponse, WorktreeCleanupDeleted, WorktreeCleanupPlan,
    WorktreeCleanupRequest, WorktreeCleanupResult, WorktreeCleanupSkipped, WorktreeCleanupTarget,
    WorktreeConflictState, WorktreeDiffResponse, WorktreeInspection, WorktreeInventory,
    WorktreeInventorySummary, WorktreeListResponse, WorktreeMeta, WorktreeRecordView,
    WorktreeReference, WorktreeRegistry, WorktreeRegistryRecord, WorktreeRemovalResult,
    WorktreeStatus,
};

#[path = "housekeeping.rs"]
mod housekeeping;

#[path = "sync.rs"]
mod sync;

const DEFAULT_MAX_PATCH_BYTES: usize = 200_000;

#[allow(clippy::type_complexity)]
static MERGE_LOCKS: OnceLock<Mutex<HashMap<String, Arc<AMutex<()>>>>> = OnceLock::new();

const LIST_PARALLELISM: usize = 8;
const LIST_CACHE_TTL: Duration = Duration::from_secs(2);
const PRUNE_INTERVAL: Duration = Duration::from_secs(60);

static LIST_CACHE_GENERATION: AtomicU64 = AtomicU64::new(0);
static LIST_CACHE: OnceLock<Mutex<HashMap<PathBuf, ListCacheSlot>>> = OnceLock::new();
static PRUNE_TIMES: OnceLock<Mutex<HashMap<PathBuf, Instant>>> = OnceLock::new();

type ListCacheSlot = Arc<AMutex<Option<CachedList>>>;

struct CachedList {
    response: WorktreeListResponse,
    generation: u64,
    computed_at: Instant,
}

impl CachedList {
    fn is_fresh(&self) -> bool {
        self.generation == list_cache_generation() && self.computed_at.elapsed() < LIST_CACHE_TTL
    }
}

fn list_cache_generation() -> u64 {
    LIST_CACHE_GENERATION.load(Ordering::SeqCst)
}

#[derive(Clone, Debug)]
pub struct RegistryRevision {
    generation: Arc<AtomicU64>,
    value: u64,
}

impl PartialEq for RegistryRevision {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.generation, &other.generation) && self.value == other.value
    }
}

impl Eq for RegistryRevision {}

fn registry_generation(cache_dir: &Path) -> Arc<AtomicU64> {
    static GENERATIONS: OnceLock<Mutex<HashMap<PathBuf, Weak<AtomicU64>>>> = OnceLock::new();
    let mut generations = GENERATIONS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(generation) = generations.get(cache_dir).and_then(Weak::upgrade) {
        return generation;
    }
    let key = normalize_existing_or_parent(cache_dir);
    if let Some(generation) = generations.get(&key).and_then(Weak::upgrade) {
        generations.insert(cache_dir.to_path_buf(), Arc::downgrade(&generation));
        return generation;
    }
    generations.retain(|_, generation| generation.strong_count() > 0);
    let generation = Arc::new(AtomicU64::new(0));
    generations.insert(key, Arc::downgrade(&generation));
    generations.insert(cache_dir.to_path_buf(), Arc::downgrade(&generation));
    generation
}

pub fn registry_revision(cache_dir: &Path) -> RegistryRevision {
    let generation = registry_generation(cache_dir);
    let value = generation.load(Ordering::SeqCst);
    RegistryRevision {
        generation,
        value,
    }
}

fn invalidate_list_cache() {
    LIST_CACHE_GENERATION.fetch_add(1, Ordering::SeqCst);
}

struct RegistryPublicationInvalidation(Arc<AtomicU64>);

impl Drop for RegistryPublicationInvalidation {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
        invalidate_list_cache();
    }
}

#[cfg(test)]
struct RegistryPublicationPause {
    entered: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
}

#[cfg(test)]
tokio::task_local! {
    static REGISTRY_PUBLICATION_PAUSE: Arc<RegistryPublicationPause>;
}

fn list_cache_slot(registry_path: &Path) -> ListCacheSlot {
    LIST_CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entry(registry_path.to_path_buf())
        .or_insert_with(|| Arc::new(AMutex::new(None)))
        .clone()
}

fn prune_due(source_root: &Path, force: bool) -> bool {
    let now = Instant::now();
    let mut times = PRUNE_TIMES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match times.get(source_root) {
        Some(last) if !force && now.duration_since(*last) < PRUNE_INTERVAL => false,
        _ => {
            times.insert(source_root.to_path_buf(), now);
            true
        }
    }
}

pub fn merge_lock_for(checkout: &Path) -> Arc<AMutex<()>> {
    let key = crate::locking::repository_identity(checkout)
        .or_else(|_| dunce::canonicalize(checkout).map_err(|e| e.to_string()))
        .unwrap_or_else(|_| checkout.to_path_buf())
        .to_string_lossy()
        .to_string();
    MERGE_LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entry(key)
        .or_insert_with(|| Arc::new(AMutex::new(())))
        .clone()
}

#[derive(Debug)]
struct RegistryGuard {
    _repository: Option<Arc<MergePermit>>,
    _physical: PhysicalGuard,
}

#[derive(Clone, Debug)]
pub struct KnownLandingCommit {
    pub commit: String,
    pub exact: crate::types::CheckedMergeIdentity,
}

pub type LandingCommitObserver =
    Arc<dyn Fn(KnownLandingCommit) -> Result<(), String> + Send + Sync>;

/// Dependency-inverted admission barrier. Implementations must cancel and drain
/// admitted effects before returning; an error must never authorize mutation.
pub trait WorktreeEffectFence: std::fmt::Debug + Send + Sync {
    fn begin(
        &self,
        project: PathBuf,
        roots: Vec<PathBuf>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<EffectGuard, String>> + Send + '_>>;
}

/// Owned by the actual blocking worker, not merely its cancellable awaiter.
pub type EffectGuard = Arc<dyn Send + Sync>;

#[derive(Clone, Debug, PartialEq, Eq)]
struct MergeTargetSeal {
    common_dir: PathBuf,
    branch: String,
    roots: Vec<PathBuf>,
}

#[derive(Clone)]
struct EffectToken {
    records: Vec<WorktreeMeta>,
    includes_source: bool,
    archive: Option<crate::types::WorktreeArchiveIdentity>,
    legacy: Option<WorktreeMeta>,
    target: Option<MergeTargetSeal>,
    target_materialized: Arc<AtomicBool>,
    legacy_removed: Arc<AtomicBool>,
    _guard: EffectGuard,
}

impl std::fmt::Debug for EffectToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EffectToken")
            .field("records", &self.records)
            .field("includes_source", &self.includes_source)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
pub struct WorktreeService {
    cache_dir: PathBuf,
    source_workspace_root: PathBuf,
    project_hash: String,
    merge_checkout: Option<Arc<git::MergeCheckout>>,
    nested_merge_identity: Option<(WorktreeMeta, WorktreeMeta)>,
    merge_permit: Option<Arc<MergePermit>>,
    merge_authority: Option<MergeAuthority>,
    effect_fence: Option<Arc<dyn WorktreeEffectFence>>,
    effect_token: Option<Arc<EffectToken>>,
    sync_actor: Option<String>,
}

fn recoverable_chat_record<'a>(
    registry: &'a WorktreeRegistry,
    chat_id: &str,
) -> Result<Option<&'a WorktreeRegistryRecord>, String> {
    let mut candidates = registry
        .records
        .iter()
        .filter(|record| record.creator_chat_id.as_deref() == Some(chat_id));
    let Some(record) = candidates.next() else {
        return Ok(None);
    };
    if candidates.next().is_some() {
        return Err(
            "Chat worktree recovery is ambiguous; retain checkouts for manual recovery".into(),
        );
    }
    let meta = &record.meta;
    if chat_id.is_empty()
        || meta.kind != "chat"
        || meta.branch.as_deref() != Some(default_branch_name("chat", Some(chat_id)).as_str())
        || meta.task_id.is_some()
        || meta.card_id.is_some()
        || meta.agent_id.is_some()
        || record.references.iter().any(|reference| {
            reference.kind != "chat"
                || reference.chat_id.as_deref() != Some(chat_id)
                || reference.task_id.is_some()
                || reference.card_id.is_some()
                || reference.agent_id.is_some()
        })
    {
        return Err("Chat worktree recovery refused: checkout ownership does not match".into());
    }
    Ok(Some(record))
}

impl WorktreeService {
    pub fn new(cache_dir: PathBuf, source_workspace_root: PathBuf) -> Result<Self, String> {
        let cache_dir = normalize_existing_or_parent(&cache_dir);
        let source_workspace_root = canonicalize_existing_dir(&source_workspace_root)?;
        Ok(Self::from_normalized_paths(
            cache_dir,
            source_workspace_root,
        ))
    }

    pub async fn new_async(
        cache_dir: PathBuf,
        source_workspace_root: PathBuf,
    ) -> Result<Self, String> {
        let cache_dir = normalize_existing_or_parent_async(&cache_dir).await;
        let source_workspace_root = canonicalize_existing_dir_async(&source_workspace_root).await?;
        Ok(Self::from_normalized_paths(
            cache_dir,
            source_workspace_root,
        ))
    }

    fn from_normalized_paths(cache_dir: PathBuf, source_workspace_root: PathBuf) -> Self {
        let project_hash = project_hash_for_path(&source_workspace_root);
        Self {
            cache_dir,
            source_workspace_root,
            project_hash,
            merge_checkout: None,
            nested_merge_identity: None,
            merge_permit: None,
            merge_authority: None,
            effect_fence: None,
            effect_token: None,
            sync_actor: None,
        }
    }

    /// Supply the application's coordinator explicitly. No process-global
    /// authority or path-to-runtime lookup is used by this crate.
    pub fn with_effect_fence(&self, fence: Arc<dyn WorktreeEffectFence>) -> Self {
        Self {
            effect_fence: Some(fence),
            effect_token: None,
            ..self.clone()
        }
    }

    /// Call before acquiring any repository/registry or application locks.
    /// Tokens can be reused only for the exact registered identities they fence.
    pub async fn fence_worktrees(
        &self,
        ids: &[String],
        include_source: bool,
    ) -> Result<Self, String> {
        self.fence_worktrees_inner(ids, include_source, None).await
    }

    async fn fence_worktrees_inner(
        &self,
        ids: &[String],
        include_source: bool,
        target: Option<MergeTargetSeal>,
    ) -> Result<Self, String> {
        if ids.is_empty() {
            return Err("Effect fencing requires registered worktree identities".into());
        }
        let registry = self.load_registry_unlocked().await?;
        let mut records = Vec::new();
        for id in ids {
            validate_worktree_id(id)?;
            let record = registry.records.iter().find(|r| &r.meta.id == id).ok_or(
                "Effect fencing requires a registered checkout; retain unregistered state",
            )?;
            self.validate_registered_record_checkout_record(record)
                .await?;
            let project =
                canonicalize_existing_dir_async(&record.meta.source_workspace_root).await?;
            let root = canonicalize_existing_dir_async(&record.meta.root).await?;
            if project != self.source_workspace_root
                || project != record.meta.source_workspace_root
                || root != record.meta.root
            {
                return Err("Effect fence canonical registered identity changed".into());
            }
            if !records.contains(&record.meta) {
                records.push(record.meta.clone());
            }
        }
        if let Some(token) = &self.effect_token {
            if (!include_source || token.includes_source)
                && records.iter().all(|meta| token.records.contains(meta))
                && target.as_ref().map_or(true, |target| {
                    token.target.as_ref().is_some_and(|sealed| {
                        sealed.branch == target.branch && sealed.common_dir == target.common_dir
                    })
                })
            {
                self.validate_effect_token().await?;
                return Ok(self.clone());
            }
            return Err("Existing effect token does not cover requested checkout domains".into());
        }
        if self.merge_permit.is_some() {
            return Err(
                "Acquire worktree effect fence before merge permit; checkout retained".into(),
            );
        }
        let fence = self.effect_fence.as_ref().ok_or(
            "Worktree mutation requires an application effect coordinator; checkout retained",
        )?;
        let mut roots: Vec<_> = records.iter().map(|meta| meta.root.clone()).collect();
        if include_source {
            roots.push(self.source_workspace_root.clone());
        }
        if let Some(target) = &target {
            roots.extend(target.roots.iter().cloned());
        }
        roots.sort();
        roots.dedup();
        let guard = fence
            .begin(self.source_workspace_root.clone(), roots)
            .await?;
        let service = Self {
            effect_token: Some(Arc::new(EffectToken {
                records,
                includes_source: include_source,
                archive: None,
                legacy: None,
                target,
                target_materialized: Arc::new(AtomicBool::new(false)),
                legacy_removed: Arc::new(AtomicBool::new(false)),
                _guard: guard,
            })),
            ..self.clone()
        };
        service.validate_effect_token().await?;
        Ok(service)
    }

    /// Prepare both trusted source and registered target domains before permits.
    pub async fn fence_merge_worktree(
        &self,
        id: &str,
        target_branch: Option<&str>,
    ) -> Result<Self, String> {
        let requested_target = target_branch.map(str::to_string);
        let registry = self.load_registry_unlocked().await?;
        let source = registry
            .records
            .iter()
            .find(|r| r.meta.id == id)
            .ok_or("Merge source registration missing")?;
        let target = requested_target
            .as_ref()
            .or(source.meta.base_branch.as_ref())
            .ok_or("Merge target branch missing")?;
        let mut ids = vec![id.to_string()];
        ids.extend(
            registry
                .records
                .iter()
                .filter(|r| r.meta.branch.as_ref() == Some(target))
                .map(|r| r.meta.id.clone()),
        );
        let target = self.capture_merge_target(target.clone()).await?;
        self.fence_worktrees_inner(&ids, true, Some(target)).await
    }

    /// Fence a board-authorized legacy merge, not a session binding. Callers must
    /// supply the accepted task/card attachment; this token grants no session authority.
    pub async fn fence_legacy_merge_worktree(
        &self,
        meta: &WorktreeMeta,
        target_branch: &str,
    ) -> Result<Self, String> {
        if self.merge_permit.is_some() || self.effect_token.is_some() {
            return Err("Legacy merge fencing must precede permits and other tokens".into());
        }
        let service = self.clone();
        let identity = meta.clone();
        tokio::task::spawn_blocking(move || service.validate_legacy_merge_identity(&identity))
            .await
            .map_err(|e| e.to_string())??;
        validate_branch_name(target_branch)?;
        let target = self.capture_merge_target(target_branch.to_string()).await?;
        let registry = self.load_registry_unlocked().await?;
        let mut records = Vec::new();
        for record in registry
            .records
            .iter()
            .filter(|r| r.meta.branch.as_deref() == Some(target_branch))
        {
            self.validate_registered_record_checkout_record(record)
                .await?;
            records.push(record.meta.clone());
        }
        let mut roots: Vec<_> = records.iter().map(|m| m.root.clone()).collect();
        roots.push(meta.root.clone());
        roots.push(self.source_workspace_root.clone());
        roots.extend(target.roots.iter().cloned());
        roots.sort();
        roots.dedup();
        let fence = self
            .effect_fence
            .as_ref()
            .ok_or("Legacy merge requires application effect coordinator")?;
        let guard = fence
            .begin(self.source_workspace_root.clone(), roots)
            .await?;
        let service = Self {
            effect_token: Some(Arc::new(EffectToken {
                records,
                includes_source: true,
                archive: None,
                legacy: Some(meta.clone()),
                target: Some(target),
                target_materialized: Arc::new(AtomicBool::new(false)),
                legacy_removed: Arc::new(AtomicBool::new(false)),
                _guard: guard,
            })),
            ..self.clone()
        };
        service.validate_effect_token().await?;
        Ok(service)
    }

    async fn validate_effect_token(&self) -> Result<(), String> {
        let service = self.clone();
        tokio::task::spawn_blocking(move || service.validate_effect_token_blocking())
            .await
            .map_err(|e| e.to_string())?
    }

    async fn capture_merge_target(&self, branch: String) -> Result<MergeTargetSeal, String> {
        let service = self.clone();
        tokio::task::spawn_blocking(move || service.capture_merge_target_blocking(&branch))
            .await
            .map_err(|e| e.to_string())?
    }

    fn capture_merge_target_blocking(&self, branch: &str) -> Result<MergeTargetSeal, String> {
        validate_branch_name(branch)?;
        let common_dir = source_common_dir(&self.source_workspace_root)?;
        let mut roots = Vec::new();
        for entry in git::list_git_worktrees_checked(&self.source_workspace_root)? {
            if entry.branch.as_deref() != Some(branch) {
                continue;
            }
            let root = canonicalize_existing_dir(&entry.root)?;
            let repo = git::discover_repo(&root)?;
            if canonicalize_existing_dir(&git::repo_root(&repo)?)? != root
                || git::canonical_common_dir(&repo)? != common_dir
            {
                return Err("Git merge target repository identity changed".into());
            }
            git::ensure_repo_on_branch(&repo, "Merge target", branch)?;
            roots.push(root);
        }
        roots.sort();
        roots.dedup();
        if roots.len() > 1 {
            return Err("Merge target branch is checked out more than once".into());
        }
        Ok(MergeTargetSeal {
            common_dir,
            branch: branch.into(),
            roots,
        })
    }

    fn validate_legacy_merge_identity(&self, meta: &WorktreeMeta) -> Result<(), String> {
        if meta.kind != "task_agent"
            || meta.id.is_empty()
            || meta.task_id.as_deref().unwrap_or_default().is_empty()
            || meta.card_id.as_deref().unwrap_or_default().is_empty()
            || meta.branch.as_deref().unwrap_or_default().is_empty()
            || canonicalize_existing_dir(&meta.source_workspace_root)? != self.source_workspace_root
            || canonicalize_existing_dir(&meta.root)? != meta.root
            || meta.root == self.source_workspace_root
            || !git::list_git_worktrees(&self.source_workspace_root)
                .iter()
                .any(|entry| normalized_path_key(&entry.root).ok().as_ref() == Some(&meta.root))
        {
            return Err("Legacy checkout proof changed after drain".into());
        }
        validate_worktree_meta_checkout(meta)
    }

    fn validate_effect_token_blocking(&self) -> Result<(), String> {
        let token = self
            .effect_token
            .as_ref()
            .ok_or("Worktree effect token missing")?;
        if let Some(identity) = &token.archive {
            return self.validate_archive_effect_identity(identity);
        }
        if let Some(target) = &token.target {
            let current = self.capture_merge_target_blocking(&target.branch)?;
            // Materialization may occupy a previously unoccupied branch at the
            // already-fenced source or a worker-owned private temporary target.
            let owned_root = self.merge_checkout.as_ref().filter(|checkout| {
                checkout.target_branch() == target.branch
                    && (checkout.root == self.source_workspace_root || checkout.is_temporary())
            });
            if current.common_dir != target.common_dir
                || (current.roots != target.roots
                    && !(target.roots.is_empty()
                        && token.target_materialized.load(Ordering::Acquire)
                        && owned_root
                            .is_some_and(|checkout| current.roots == vec![checkout.root.clone()])))
            {
                return Err(
                    "Git merge target identity changed after effect drain; retry with a new fence"
                        .into(),
                );
            }
            if let Some(checkout) = &self.merge_checkout {
                if checkout.target_branch() != target.branch
                    || (!target.roots.contains(&checkout.root)
                        && checkout.root != self.source_workspace_root
                        && !checkout.is_temporary())
                {
                    return Err("Resolved merge target is outside the sealed fence".into());
                }
            }
        }
        let registry: WorktreeRegistry = match std::fs::read(self.registry_path()) {
            Ok(content) => serde_json::from_slice(&content).map_err(|e| e.to_string())?,
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound && token.records.is_empty() =>
            {
                WorktreeRegistry {
                    schema_version: 1,
                    source_workspace_root: self.source_workspace_root.clone(),
                    project_hash: self.project_hash.clone(),
                    records: Vec::new(),
                }
            }
            Err(error) => return Err(error.to_string()),
        };
        if let Some(meta) = &token.legacy {
            if token.legacy_removed.load(Ordering::Acquire) {
                if meta.root.exists()
                    || git::list_git_worktrees_checked(&self.source_workspace_root)?
                        .iter()
                        .any(|entry| {
                            normalized_path_key(&entry.root).ok().as_ref() == Some(&meta.root)
                        })
                {
                    return Err("Removed legacy checkout reappeared; retain state".into());
                }
            } else {
                self.validate_legacy_merge_identity(meta)?;
            }
        }
        for meta in &token.records {
            let record =
                registry.records.iter().find(|r| r.meta == *meta).ok_or(
                    "Worktree registration changed after effect drain; no effects authorized",
                )?;
            validate_registered_record_checkout(record)?;
            if canonicalize_existing_dir(&meta.root)? != meta.root
                || canonicalize_existing_dir(&meta.source_workspace_root)?
                    != self.source_workspace_root
            {
                return Err("Worktree canonical identity changed after effect drain".into());
            }
        }
        Ok(())
    }

    fn require_source_effect_token(&self) -> Result<(), String> {
        if !self
            .effect_token
            .as_ref()
            .is_some_and(|token| token.includes_source)
        {
            return Err(
                "Merge effects require a source-domain effect token acquired before permits".into(),
            );
        }
        Ok(())
    }

    fn invalidate_registry_cache(&self) {
        registry_generation(&self.cache_dir).fetch_add(1, Ordering::SeqCst);
        invalidate_list_cache();
    }

    pub fn source_workspace_root(&self) -> &Path {
        &self.source_workspace_root
    }

    pub fn merge_checkout_root(&self) -> &Path {
        self.merge_checkout
            .as_ref()
            .map_or(self.source_workspace_root(), |checkout| &checkout.root)
    }

    pub async fn acquire_merge_permit(&self) -> Result<Self, String> {
        if let Some(permit) = &self.merge_permit {
            permit.validate(self.source_workspace_root())?;
            return Ok(self.clone());
        }
        Ok(Self {
            merge_permit: Some(MergePermit::acquire(self.source_workspace_root()).await?),
            ..self.clone()
        })
    }

    pub fn with_merge_authority(&self, validator: crate::locking::PhaseValidator) -> Self {
        let previous = self.merge_authority.clone();
        let validator = Arc::new(move || {
            if let Some(previous) = &previous {
                (previous.0)()?;
            }
            validator()
        });
        Self {
            merge_authority: Some(MergeAuthority(validator)),
            ..self.clone()
        }
    }

    fn validate_merge_phase(&self) -> Result<(), String> {
        let permit = self
            .merge_permit
            .as_ref()
            .ok_or("Managed merge permit missing")?;
        permit.validate(self.source_workspace_root())?;
        if self.merge_checkout_root().exists() {
            permit.validate(self.merge_checkout_root())?;
        }
        if let Some(authority) = &self.merge_authority {
            (authority.0)().map_err(|e| format!("Managed merge authority lost; retain source and integration state, never replay: {e}"))?;
        }
        Ok(())
    }

    pub async fn checkpoint_merge_phase(&self) -> Result<(), String> {
        let service = self.clone();
        tokio::task::spawn_blocking(move || service.validate_merge_phase())
            .await
            .map_err(|e| e.to_string())?
    }

    pub async fn run_merge_git_phase<T: Send + 'static>(
        &self,
        operation: impl FnOnce() -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        let service = self.clone();
        tokio::task::spawn_blocking(move || {
            service.require_source_effect_token()?;
            service.validate_merge_phase()?;
            service.validate_effect_token_blocking()?;
            let result = crate::locking::with_git_phase(service.merge_authority.clone(), operation);
            service.validate_merge_phase()?;
            result
        })
        .await
        .map_err(|e| e.to_string())?
    }

    /// Remove only the legacy source sealed before drain. The Git worker owns
    /// the guard across removal and branch deletion even if its awaiter aborts.
    pub async fn cleanup_legacy_merge_source(&self) -> Result<(bool, bool), String> {
        let service = self.clone();
        self.run_merge_git_phase(move || {
            let token = service
                .effect_token
                .as_ref()
                .ok_or("Legacy effect token missing")?;
            let meta = token
                .legacy
                .as_ref()
                .ok_or("Legacy source identity missing")?;
            if token.legacy_removed.load(Ordering::Acquire) {
                return Err("Legacy cleanup already performed; do not replay".into());
            }
            let branch = meta.branch.as_deref().ok_or("Legacy branch missing")?;
            if token
                .target
                .as_ref()
                .is_some_and(|target| target.branch == branch)
            {
                return Err("Legacy source equals merge target; retained".into());
            }
            git::ensure_clean_worktree(&meta.root, "Legacy cleanup source")?;
            service.validate_effect_token_blocking()?;
            git::run_git(
                &service.source_workspace_root,
                &["worktree", "remove", &meta.root.to_string_lossy()],
            )?;
            if meta.root.exists()
                || git::list_git_worktrees_checked(&service.source_workspace_root)?
                    .iter()
                    .any(|entry| normalized_path_key(&entry.root).ok().as_ref() == Some(&meta.root))
            {
                return Err("Legacy removal outcome unproven; retain branch and inspect".into());
            }
            token.legacy_removed.store(true, Ordering::Release);
            service.validate_merge_phase()?;
            service.validate_effect_token_blocking()?;
            let deleted = git::delete_branch(&service.source_workspace_root, branch)?;
            Ok((true, deleted))
        })
        .await
    }

    pub async fn materialize_merge_target(&self) -> Result<(), String> {
        let service = self.clone();
        tokio::task::spawn_blocking(move || {
            service.require_source_effect_token()?;
            service.validate_merge_phase()?;
            service.validate_effect_token_blocking()?;
            service
                .merge_checkout
                .as_ref()
                .ok_or("Merge target unresolved")?
                .materialize()?;
            service
                .effect_token
                .as_ref()
                .ok_or("Effect token missing")?
                .target_materialized
                .store(true, Ordering::Release);
            service.validate_effect_token_blocking()
        })
        .await
        .map_err(|e| e.to_string())?
    }

    pub async fn cleanup_merge_target(&self) -> Result<(), String> {
        let service = self.clone();
        tokio::task::spawn_blocking(move || {
            service.require_source_effect_token()?;
            service.validate_merge_phase()?;
            service
                .merge_checkout
                .as_ref()
                .ok_or("Merge target unresolved")?
                .cleanup()
        })
        .await
        .map_err(|e| e.to_string())?
    }

    async fn registry_write_lock(&self) -> Result<Arc<RegistryGuard>, String> {
        let repository = if crate::locking::repository_identity(self.source_workspace_root())
            .is_ok()
        {
            self.acquire_merge_permit().await?.merge_permit
        } else {
            if self
                .source_workspace_root
                .join(".git")
                .try_exists()
                .map_err(|e| e.to_string())?
            {
                return Err("Cannot prove physical Git identity for registry transaction; mutation unsupported".into());
            }
            None
        };
        let path = match &repository {
            Some(_) => crate::locking::repository_identity(self.source_workspace_root())?
                .join("refact-worktree-registry.lock"),
            None => self.registry_dir().join("index.lock"),
        };
        let physical = PhysicalGuard::acquire(&path).await?;
        self.refuse_ambiguous_registry_aliases().await?;
        Ok(Arc::new(RegistryGuard {
            _repository: repository,
            _physical: physical,
        }))
    }

    async fn refuse_ambiguous_registry_aliases(&self) -> Result<(), String> {
        let Some(parent) = self.registry_dir().parent().map(Path::to_path_buf) else {
            return Ok(());
        };
        let mut entries = match tokio::fs::read_dir(parent).await {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.to_string()),
        };
        let mut count = 0;
        while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
            count += 1;
            if count > 4096 {
                return Err(
                    "Registry alias inventory exceeds safe bound; mutation unsupported".into(),
                );
            }
            if entry.file_name() == std::ffi::OsStr::new(&self.project_hash) {
                continue;
            }
            let path = entry.path().join("index.json");
            let content = match tokio::fs::read(&path).await {
                Ok(content) => content,
                Err(e)
                    if e.kind() == std::io::ErrorKind::NotFound
                        || e.kind() == std::io::ErrorKind::NotADirectory =>
                {
                    continue
                }
                Err(e) => return Err(format!("Cannot exclude ambiguous registry alias: {e}")),
            };
            let other: WorktreeRegistry = serde_json::from_slice(&content)
                .map_err(|e| format!("Cannot exclude ambiguous registry alias: {e}"))?;
            if normalized_path_key_async(&other.source_workspace_root).await?
                == self.source_workspace_root
            {
                return Err("Multiple legacy registry indices resolve to this physical project; inspect without merging or deleting metadata".into());
            }
        }
        Ok(())
    }

    pub fn with_nested_merge_identity(&self, source: WorktreeMeta, parent: WorktreeMeta) -> Self {
        Self {
            nested_merge_identity: Some((source, parent)),
            ..self.clone()
        }
    }

    pub async fn for_merge_target(&self, target: &str) -> Result<Self, String> {
        validate_branch_name(target)?;
        if self.effect_token.is_some() {
            self.validate_effect_token().await?;
        }
        if let Some(checkout) = &self.merge_checkout {
            if checkout.target_branch() == target {
                return Ok(self.clone());
            }
            return Err("Merge target changed on an already resolved service".into());
        }
        let source = self.source_workspace_root.clone();
        let target = target.to_string();
        let checkout =
            tokio::task::spawn_blocking(move || git::prepare_merge_checkout(&source, &target))
                .await
                .map_err(|error| format!("Merge target resolution failed: {error}"))??;
        let service = Self {
            merge_checkout: Some(Arc::new(checkout)),
            ..self.clone()
        };
        if service.effect_token.is_some() {
            service.validate_effect_token().await?;
        }
        Ok(service)
    }

    pub fn project_hash(&self) -> &str {
        &self.project_hash
    }

    pub fn registry_dir(&self) -> PathBuf {
        self.cache_dir.join("worktrees").join(&self.project_hash)
    }

    pub fn registry_path(&self) -> PathBuf {
        self.registry_dir().join("index.json")
    }

    pub fn worktree_path_for_id(&self, id: &str) -> Result<PathBuf, String> {
        validate_worktree_id(id)?;
        let path = self.registry_dir().join(id);
        if !path.starts_with(self.registry_dir()) {
            return Err("Resolved worktree path escapes registry directory".to_string());
        }
        Ok(path)
    }

    pub async fn load_registry(&self) -> Result<WorktreeRegistry, String> {
        self.load_registry_unlocked().await
    }

    pub async fn compare_and_save_registry(
        &self,
        expected: &WorktreeRegistry,
        registry: &WorktreeRegistry,
    ) -> Result<(), String> {
        let _guard = self.registry_write_lock().await?;
        self.validate_registry(registry).await?;
        if self.load_registry_unlocked().await? != *expected {
            return Err("Worktree registry snapshot changed; reload before applying edits".into());
        }
        self.save_registry_unlocked(registry, &_guard).await
    }

    async fn load_registry_for_listing(&self) -> Result<WorktreeRegistry, String> {
        let registry = self.load_registry_unlocked().await?;
        let mut missing = HashSet::new();
        for record in &registry.records {
            if !tokio::fs::try_exists(&record.meta.root)
                .await
                .unwrap_or(true)
            {
                missing.insert(record.meta.id.clone());
            }
        }
        if !prune_due(&self.source_workspace_root, !missing.is_empty()) {
            return Ok(registry);
        }
        let _repository_guard = self.acquire_merge_permit().await?;
        let source_root = self.source_workspace_root.clone();
        let repository_guard = _repository_guard.clone();
        let git_roots = tokio::task::spawn_blocking(move || {
            repository_guard.validate_merge_phase()?;
            git::prune_worktrees(&source_root)?;
            repository_guard.validate_merge_phase()?;
            Ok::<HashSet<PathBuf>, String>(
                git::list_git_worktrees(&source_root)
                    .into_iter()
                    .filter_map(|entry| normalized_path_key(&entry.root).ok())
                    .collect(),
            )
        })
        .await
        .map_err(|error| format!("Git worktree reconciliation task failed: {error}"))??;
        let is_stale = |record: &WorktreeRegistryRecord| {
            missing.contains(&record.meta.id)
                && !normalized_path_key(&record.meta.root)
                    .map(|root| git_roots.contains(&root))
                    .unwrap_or(true)
        };
        if !registry.records.iter().any(is_stale) {
            return Ok(registry);
        }
        let _guard = _repository_guard.registry_write_lock().await?;
        let mut registry = self.load_registry_unlocked().await?;
        let before = registry.records.len();
        let mut retained = Vec::with_capacity(before);
        for record in registry.records.drain(..) {
            let exists = tokio::fs::try_exists(&record.meta.root)
                .await
                .unwrap_or(true);
            let registered_in_git = normalized_path_key(&record.meta.root)
                .map(|root| git_roots.contains(&root))
                .unwrap_or(true);
            if exists || registered_in_git {
                retained.push(record);
            }
        }
        registry.records = retained;
        if registry.records.len() != before {
            self.save_registry_unlocked(&registry, &_guard).await?;
        }
        Ok(registry)
    }

    pub async fn list_worktrees(&self) -> Result<WorktreeListResponse, String> {
        self.list_worktrees_with_branches(true).await
    }

    pub async fn list_worktrees_with_branches(
        &self,
        include_branches: bool,
    ) -> Result<WorktreeListResponse, String> {
        let slot = list_cache_slot(&self.registry_path());
        let mut response = {
            let mut cached = slot.lock().await;
            match cached.as_ref() {
                Some(entry) if entry.is_fresh() => entry.response.clone(),
                _ => {
                    let generation = list_cache_generation();
                    let response = self.compute_worktree_list().await?;
                    *cached = Some(CachedList {
                        response: response.clone(),
                        generation,
                        computed_at: Instant::now(),
                    });
                    response
                }
            }
        };
        if include_branches {
            let source_root = self.source_workspace_root.clone();
            response.source_branches =
                tokio::task::spawn_blocking(move || match git::discover_repo(&source_root) {
                    Ok(repo) => git::local_branches(&repo),
                    Err(_) => Vec::new(),
                })
                .await
                .map_err(|e| format!("Worktree branch listing task failed: {}", e))?;
        }
        Ok(response)
    }

    async fn compute_worktree_list(&self) -> Result<WorktreeListResponse, String> {
        let registry = self.load_registry_for_listing().await?;
        let records = Arc::new(registry.records);
        let mut source_common_dirs: HashMap<PathBuf, PathBuf> = HashMap::new();
        for record in records.iter() {
            let source = record.meta.source_workspace_root.clone();
            if !source_common_dirs.contains_key(&source) {
                let lookup = source.clone();
                let common_dir = tokio::task::spawn_blocking(move || source_common_dir(&lookup))
                    .await
                    .map_err(|e| format!("Worktree source inspection task failed: {}", e))??;
                source_common_dirs.insert(source, common_dir);
            }
        }
        let source_common_dirs = Arc::new(source_common_dirs);
        let permits = Arc::new(tokio::sync::Semaphore::new(LIST_PARALLELISM));
        let mut tasks = tokio::task::JoinSet::new();
        for index in 0..records.len() {
            let records = records.clone();
            let source_common_dirs = source_common_dirs.clone();
            let permits = permits.clone();
            tasks.spawn(async move {
                let _permit = permits
                    .acquire_owned()
                    .await
                    .map_err(|e| format!("Worktree listing semaphore closed: {}", e))?;
                let status = tokio::task::spawn_blocking(move || {
                    let meta = &records[index].meta;
                    let common_dir = source_common_dirs
                        .get(&meta.source_workspace_root)
                        .ok_or_else(|| "Worktree source common dir missing".to_string())?;
                    inspect_listed_record(meta, common_dir)
                })
                .await
                .map_err(|e| format!("Worktree validation task failed: {}", e))?;
                Ok::<_, String>((index, status))
            });
        }
        let mut statuses: Vec<Option<Result<WorktreeStatus, String>>> =
            (0..records.len()).map(|_| None).collect();
        while let Some(joined) = tasks.join_next().await {
            let (index, status) =
                joined.map_err(|e| format!("Worktree listing task failed: {}", e))??;
            statuses[index] = Some(status);
        }
        let mut worktrees = Vec::with_capacity(records.len());
        for (record, status) in records.iter().zip(statuses) {
            let status = status.ok_or_else(|| "Worktree listing task missing".to_string())??;
            worktrees.push(self.record_view_with_status(record, status).await?);
        }
        worktrees.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        let source_root = self.source_workspace_root.clone();
        let source_current_branch =
            tokio::task::spawn_blocking(move || match git::discover_repo(&source_root) {
                Ok(repo) => git::current_branch(&repo),
                Err(_) => None,
            })
            .await
            .map_err(|e| format!("Worktree source inspection task failed: {}", e))?;
        Ok(WorktreeListResponse {
            project_hash: self.project_hash.clone(),
            source_workspace_root: self.source_workspace_root.clone(),
            source_current_branch,
            source_branches: Vec::new(),
            worktrees,
        })
    }

    pub async fn get_worktree(&self, id: &str) -> Result<WorktreeRecordView, String> {
        validate_worktree_id(id)?;
        let registry = self.load_registry_unlocked().await?;
        let record = registry
            .records
            .iter()
            .find(|record| record.meta.id == id)
            .ok_or_else(|| format!("Worktree '{}' not found", id))?;
        self.validate_registered_record_checkout_record(record)
            .await?;
        self.record_view(record).await
    }

    pub async fn validate_registered_worktree_checkout(&self, id: &str) -> Result<(), String> {
        validate_worktree_id(id)?;
        let registry = self.load_registry_unlocked().await?;
        let record = registry
            .records
            .iter()
            .find(|record| record.meta.id == id)
            .ok_or_else(|| format!("Worktree '{}' not found", id))?;
        self.validate_registered_record_checkout_record(record)
            .await
    }

    pub async fn recover_chat_worktree(
        &self,
        chat_id: &str,
    ) -> Result<Option<WorktreeMeta>, String> {
        let _guard = self.registry_write_lock().await?;
        let registry = self.load_registry_unlocked().await?;
        let Some(record) = recoverable_chat_record(&registry, chat_id)? else {
            return Ok(None);
        };
        let meta = &record.meta;
        let source = self.source_workspace_root.clone();
        let source_repo_root =
            tokio::task::spawn_blocking(move || git::repo_root(&git::discover_repo(&source)?))
                .await
                .map_err(|error| format!("Chat worktree source validation failed: {error}"))??;
        if normalized_path_key_async(&meta.repo_root).await?
            != normalized_path_key_async(&source_repo_root).await?
        {
            return Err("Chat worktree recovery refused: repository root does not match".into());
        }
        self.validate_worktree_meta_strict(meta).await.map(Some)
    }

    pub async fn add_recovered_chat_reference(
        &self,
        id: &str,
        chat_id: &str,
        expected: &WorktreeMeta,
        reference: WorktreeReference,
    ) -> Result<WorktreeRecordView, String> {
        let _guard = self.registry_write_lock().await?;
        let mut registry = self.load_registry_unlocked().await?;
        let record = recoverable_chat_record(&registry, chat_id)?
            .ok_or("Chat worktree recovery provenance is unavailable")?;
        if record.meta != *expected
            || record.meta.id != id
            || reference
                != (WorktreeReference {
                    kind: "chat".into(),
                    chat_id: Some(chat_id.into()),
                    ..Default::default()
                })
        {
            return Err("Chat worktree recovery identity changed".into());
        }
        self.validate_worktree_meta_strict(expected).await?;
        let record = registry
            .records
            .iter_mut()
            .find(|record| record.meta.id == id)
            .ok_or("Chat worktree recovery record disappeared")?;
        if !record.references.contains(&reference) {
            record.references.push(reference);
            record.updated_at = Utc::now().to_rfc3339();
        }
        let record = record.clone();
        self.save_registry_unlocked(&registry, &_guard).await?;
        self.record_view(&record).await
    }

    /// Create a new registered checkout from immutable review bytes; never reuse an old checkout.
    pub async fn create_review_snapshot(
        &self,
        chat_id: &str,
        review_id: &str,
        files: Vec<crate::review::SnapshotFile>,
    ) -> Result<CreateWorktreeResponse, String> {
        if chat_id.is_empty() || review_id.is_empty() {
            return Err("Review and chat identities required".into());
        }
        let repository_guard = self.acquire_merge_permit().await?;
        let guard = repository_guard.registry_write_lock().await?;
        let mut registry = self.load_registry_unlocked().await?;
        let id = self.next_worktree_id(&registry).await?;
        let path = self.worktree_path_for_id(&id)?;
        tokio::fs::create_dir_all(self.registry_dir())
            .await
            .map_err(|e| e.to_string())?;
        let branch = format!("refact/review/{id}");
        let source = self.source_workspace_root.clone();
        let destination = path.clone();
        let name = id.clone();
        let branch_name = branch.clone();
        let held = guard.clone();
        let (commit, repo_root) = tokio::task::spawn_blocking(move || {
            let _held = held;
            crate::review::materialize(&source, &destination, &name, &branch_name, files)
        })
        .await
        .map_err(|e| e.to_string())??;
        let now = Utc::now().to_rfc3339();
        let status = WorktreeStatus {
            path_exists: true,
            is_git_worktree: true,
            branch: Some(branch.clone()),
            head_commit: Some(commit.clone()),
            ..Default::default()
        };
        let record = WorktreeRegistryRecord {
            creator_chat_id: Some(chat_id.into()),
            meta: WorktreeMeta {
                id,
                kind: "review".into(),
                root: normalized_path_key_async(&path).await?,
                source_workspace_root: self.source_workspace_root.clone(),
                repo_root,
                branch: Some(branch),
                base_branch: None,
                base_commit: Some(commit),
                task_id: None,
                card_id: None,
                agent_id: None,
                enforce: true,
            },
            created_at: now.clone(),
            updated_at: now.clone(),
            last_seen_at: Some(now),
            references: vec![WorktreeReference {
                kind: format!("review:{review_id}"),
                chat_id: Some(chat_id.into()),
                ..Default::default()
            }],
            last_known_status: Some(status.clone()),
        };
        registry.records.push(record.clone());
        self.save_registry_unlocked(&registry, &guard)
            .await
            .map_err(|e| {
                format!(
                    "{e}; review checkout {} retained for recovery",
                    path.display()
                )
            })?;
        invalidate_list_cache();
        Ok(CreateWorktreeResponse {
            worktree: self.record_view_with_status(&record, status).await?,
            branch_was_created: true,
            dirty_source_warning: false,
            warnings: vec![],
        })
    }

    pub async fn create_chat_worktree(
        &self,
        chat_id: &str,
    ) -> Result<CreateWorktreeResponse, String> {
        if chat_id.is_empty() {
            return Err("Chat identity cannot be empty".into());
        }
        self.create_worktree_inner(
            CreateWorktreeRequest {
                kind: Some("chat".into()),
                chat_id: Some(chat_id.into()),
                ..Default::default()
            },
            Some(chat_id.into()),
        )
        .await
    }

    pub async fn create_worktree(
        &self,
        request: CreateWorktreeRequest,
    ) -> Result<CreateWorktreeResponse, String> {
        self.create_worktree_inner(request, None).await
    }

    async fn create_worktree_inner(
        &self,
        request: CreateWorktreeRequest,
        creator_chat_id: Option<String>,
    ) -> Result<CreateWorktreeResponse, String> {
        let kind = validate_kind(request.kind.as_deref().unwrap_or("chat"))?;
        let branch = match request.branch.clone() {
            Some(branch) => {
                validate_branch_name(&branch)?;
                branch
            }
            None => default_branch_name(&kind, request.chat_id.as_deref()),
        };
        validate_branch_name(&branch)?;
        if let Some(base) = request.base_branch.as_deref() {
            validate_branch_name(base)?;
        }

        let _repository_guard = self.acquire_merge_permit().await?;
        let _guard = _repository_guard.registry_write_lock().await?;
        let mut registry = self.load_registry_unlocked().await?;
        let id = self.next_worktree_id(&registry).await?;
        let worktree_path = self.worktree_path_for_id(&id)?;
        if tokio::fs::try_exists(&worktree_path).await.unwrap_or(false) {
            return Err(format!(
                "Worktree path '{}' already exists",
                worktree_path.display()
            ));
        }
        let parent = worktree_path
            .parent()
            .ok_or_else(|| "Worktree path has no parent".to_string())?;
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed to create worktree parent: {}", e))?;

        let create_source = self.source_workspace_root.clone();
        let create_worktree_path = worktree_path.clone();
        let create_id = id.clone();
        let create_branch = branch.clone();
        let create_base_branch = request.base_branch.clone();
        let create_base_commit = request.base_commit.clone();
        let registry_guard = _guard.clone();
        let repository_guard = _repository_guard.clone();
        let (created, status) = match tokio::task::spawn_blocking(move || {
            let _registry_guard = registry_guard;
            let _repository_guard = repository_guard;
            let created = git::create_worktree(
                &create_source,
                &create_worktree_path,
                &create_id,
                &create_branch,
                create_base_branch.as_deref(),
                create_base_commit.as_deref(),
            )?;
            let status = git::status_for_path(&create_worktree_path);
            Ok::<_, String>((created, status))
        })
        .await
        {
            Ok(result) => result?,
            Err(e) => return Err(format!("Worktree creation task failed: {}", e)),
        };
        let now = Utc::now().to_rfc3339();
        let reference = request_to_reference(&kind, &request);
        let references = reference.into_iter().collect::<Vec<_>>();
        let meta = WorktreeMeta {
            id: id.clone(),
            kind,
            root: normalized_path_key_async(&worktree_path).await?,
            source_workspace_root: self.source_workspace_root.clone(),
            repo_root: created.repo_root,
            branch: Some(branch.clone()),
            base_branch: created.base_branch,
            base_commit: Some(created.base_commit),
            task_id: request.task_id.clone(),
            card_id: request.card_id.clone(),
            agent_id: request.agent_id.clone(),
            enforce: true,
        };
        let record = WorktreeRegistryRecord {
            creator_chat_id,
            meta,
            created_at: now.clone(),
            updated_at: now,
            last_seen_at: Some(Utc::now().to_rfc3339()),
            references,
            last_known_status: Some(status.clone()),
        };
        registry.records.push(record.clone());
        if let Err(e) = self.save_registry_unlocked(&registry, &_guard).await {
            return Err(format!(
                "Failed to publish worktree registry: {}; created checkout '{}' and branch '{}' retained for inspection; never replay creation blindly",
                e, worktree_path.display(), branch
            ));
        }
        drop(_guard);
        drop(_repository_guard);
        let cancellation = crate::cache::PreparationCancellation::new();
        let cancelled = cancellation.flag();
        let cache_service = self.clone();
        let cache_meta = record.meta.clone();
        let cache_report = tokio::task::spawn_blocking(move || {
            cache_service
                .cache_execution_context_blocking(&cache_meta)
                .map(|context| context.prepare(cancelled))
                .unwrap_or_else(crate::cache::degraded)
        })
        .await
        .unwrap_or_else(|_| crate::cache::degraded(crate::types::CacheReason::IoFailure));
        drop(cancellation);
        self.invalidate_registry_cache();

        let mut warnings = Vec::new();
        if created.dirty_source {
            warnings.push(
                "Source checkout has uncommitted changes; worktree was created from committed HEAD/base only"
                    .to_string(),
            );
        }
        let mut worktree = self.record_view_with_status(&record, status).await?;
        worktree.cache_report = Some(cache_report);
        Ok(CreateWorktreeResponse {
            worktree,
            branch_was_created: created.branch_was_created,
            dirty_source_warning: created.dirty_source,
            warnings,
        })
    }

    pub async fn add_reference(
        &self,
        id: &str,
        reference: WorktreeReference,
    ) -> Result<WorktreeRecordView, String> {
        validate_worktree_id(id)?;
        if !reference.has_identity() {
            return Err("Worktree reference must include at least one id".to_string());
        }
        let _guard = self.registry_write_lock().await?;
        let mut registry = self.load_registry_unlocked().await?;
        let (record, changed) = {
            let record = registry
                .records
                .iter_mut()
                .find(|record| record.meta.id == id)
                .ok_or_else(|| format!("Worktree '{}' not found", id))?;
            let changed = !record.references.contains(&reference);
            if changed {
                record.references.push(reference);
                record.updated_at = Utc::now().to_rfc3339();
            }
            (record.clone(), changed)
        };
        if changed {
            self.save_registry_unlocked(&registry, &_guard).await?;
        }
        drop(_guard);
        self.record_view(&record).await
    }

    pub async fn remove_reference(
        &self,
        id: &str,
        reference: &WorktreeReference,
    ) -> Result<WorktreeRecordView, String> {
        validate_worktree_id(id)?;
        if !reference.has_identity() {
            return Err("Worktree reference must include at least one id".to_string());
        }
        let _guard = self.registry_write_lock().await?;
        let mut registry = self.load_registry_unlocked().await?;
        let (record, changed) = {
            let record = registry
                .records
                .iter_mut()
                .find(|record| record.meta.id == id)
                .ok_or_else(|| format!("Worktree '{}' not found", id))?;
            let before = record.references.len();
            record.references.retain(|item| item != reference);
            let changed = record.references.len() != before;
            if changed {
                record.updated_at = Utc::now().to_rfc3339();
            }
            (record.clone(), changed)
        };
        if changed {
            self.save_registry_unlocked(&registry, &_guard).await?;
        }
        drop(_guard);
        self.record_view(&record).await
    }

    pub async fn validate_worktree_meta(
        &self,
        meta: &WorktreeMeta,
    ) -> Result<WorktreeMeta, String> {
        self.validate_worktree_meta_strict(meta).await
    }

    pub async fn validate_worktree_meta_strict(
        &self,
        meta: &WorktreeMeta,
    ) -> Result<WorktreeMeta, String> {
        validate_worktree_id(&meta.id)?;
        let meta_source = canonicalize_existing_dir_async(&meta.source_workspace_root).await?;
        if normalized_path_key_async(&meta_source).await?
            != normalized_path_key_async(&self.source_workspace_root).await?
        {
            return Err("Worktree source root does not match current workspace".to_string());
        }
        let meta_root = normalized_path_key_async(&meta.root).await?;
        let registry = self.load_registry_unlocked().await?;
        let record = registry
            .records
            .iter()
            .find(|record| record.meta.id == meta.id)
            .cloned()
            .ok_or_else(|| format!("Worktree '{}' is not registered", meta.id))?;
        let record_root = normalized_path_key_async(&record.meta.root).await?;
        if record_root != meta_root {
            return Err(format!(
                "Worktree '{}' root mismatch: '{}' != '{}'",
                meta.id,
                meta_root.display(),
                record_root.display()
            ));
        }
        let record_source =
            canonicalize_existing_dir_async(&record.meta.source_workspace_root).await?;
        if normalized_path_key_async(&record_source).await?
            != normalized_path_key_async(&meta_source).await?
        {
            return Err(format!("Worktree '{}' source root mismatch", meta.id));
        }
        let status_root = record.meta.root.clone();
        let status = tokio::task::spawn_blocking(move || git::status_for_path(&status_root))
            .await
            .map_err(|e| format!("Worktree status task failed: {}", e))?;
        if !status.path_exists {
            return Err(format!(
                "Worktree '{}' root '{}' is registered but no longer exists on disk",
                meta.id,
                record.meta.root.display()
            ));
        }
        if !status.is_git_worktree {
            return Err(format!(
                "Worktree '{}' root '{}' exists but is not a git worktree",
                meta.id,
                record.meta.root.display()
            ));
        }
        self.validate_registered_record_checkout_record(&record)
            .await?;
        Ok(record.meta.clone())
    }

    pub async fn validate_legacy_task_agent_worktree_meta(
        &self,
        meta: &WorktreeMeta,
    ) -> Result<WorktreeMeta, String> {
        validate_worktree_id(&meta.id)?;
        if meta.kind != "task_agent" {
            return Err("Legacy worktree metadata must be task_agent kind".to_string());
        }
        if meta.task_id.as_deref().unwrap_or_default().is_empty()
            || meta.card_id.as_deref().unwrap_or_default().is_empty()
            || meta.agent_id.as_deref().unwrap_or_default().is_empty()
        {
            return Err("Legacy task-agent worktree metadata is missing identity".to_string());
        }
        let meta_source = canonicalize_existing_dir_async(&meta.source_workspace_root).await?;
        if normalized_path_key_async(&meta_source).await?
            != normalized_path_key_async(&self.source_workspace_root).await?
        {
            return Err("Worktree source root does not match current workspace".to_string());
        }
        let meta_root = normalized_path_key_async(&meta.root).await?;
        if let Ok(validated) = self.validate_worktree_meta_strict(meta).await {
            return Ok(validated);
        }
        if normalized_path_key_async(&meta_source).await? == meta_root {
            return Err(format!(
                "Legacy worktree '{}' root equals the source workspace root '{}'; \
                 refusing to bind a task-agent session to the source workspace.",
                meta.id,
                meta_root.display()
            ));
        }
        let cache_root = normalized_path_key_async(&self.registry_dir()).await?;
        if !meta_root.starts_with(&cache_root) {
            return Err(format!(
                "Legacy worktree '{}' root '{}' is outside the Refact worktree cache '{}'; \
                 refusing to bind a task-agent session to it.",
                meta.id,
                meta_root.display(),
                cache_root.display()
            ));
        }
        let source_root = self.source_workspace_root.clone();
        let status_root = meta_root.clone();
        let expected_root = meta_root.clone();
        let (discovered, status) = tokio::task::spawn_blocking(move || {
            let discovered = git::list_git_worktrees(&source_root)
                .into_iter()
                .any(|entry| {
                    normalized_path_key(&entry.root)
                        .map(|root| root == expected_root)
                        .unwrap_or(false)
                });
            (discovered, git::status_for_path(&status_root))
        })
        .await
        .map_err(|e| format!("Legacy worktree validation task failed: {}", e))?;
        if discovered && status.path_exists && status.is_git_worktree {
            let validation_meta = meta.clone();
            tokio::task::spawn_blocking(move || validate_worktree_meta_checkout(&validation_meta))
                .await
                .map_err(|e| format!("Legacy worktree validation task failed: {}", e))??;
            return Ok(meta.clone());
        }
        Err(format!(
            "Worktree '{}' is not registered or discoverable for current workspace",
            meta.id
        ))
    }

    pub async fn diff_worktree(&self, id: &str) -> Result<WorktreeDiffResponse, String> {
        self.diff_worktree_with_limit(id, DEFAULT_MAX_PATCH_BYTES)
            .await
    }

    pub async fn diff_worktree_with_limit(
        &self,
        id: &str,
        max_patch_bytes: usize,
    ) -> Result<WorktreeDiffResponse, String> {
        validate_worktree_id(id)?;
        let registry = self.load_registry_unlocked().await?;
        let record = registry
            .records
            .iter()
            .find(|record| record.meta.id == id)
            .cloned()
            .ok_or_else(|| format!("Worktree '{}' not found", id))?;
        self.validate_registered_record_diff_record(&record).await?;
        let diff_root = record.meta.root.clone();
        let base_commit = record.meta.base_commit.clone();
        let base_branch = record.meta.base_branch.clone();
        let (diff, status, ahead, behind) = tokio::task::spawn_blocking(move || {
            let diff = git::diff_for_path(
                &diff_root,
                base_commit.as_deref(),
                base_branch.as_deref(),
                max_patch_bytes,
            )?;
            let status = git::status_for_path(&diff_root);
            let (ahead, behind) = git::base_relative_ahead_behind(
                &diff_root,
                base_commit.as_deref(),
                base_branch.as_deref(),
            )
            .map(|(ahead, behind)| (Some(ahead), Some(behind)))
            .unwrap_or((None, None));
            Ok::<_, String>((diff, status, ahead, behind))
        })
        .await
        .map_err(|e| format!("Worktree diff task failed: {}", e))??;
        Ok(WorktreeDiffResponse {
            resolved_base_commit: diff.resolved_base_commit,
            id: id.to_string(),
            branch: record.meta.branch.clone(),
            base_branch: record.meta.base_branch.clone(),
            base_commit: record.meta.base_commit.clone(),
            ahead,
            behind,
            status,
            files: diff.files,
            stats: diff.stats,
            patch: diff.patch,
            patch_truncated: diff.patch_truncated,
            patch_shown_bytes: diff.patch_shown_bytes,
            patch_total_bytes: diff.patch_total_bytes,
        })
    }

    pub async fn merge_worktree(
        &self,
        id: &str,
        request: MergeWorktreeRequest,
    ) -> Result<MergeWorktreeResponse, String> {
        self.merge_worktree_checked_inner(id, request, None, None)
            .await
    }

    pub async fn merge_worktree_checked(
        &self,
        id: &str,
        request: MergeWorktreeRequest,
        expected: crate::types::CheckedMergeIdentity,
    ) -> Result<MergeWorktreeResponse, String> {
        self.checked_landing(id, request, expected, None).await
    }

    pub async fn merge_worktree_checked_observed(
        &self,
        id: &str,
        request: MergeWorktreeRequest,
        expected: crate::types::CheckedMergeIdentity,
        observer: LandingCommitObserver,
    ) -> Result<MergeWorktreeResponse, String> {
        if request.strategy != crate::types::WorktreeMergeStrategy::Squash {
            return Err("Observed checked landing requires squash strategy".into());
        }
        self.checked_landing(id, request, expected, Some(observer))
            .await
    }

    async fn checked_landing(
        &self,
        id: &str,
        request: MergeWorktreeRequest,
        expected: crate::types::CheckedMergeIdentity,
        observer: Option<LandingCommitObserver>,
    ) -> Result<MergeWorktreeResponse, String> {
        if self.merge_authority.is_none() {
            return Err("Checked landing unsupported without persisted merge phase authority; source retained".into());
        }
        if request.include_uncommitted
            || request.delete_after_merge
            || request.target_branch.as_deref() != Some(expected.target_branch.as_str())
        {
            return Err(
                "Checked landing requires explicit target and retained clean source".into(),
            );
        }
        self.merge_worktree_checked_inner(id, request, Some(expected), observer)
            .await
    }

    async fn merge_worktree_checked_inner(
        &self,
        id: &str,
        request: MergeWorktreeRequest,
        expected: Option<crate::types::CheckedMergeIdentity>,
        observer: Option<LandingCommitObserver>,
    ) -> Result<MergeWorktreeResponse, String> {
        let cleanup_target = self.merge_permit.is_none();
        let service = self
            .fence_merge_worktree(id, request.target_branch.as_deref())
            .await?;
        let id = id.to_string();
        async move {
            validate_worktree_id(&id)?;
            if let Some(target_branch) = request.target_branch.as_deref() {
                validate_branch_name(target_branch)?;
            }
            let service = service.acquire_merge_permit().await?;
            let target = request.target_branch.clone();
            let target = match target {
                Some(target) => target,
                None => service
                    .get_worktree(&id)
                    .await?
                    .meta
                    .base_branch
                    .ok_or("Merge target branch missing")?,
            };
            let service = if service.merge_checkout.is_some() {
                service
            } else {
                git::ensure_dirty_worktree_on_branch(
                    service.source_workspace_root(),
                    "Target workspace",
                    &target,
                )?;
                service.for_merge_target(&target).await?
            };
            let _registry_guard = service.registry_write_lock().await?;
            service.checkpoint_merge_phase().await?;
            service.validate_effect_token().await?;
            service.materialize_merge_target().await?;
            let registry = service.load_registry_unlocked().await?;
            if let Some(expected) = &expected {
                service.validate_checked_merge(&id, expected).await?;
            }
            let blocking_service = service.clone();
            let blocking_id = id.clone();
            let registry_guard = _registry_guard.clone();
            let merged = tokio::task::spawn_blocking(move || {
                let _registry_guard = registry_guard;
                blocking_service.validate_effect_token_blocking()?;
                let mut registry = registry;
                let mut registry_changed = false;
                let response = blocking_service.merge_worktree_inner_blocking(
                    &blocking_id,
                    request,
                    &mut registry,
                    &mut registry_changed,
                    expected.zip(observer),
                )?;
                Ok::<_, String>((response, registry_changed.then_some(registry)))
            })
            .await;
            service.invalidate_registry_cache();
            let (response, updated_registry) =
                merged.map_err(|e| format!("Worktree merge blocking task failed: {}", e))??;
            service.checkpoint_merge_phase().await?;
            if let Some(registry) = updated_registry {
                service
                    .save_registry_unlocked(&registry, &_registry_guard)
                    .await?;
            }
            if cleanup_target {
                service.cleanup_merge_target().await?;
            }
            Ok(response)
        }
        .await
    }

    pub async fn validate_checked_merge(
        &self,
        id: &str,
        expected: &crate::types::CheckedMergeIdentity,
    ) -> Result<(), String> {
        let view = self.get_worktree(id).await?;
        let diff = self.diff_worktree(id).await?;
        let root = self.merge_checkout_root().to_path_buf();
        let target = tokio::task::spawn_blocking(move || git::status_for_path(&root))
            .await
            .map_err(|e| e.to_string())?;
        if expected.target_files.is_empty()
            || expected.target_files.len() > 100
            || view.status.dirty
            || view.status.conflicted
            || view.status.error.is_some()
            || target.dirty
            || target.conflicted
            || target.error.is_some()
            || target.branch.as_deref() != Some(expected.target_branch.as_str())
            || target.head_commit.as_deref() != Some(expected.target_head.as_str())
            || view.status.head_commit.as_deref() != Some(expected.source_head.as_str())
            || view.meta.base_commit.as_deref() != Some(expected.target_head.as_str())
            || view.meta.base_branch.as_deref() != Some(expected.target_branch.as_str())
            || diff.patch_truncated
            || diff.files.is_empty()
            || diff
                .files
                .iter()
                .any(|file| !expected.target_files.contains(&file.path))
            || format!(
                "{:x}",
                Sha256::digest(serde_json::to_vec(&diff).map_err(|e| e.to_string())?)
            ) != expected.patch_digest
        {
            return Err(
                "Checked landing source, exact patch, target head or clean precondition changed"
                    .into(),
            );
        }
        Ok(())
    }

    pub async fn recover_checked_landing(
        &self,
        id: &str,
        expected: crate::types::CheckedMergeIdentity,
        expected_post: String,
    ) -> Result<String, String> {
        if self.merge_authority.is_none() {
            return Err("Checked recovery unsupported without persisted merge phase authority; source retained".into());
        }
        let service = self.fence_worktrees(&[id.to_string()], true).await?;
        let id = id.to_string();
        async move {
            let service = service.acquire_merge_permit().await?;
            let _registry_guard = service.registry_write_lock().await?;
            let view = service.get_worktree(&id).await?;
            if view.status.dirty || view.status.conflicted || view.status.error.is_some()
                || view.status.head_commit.as_deref() != Some(expected.source_head.as_str()) {
                return Err("Retained landing source changed; recovery blocked".into());
            }
            tokio::task::spawn_blocking(move || {
                let _registry_guard = _registry_guard;
                service.validate_effect_token_blocking()?;
                service.validate_merge_phase()?;
                let root = service.source_workspace_root();
                git::ensure_checkout_on_branch(root, "Recovery target", &expected.target_branch)?;
                git::ensure_clean_worktree(root, "Recovery target")?;
                git::ensure_no_merge_in_progress(root, "Recovery target")?;
                if git::head_rev(root)? != expected_post {
                    return Err("Recovery target differs from expected landing post-image".into());
                }
                let repo = git2::Repository::open(root).map_err(|e| e.to_string())?;
                let post = repo.find_commit(git2::Oid::from_str(&expected_post).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
                let source = repo.find_commit(git2::Oid::from_str(&expected.source_head).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
                if post.parent_count() != 1 || post.parent_id(0).map_err(|e| e.to_string())?.to_string() != expected.target_head || post.tree_id() != source.tree_id() {
                    return Err("Recovery material is not the exact native squash landing".into());
                }
                service.validate_merge_phase()?;
                git::run_git_with_refact_author(root, &["revert", "--no-edit", "--no-gpg-sign", &expected_post])?;
                service.validate_merge_phase()?;
                let recovered = git::head_rev(root)?;
                let current = repo.find_commit(git2::Oid::from_str(&recovered).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
                let previous = post.parent(0).map_err(|e| e.to_string())?;
                if current.tree_id() != previous.tree_id() {
                    return Err("Recovery post-image is not the previous known-good tree; inspect, never replay".into());
                }
                git::ensure_clean_worktree(root, "Recovered target")?;
                Ok(recovered)
            }).await.map_err(|e| e.to_string())?
        }.await
    }

    fn merge_worktree_inner_blocking(
        &self,
        id: &str,
        request: MergeWorktreeRequest,
        registry: &mut WorktreeRegistry,
        registry_changed: &mut bool,
        observer: Option<(crate::types::CheckedMergeIdentity, LandingCommitObserver)>,
    ) -> Result<MergeWorktreeResponse, String> {
        crate::locking::with_git_phase(self.merge_authority.clone(), || {
            self.merge_worktree_guarded_inner(id, request, registry, registry_changed, observer)
        })
    }

    fn merge_worktree_guarded_inner(
        &self,
        id: &str,
        request: MergeWorktreeRequest,
        registry: &mut WorktreeRegistry,
        registry_changed: &mut bool,
        observer: Option<(crate::types::CheckedMergeIdentity, LandingCommitObserver)>,
    ) -> Result<MergeWorktreeResponse, String> {
        self.validate_merge_phase()?;
        let index = registry
            .records
            .iter()
            .position(|record| record.meta.id == id)
            .ok_or_else(|| format!("Worktree '{}' not found", id))?;
        let record = registry.records[index].clone();
        let source_branch = record
            .meta
            .branch
            .clone()
            .ok_or_else(|| format!("Worktree '{}' has no source branch", id))?;
        validate_registered_record_checkout(&record)?;
        git::ensure_no_merge_in_progress(&record.meta.root, "Source worktree")?;
        validate_branch_name(&source_branch)?;
        let target_branch = request
            .target_branch
            .clone()
            .or_else(|| record.meta.base_branch.clone())
            .ok_or_else(|| format!("Worktree '{}' has no base branch", id))?;
        validate_branch_name(&target_branch)?;
        if !git::branch_exists(&record.meta.source_workspace_root, &source_branch)? {
            return Err(format!("Source branch '{}' not found", source_branch));
        }
        if !git::branch_exists(&record.meta.source_workspace_root, &target_branch)? {
            return Err(format!("Target branch '{}' not found", target_branch));
        }
        let affected_references = record.references.clone();
        let affected_reference_count = affected_references.len();
        let target_root = self.merge_checkout_root();
        if let Some((source, parent)) = &self.nested_merge_identity {
            if &record.meta != source
                || parent.branch.as_deref() != Some(target_branch.as_str())
                || source.base_branch != parent.branch
                || normalized_path_key(&parent.source_workspace_root)? != self.source_workspace_root
                || normalized_path_key(&source.source_workspace_root)? != self.source_workspace_root
                || normalized_path_key(&parent.root)? != normalized_path_key(target_root)?
                || normalized_path_key(&source.root)? == normalized_path_key(target_root)?
            {
                return Err("Nested merge source or parent checkout identity changed".to_string());
            }
            if normalized_path_key(&parent.root)? != self.source_workspace_root
                && !registry.records.iter().any(|record| &record.meta == parent)
            {
                return Err("Nested merge parent registration changed".to_string());
            }
            validate_worktree_meta_checkout(parent)?;
        }
        if self.merge_checkout.is_some() {
            git::validate_merge_checkout(target_root, &target_branch)?;
        }
        if !request.acknowledge_dirty_baseline {
            if let Some(baseline) = git::dirty_baseline(target_root)? {
                return Ok(MergeWorktreeResponse {
                    id: id.to_string(),
                    status: "dirty_baseline".to_string(),
                    merged: false,
                    strategy: request.strategy.as_str().to_string(),
                    source_branch,
                    target_branch,
                    committed_uncommitted: None,
                    merge_commit: None,
                    cleanup: None,
                    conflict: None,
                    warnings: vec![format!(
                        "Target workspace has {} uncommitted file(s) that were not merged; \
                     commit or stash them, or retry with acknowledge_dirty_baseline=true \
                     to stash and restore them around the merge",
                        baseline.files.len()
                    )],
                    dirty_baseline: Some(baseline),
                    affected_references,
                    affected_reference_count,
                });
            }
        }
        git::ensure_no_merge_in_progress(target_root, "Target workspace")?;
        git::ensure_dirty_worktree_on_branch(target_root, "Target workspace", &target_branch)?;
        let commit_message = request
            .commit_message
            .clone()
            .filter(|message| !message.trim().is_empty())
            .unwrap_or_else(|| fallback_merge_message(&record, &source_branch));
        let mut committed_uncommitted = None;
        if git::has_porcelain_changes(&record.meta.root)? {
            if !request.include_uncommitted {
                return Err(format!(
                "Worktree '{}' has uncommitted changes; set include_uncommitted=true to auto-commit them first",
                id
            ));
            }
            self.validate_merge_phase()?;
            committed_uncommitted = git::commit_all(&record.meta.root, &commit_message)?;
        }
        self.validate_merge_phase()?;
        git::ensure_clean_worktree(&record.meta.root, "Source worktree")?;
        let ahead = git::commits_ahead(
            &record.meta.source_workspace_root,
            &target_branch,
            &source_branch,
        )?;
        self.validate_merge_phase()?;
        if ahead == 0 {
            self.validate_merge_phase()?;
            let cleanup = if request.delete_after_merge && source_branch != target_branch {
                let removal =
                    self.cleanup_registered_worktree_blocking(registry, index, &record, true)?;
                *registry_changed = true;
                Some(removal)
            } else {
                None
            };
            return Ok(MergeWorktreeResponse {
                id: id.to_string(),
                status: "nothing_to_merge".to_string(),
                merged: false,
                strategy: request.strategy.as_str().to_string(),
                source_branch,
                target_branch,
                committed_uncommitted,
                merge_commit: None,
                cleanup,
                conflict: None,
                dirty_baseline: None,
                affected_references,
                affected_reference_count,
                warnings: Vec::new(),
            });
        }
        let preflight = git::preflight_merge_conflicts(
            &record.meta.source_workspace_root,
            &target_branch,
            &source_branch,
            request.strategy.as_str(),
        )?;
        let preflight_warnings = preflight.warnings;
        let conflicts = preflight.conflicts;
        if !conflicts.is_empty() {
            return Ok(MergeWorktreeResponse {
                id: id.to_string(),
                status: "conflict".to_string(),
                merged: false,
                strategy: request.strategy.as_str().to_string(),
                source_branch,
                target_branch,
                committed_uncommitted,
                merge_commit: None,
                cleanup: None,
                conflict: Some(conflict_state(conflicts, true, false)),
                dirty_baseline: None,
                affected_references,
                affected_reference_count,
                warnings: preflight_warnings,
            });
        }
        self.validate_merge_phase()?;
        let target_stash = if let Some((source, parent)) = &self.nested_merge_identity {
            validate_worktree_meta_checkout(source)?;
            validate_worktree_meta_checkout(parent)?;
            git::ensure_clean_worktree(target_root, "Nested merge target")?;
            None
        } else {
            git::stash_dirty_worktree(target_root, "Target workspace", Some(&target_branch))?
        };
        self.validate_merge_phase()?;
        let checkout_result = if self.nested_merge_identity.is_some() {
            Ok(())
        } else {
            git::checkout_branch(target_root, &target_branch)
        };
        if let Err(e) = checkout_result {
            self.validate_merge_phase()?;
            let mut cleanup_warnings = Vec::new();
            if let Some(stash) = target_stash.as_ref() {
                restore_target_stash(target_root, stash, &mut cleanup_warnings);
            }
            return Err(format_merge_failure(
                &format!("Failed to checkout target branch '{}'", target_branch),
                &e,
                cleanup_warnings,
            ));
        }
        self.validate_merge_phase()?;
        let target_pre_merge_head = match git::head_rev(target_root) {
            Ok(head) => head,
            Err(e) => {
                let mut cleanup_warnings = Vec::new();
                if let Some(stash) = target_stash.as_ref() {
                    restore_target_stash(target_root, stash, &mut cleanup_warnings);
                }
                return Err(format_merge_failure(
                    "Failed to read target workspace HEAD after checkout",
                    &e,
                    cleanup_warnings,
                ));
            }
        };
        self.validate_merge_phase()?;
        let merge_result = if request.strategy.as_str() == "squash" {
            git::run_git(target_root, &["merge", "--squash", &source_branch])
        } else {
            git::run_git_with_refact_author(
                target_root,
                &[
                    "merge",
                    "--no-ff",
                    "--no-gpg-sign",
                    &source_branch,
                    "-m",
                    &commit_message,
                ],
            )
        };
        self.validate_merge_phase()?;
        if let Err(e) = merge_result {
            let conflict_files = git::conflict_files_for_path(target_root);
            let merge_abort_warnings = if self.nested_merge_identity.is_some() {
                vec![format!(
                "Nested merge state retained at '{}'; inspect the target index and worktree before retrying",
                target_root.display()
            )]
            } else {
                git::cleanup_failed_merge(target_root)
            };
            let merge_abort_ok = merge_abort_warnings.is_empty();
            let mut cleanup_warnings = merge_abort_warnings;
            let stash_restored = if let Some(stash) = target_stash.as_ref() {
                restore_target_stash(target_root, stash, &mut cleanup_warnings)
            } else {
                true
            };
            if !conflict_files.is_empty() {
                let cleanup_ok = merge_abort_ok && stash_restored;
                let conflict = post_cleanup_conflict_state(target_root, conflict_files, cleanup_ok);
                return Ok(MergeWorktreeResponse {
                    id: id.to_string(),
                    status: "conflict".to_string(),
                    merged: false,
                    strategy: request.strategy.as_str().to_string(),
                    source_branch,
                    target_branch,
                    committed_uncommitted,
                    merge_commit: None,
                    cleanup: None,
                    conflict: Some(conflict),
                    dirty_baseline: None,
                    affected_references,
                    affected_reference_count,
                    warnings: cleanup_warnings,
                });
            }
            return Err(format_merge_failure("Merge failed", &e, cleanup_warnings));
        }
        if request.strategy.as_str() == "squash" {
            self.validate_merge_phase()?;
            let observed = observer.is_some();
            let observed_repository = if let Some((exact, _)) = &observer {
                let repo = git2::Repository::open(target_root).map_err(|e| e.to_string())?;
                let head = repo.head().map_err(|e| e.to_string())?;
                if head.shorthand() != Some(exact.target_branch.as_str())
                    || head
                        .peel_to_commit()
                        .map_err(|e| e.to_string())?
                        .id()
                        .to_string()
                        != exact.target_head
                {
                    return Err("Observed landing pre-commit target changed; retain state".into());
                }
                Some(git::canonical_common_dir(&repo)?)
            } else {
                None
            };
            if let Err(e) = git::run_git_with_refact_author_observed(
                target_root,
                &["commit", "-m", &commit_message, "--no-gpg-sign"],
                || {
                    if let Some((exact, observer)) = observer {
                        let repo =
                            git2::Repository::open(target_root).map_err(|e| e.to_string())?;
                        let head = repo.head().map_err(|e| e.to_string())?;
                        let landed = head.peel_to_commit().map_err(|e| e.to_string())?;
                        let source = repo
                            .find_commit(
                                git2::Oid::from_str(&exact.source_head)
                                    .map_err(|e| e.to_string())?,
                            )
                            .map_err(|e| e.to_string())?;
                        if Some(git::canonical_common_dir(&repo)?) != observed_repository
                            || dunce::canonicalize(target_root)
                                .map_err(|e| e.to_string())?
                                .as_path()
                                != target_root
                            || head.shorthand() != Some(exact.target_branch.as_str())
                            || landed.parent_count() != 1
                            || landed.parent_id(0).map_err(|e| e.to_string())?.to_string()
                                != exact.target_head
                            || landed.tree_id() != source.tree_id()
                            || landed.id().to_string() == exact.target_head
                        {
                            return Err("Successful commit identity unavailable; retain state and inspect, never replay".into());
                        }
                        observer(KnownLandingCommit {
                            commit: landed.id().to_string(),
                            exact,
                        })?;
                    }
                    Ok(())
                },
            ) {
                if observed {
                    return Err(format!("Observed landing stopped; retain source and target, inspect without replay: {e}"));
                }
                self.validate_merge_phase()?;
                if self.nested_merge_identity.is_some() {
                    return Err(format!(
                    "Failed to commit squash merge: {}; integration state retained at '{}' with source worktree '{}' intact; inspect the target index and worktree before retrying",
                    e, target_root.display(), record.meta.root.display()
                ));
                }
                let mut cleanup_warnings = git::cleanup_failed_merge(target_root);
                if let Some(stash) = target_stash.as_ref() {
                    restore_target_stash(target_root, stash, &mut cleanup_warnings);
                }
                return Err(format_merge_failure(
                    "Failed to commit squash merge",
                    &e,
                    cleanup_warnings,
                ));
            }
        }
        self.validate_merge_phase()?;
        let mut warnings = preflight_warnings;
        let merge_commit = match git::head_rev(target_root) {
            Ok(head) => Some(head),
            Err(e) => {
                warnings.push(format!("Failed to read merge commit: {}", e));
                None
            }
        };
        if let Some(stash) = target_stash.as_ref() {
            self.validate_merge_phase()?;
            match git::apply_and_drop_stash(target_root, stash) {
                Ok(stash_warnings) => warnings.extend(stash_warnings),
                Err(e) => {
                    self.validate_merge_phase()?;
                    let conflict_files = git::conflict_files_for_path(target_root);
                    let mut cleanup_warnings = git::reset_hard(target_root, &target_pre_merge_head);
                    let reset_ok = cleanup_warnings.is_empty();
                    let stash_restored =
                        restore_target_stash(target_root, stash, &mut cleanup_warnings);
                    let restored_conflicts = git::conflict_files_for_path(target_root);
                    cleanup_warnings.insert(
                    0,
                    format!(
                        "Failed to reapply target workspace changes after merge; merge was rolled back: {}",
                        e
                    ),
                );
                    let cleanup_ok = reset_ok && stash_restored;
                    let conflict = post_cleanup_conflict_state(
                        target_root,
                        if conflict_files.is_empty() {
                            restored_conflicts
                        } else {
                            conflict_files
                        },
                        cleanup_ok,
                    );
                    return Ok(MergeWorktreeResponse {
                        id: id.to_string(),
                        status: "conflict".to_string(),
                        merged: false,
                        strategy: request.strategy.as_str().to_string(),
                        source_branch,
                        target_branch,
                        committed_uncommitted,
                        merge_commit: None,
                        cleanup: None,
                        conflict: Some(conflict),
                        dirty_baseline: None,
                        affected_references,
                        affected_reference_count,
                        warnings: cleanup_warnings,
                    });
                }
            }
        }
        self.validate_merge_phase()?;
        let cleanup = if request.delete_after_merge && source_branch != target_branch {
            let removal =
                self.cleanup_registered_worktree_blocking(registry, index, &record, true)?;
            *registry_changed = true;
            Some(removal)
        } else {
            registry.records[index].last_known_status =
                Some(git::status_for_path(&record.meta.root));
            registry.records[index].updated_at = Utc::now().to_rfc3339();
            *registry_changed = true;
            None
        };
        Ok(MergeWorktreeResponse {
            id: id.to_string(),
            status: "merged".to_string(),
            merged: true,
            strategy: request.strategy.as_str().to_string(),
            source_branch,
            target_branch,
            committed_uncommitted,
            merge_commit,
            cleanup,
            conflict: None,
            dirty_baseline: None,
            affected_references,
            affected_reference_count,
            warnings,
        })
    }

    pub async fn delete_worktree(
        &self,
        id: &str,
        delete_branch: bool,
        force_referenced: bool,
    ) -> Result<DeleteWorktreeResponse, String> {
        validate_worktree_id(id)?;
        let fenced = self.fence_worktrees(&[id.to_string()], false).await?;
        let _repository_guard = fenced.acquire_merge_permit().await?;
        let _guard = _repository_guard.registry_write_lock().await?;
        _repository_guard.validate_effect_token().await?;
        let mut registry = self.load_registry_unlocked().await?;
        let index = registry
            .records
            .iter()
            .position(|record| record.meta.id == id)
            .ok_or_else(|| format!("Worktree '{}' not found", id))?;
        let record = registry.records[index].clone();
        if !force_referenced && !record.references.is_empty() {
            return Err(format!(
                "Worktree '{}' still has {} active reference(s) (kinds: {}); \
                 pass force_referenced=true to delete anyway.",
                record.meta.id,
                record.references.len(),
                record
                    .references
                    .iter()
                    .map(|reference| reference.kind.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if self.merge_permit.is_some() {
            self.checkpoint_merge_phase().await?;
        }
        let phase_service = _repository_guard.clone();
        let removal_source = record.meta.source_workspace_root.clone();
        let removal_id = record.meta.id.clone();
        let removal_root = record.meta.root.clone();
        let removal_branch = record.meta.branch.clone();
        let removed = {
            let registry_guard = _guard.clone();
            tokio::task::spawn_blocking(move || {
                let _registry_guard = registry_guard;
                phase_service.validate_effect_token_blocking()?;
                if phase_service.merge_permit.is_some() {
                    phase_service.validate_merge_phase()?;
                }
                let stale_path = !removal_root.exists();
                let warnings = git::remove_worktree(&removal_source, &removal_id, &removal_root);
                if phase_service.merge_permit.is_some() {
                    phase_service.validate_merge_phase()?;
                }
                let branch_result = if delete_branch {
                    removal_branch
                        .as_deref()
                        .map(|branch| git::delete_branch(&removal_source, branch))
                } else {
                    None
                };
                let still_present = removal_root.exists();
                Ok::<_, String>((stale_path, warnings, branch_result, still_present))
            })
        }
        .await;
        self.invalidate_registry_cache();
        let (stale_path, mut warnings, branch_result, still_present) = removed
            .map_err(|e| format!("Worktree '{}' removal task failed: {}", record.meta.id, e))??;
        if still_present {
            return Err(format!(
                "Failed to remove worktree directory '{}': {}",
                record.meta.root.display(),
                warnings.join("; ")
            ));
        }

        let mut branch_deleted = false;
        if let Some(result) = branch_result {
            match result {
                Ok(deleted) => branch_deleted = deleted,
                Err(e) => warnings.push(e),
            }
        }
        registry.records.remove(index);
        self.save_registry_unlocked(&registry, &_guard).await?;
        let affected_reference_count = record.references.len();
        Ok(DeleteWorktreeResponse {
            deleted: true,
            branch_deleted,
            stale_path,
            affected_references: record.references,
            affected_reference_count,
            warnings,
        })
    }

    pub async fn open_worktree(&self, id: &str) -> Result<OpenWorktreeResponse, String> {
        let view = self.get_worktree(id).await?;
        Ok(OpenWorktreeResponse {
            id: view.meta.id,
            path: view.meta.root,
            branch: view.meta.branch,
            can_open_folder: view.status.path_exists && view.status.is_git_worktree,
        })
    }

    pub async fn inspect_worktrees(&self) -> Result<WorktreeInventory, String> {
        self.inspect_worktrees_with_min_age(24).await
    }

    pub async fn inspect_worktrees_with_min_age(
        &self,
        min_age_hours: u64,
    ) -> Result<WorktreeInventory, String> {
        let registry = self.load_registry_unlocked().await?;
        let service = self.clone();
        tokio::task::spawn_blocking(move || {
            service.inspect_worktrees_from_registry(&registry, min_age_hours)
        })
        .await
        .map_err(|e| format!("Worktree inspection task failed: {}", e))?
    }

    pub async fn cleanup_worktrees_dry_run(
        &self,
        request: WorktreeCleanupRequest,
    ) -> Result<WorktreeCleanupPlan, String> {
        validate_cleanup_request(&request)?;
        let registry = self.load_registry_unlocked().await?;
        let service = self.clone();
        let min_age_hours = request.min_age_hours;
        let inventory = tokio::task::spawn_blocking(move || {
            service.inspect_worktrees_from_registry(&registry, min_age_hours)
        })
        .await
        .map_err(|e| format!("Worktree cleanup inspection task failed: {}", e))??;
        Ok(self.cleanup_plan_from_inventory(&inventory, request))
    }

    pub async fn cleanup_worktrees(
        &self,
        request: WorktreeCleanupRequest,
    ) -> Result<WorktreeCleanupResult, String> {
        validate_cleanup_request(&request)?;
        let plan = self.cleanup_worktrees_dry_run(request.clone()).await?;
        if plan.candidates.is_empty() {
            return Ok(WorktreeCleanupResult {
                generated_at: Utc::now().to_rfc3339(),
                request,
                deleted: vec![],
                skipped: plan.skipped,
                warnings: vec![],
            });
        }
        let ids: Vec<_> = plan
            .candidates
            .iter()
            .map(|target| target.id.clone())
            .collect();
        let service = self.fence_worktrees(&ids, false).await?;
        async move {
            validate_cleanup_request(&request)?;
            let service = service.acquire_merge_permit().await?;
            let _guard = service.registry_write_lock().await?;
            let registry = service.load_registry_unlocked().await?;
            let blocking_service = service.clone();
            let registry_guard = _guard.clone();
            let cleaned = tokio::task::spawn_blocking(move || {
                let _registry_guard = registry_guard;
                blocking_service.validate_effect_token_blocking()?;
                let mut registry = registry;
                let mut registry_changed = false;
                let result = blocking_service.cleanup_worktrees_inner_blocking(
                    request,
                    &mut registry,
                    &mut registry_changed,
                )?;
                Ok::<_, String>((result, registry_changed.then_some(registry)))
            })
            .await;
            service.invalidate_registry_cache();
            let (result, updated_registry) =
                cleaned.map_err(|e| format!("Worktree cleanup blocking task failed: {}", e))??;
            if let Some(registry) = updated_registry {
                service.save_registry_unlocked(&registry, &_guard).await?;
            }
            Ok(result)
        }
        .await
    }

    fn cleanup_worktrees_inner_blocking(
        &self,
        request: WorktreeCleanupRequest,
        registry: &mut WorktreeRegistry,
        registry_changed: &mut bool,
    ) -> Result<WorktreeCleanupResult, String> {
        let inventory = self.inspect_worktrees_from_registry(&registry, request.min_age_hours)?;
        let plan = self.cleanup_plan_from_inventory(&inventory, request.clone());
        let mut deleted = Vec::new();
        let mut skipped = plan.skipped.clone();
        let warnings = Vec::new();

        for target in plan.candidates {
            // Reinspection may make a previously skipped checkout eligible while
            // effects drain. Eligibility cannot expand the sealed fence domain.
            if !self
                .effect_token
                .as_ref()
                .is_some_and(|token| token.records.iter().any(|meta| meta.id == target.id))
            {
                skipped.push(WorktreeCleanupSkipped {
                    id: target.id,
                    root: Some(target.root),
                    reason: "not_fenced".into(),
                    details: vec!["Checkout became eligible after cleanup fencing; retained".into()],
                });
                continue;
            }
            if let Some(index) = registry
                .records
                .iter()
                .position(|record| record.meta.id == target.id)
            {
                let record = registry.records[index].clone();
                let removal = self.cleanup_registered_worktree_blocking(
                    registry,
                    index,
                    &record,
                    target.delete_branch,
                )?;
                *registry_changed = true;
                deleted.push(WorktreeCleanupDeleted {
                    id: target.id,
                    root: target.root,
                    branch: target.branch,
                    worktree_deleted: removal.worktree_deleted,
                    branch_deleted: removal.branch_deleted,
                    registry_deleted: removal.registry_deleted,
                    stale_path: removal.stale_path,
                    warnings: removal.warnings,
                });
            } else {
                let stale_path = !target.root.exists();
                let mut item_warnings =
                    git::remove_worktree_path(&self.source_workspace_root, &target.root);
                let mut branch_deleted = false;
                if target.delete_branch {
                    if let Some(branch) = target.branch.as_deref() {
                        match git::delete_branch(&self.source_workspace_root, branch) {
                            Ok(deleted) => branch_deleted = deleted,
                            Err(e) => item_warnings.push(e),
                        }
                    }
                }
                deleted.push(WorktreeCleanupDeleted {
                    id: target.id,
                    root: target.root.clone(),
                    branch: target.branch.clone(),
                    worktree_deleted: !target.root.exists(),
                    branch_deleted,
                    registry_deleted: false,
                    stale_path,
                    warnings: item_warnings,
                });
            }
        }

        skipped.sort_by(|a, b| a.id.cmp(&b.id));
        deleted.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(WorktreeCleanupResult {
            generated_at: Utc::now().to_rfc3339(),
            request,
            deleted,
            skipped,
            warnings,
        })
    }

    fn inspect_worktrees_from_registry(
        &self,
        registry: &WorktreeRegistry,
        min_age_hours: u64,
    ) -> Result<WorktreeInventory, String> {
        let mut worktrees = Vec::new();
        let mut registered_roots = HashSet::new();
        let registry_dir = self.registry_dir();

        for record in &registry.records {
            let root_key = normalize_lexical(&record.meta.root)?;
            registered_roots.insert(root_key);
            worktrees.push(self.inspect_registered_record(record, min_age_hours));
        }

        let mut discovered_roots = HashSet::new();
        for entry in git::list_git_worktrees(&self.source_workspace_root) {
            let root_key = normalize_lexical(&entry.root)?;
            if root_key == self.source_workspace_root || registered_roots.contains(&root_key) {
                continue;
            }
            if !root_key.starts_with(&registry_dir) || !discovered_roots.insert(root_key.clone()) {
                continue;
            }
            worktrees.push(self.inspect_discovered_root(
                worktree_id_from_path(&root_key),
                root_key,
                entry.branch,
                entry.head,
                true,
                min_age_hours,
            ));
        }

        if registry_dir.exists() {
            if let Ok(read_dir) = std::fs::read_dir(&registry_dir) {
                for entry in read_dir.flatten() {
                    let path = entry.path();
                    if !path.is_dir() {
                        continue;
                    }
                    let root_key = normalize_lexical(&path)?;
                    if registered_roots.contains(&root_key) || discovered_roots.contains(&root_key)
                    {
                        continue;
                    }
                    if !discovered_roots.insert(root_key.clone()) {
                        continue;
                    }
                    worktrees.push(self.inspect_discovered_root(
                        worktree_id_from_path(&root_key),
                        root_key,
                        None,
                        None,
                        true,
                        min_age_hours,
                    ));
                }
            }
        }

        worktrees.sort_by(|a, b| a.id.cmp(&b.id));
        let summary = summarize_inventory(&worktrees);
        let cleanup_candidates = worktrees
            .iter()
            .filter(|item| item.cleanup_candidate)
            .map(|item| item.id.clone())
            .collect();
        Ok(WorktreeInventory {
            project_hash: self.project_hash.clone(),
            source_workspace_root: self.source_workspace_root.clone(),
            generated_at: Utc::now().to_rfc3339(),
            summary,
            worktrees,
            cleanup_candidates,
        })
    }

    fn inspect_registered_record(
        &self,
        record: &WorktreeRegistryRecord,
        min_age_hours: u64,
    ) -> WorktreeInspection {
        let age_hours = age_hours_since(
            record
                .last_seen_at
                .as_deref()
                .unwrap_or(record.updated_at.as_str()),
        );
        let last_used_at = record
            .last_seen_at
            .clone()
            .or_else(|| Some(record.updated_at.clone()));
        if let Err(error) = validate_registered_record_checkout(record) {
            return self.inspect_invalid_registered_record(
                record,
                age_hours,
                last_used_at,
                min_age_hours,
                error,
            );
        }
        let mut item = self.inspect_root(
            record.meta.id.clone(),
            "registered".to_string(),
            record.meta.root.clone(),
            record.meta.branch.clone(),
            record.meta.base_branch.clone(),
            record.meta.base_commit.clone(),
            record.references.clone(),
            age_hours,
            last_used_at,
            false,
            false,
            min_age_hours,
        );
        item.attached_chat_ids = attached_chat_ids(&item.references);
        item.attached_task_ids = attached_task_ids(&item.references);
        item
    }

    fn inspect_invalid_registered_record(
        &self,
        record: &WorktreeRegistryRecord,
        age_hours: Option<u64>,
        last_used_at: Option<String>,
        min_age_hours: u64,
        error: String,
    ) -> WorktreeInspection {
        let references = record.references.clone();
        let mut item = WorktreeInspection {
            id: record.meta.id.clone(),
            source: "registered".to_string(),
            root: record.meta.root.clone(),
            branch: record.meta.branch.clone(),
            base_branch: record.meta.base_branch.clone(),
            base_commit: record.meta.base_commit.clone(),
            status: WorktreeStatus {
                path_exists: record.meta.root.exists(),
                is_git_worktree: false,
                dirty: false,
                conflicted: false,
                staged_count: 0,
                unstaged_count: 0,
                untracked_count: 0,
                branch: None,
                head_commit: None,
                error: Some(format!("checkout validation failed: {}", error)),
            },
            reference_count: references.len(),
            shared: references.len() > 1,
            references,
            stale: true,
            conflicted: false,
            changed_files: 0,
            committed_files: 0,
            staged_files: 0,
            unstaged_files: 0,
            untracked_files: 0,
            additions: 0,
            deletions: 0,
            cleanup_candidate: false,
            cleanup_blockers: Vec::new(),
            disk_usage_bytes: None,
            age_hours,
            last_used_at,
            branch_merged: None,
            registry_missing: false,
            cache_dir_missing_from_registry: false,
            attached_chat_ids: Vec::new(),
            attached_task_ids: Vec::new(),
        };
        let request = WorktreeCleanupRequest {
            ids: vec![item.id.clone()],
            min_age_hours,
            ..WorktreeCleanupRequest::default()
        };
        item.cleanup_blockers = cleanup_blockers_for_item(&item, &request);
        item.attached_chat_ids = attached_chat_ids(&item.references);
        item.attached_task_ids = attached_task_ids(&item.references);
        item
    }

    fn inspect_discovered_root(
        &self,
        id: String,
        root: PathBuf,
        branch: Option<String>,
        _head: Option<String>,
        cache_dir_missing_from_registry: bool,
        min_age_hours: u64,
    ) -> WorktreeInspection {
        let age_hours = filesystem_age_hours(&root);
        let mut item = self.inspect_root(
            id,
            "discovered".to_string(),
            root,
            branch,
            None,
            None,
            Vec::new(),
            age_hours,
            None,
            true,
            cache_dir_missing_from_registry,
            min_age_hours,
        );
        item.attached_chat_ids = attached_chat_ids(&item.references);
        item.attached_task_ids = attached_task_ids(&item.references);
        item
    }

    fn inspect_root(
        &self,
        id: String,
        source: String,
        root: PathBuf,
        branch: Option<String>,
        base_branch: Option<String>,
        base_commit: Option<String>,
        references: Vec<WorktreeReference>,
        age_hours: Option<u64>,
        last_used_at: Option<String>,
        registry_missing: bool,
        cache_dir_missing_from_registry: bool,
        min_age_hours: u64,
    ) -> WorktreeInspection {
        let mut status = git::status_for_path(&root);
        if let Some(branch) = branch.as_deref() {
            if status.branch.is_none() {
                status.branch = Some(branch.to_string());
            }
        }
        let mut diff_stats = None;
        let mut diff_error = None;
        if status.path_exists && status.is_git_worktree {
            if base_commit.is_none() && base_branch.is_none() {
                diff_error = Some("base metadata is missing".to_string());
            } else {
                match git::diff_for_path(&root, base_commit.as_deref(), base_branch.as_deref(), 1) {
                    Ok(diff) => diff_stats = Some(diff.stats),
                    Err(e) => diff_error = Some(e),
                }
            }
        }
        if let Some(error) = diff_error {
            status.error = Some(format!("diff failed: {}", error));
        }
        let conflicted = status.conflicted
            || (status.path_exists
                && status.is_git_worktree
                && !git::conflict_files_for_path(&root).is_empty());
        status.conflicted = conflicted;
        let shared = references.len() > 1;
        let stale = !status.path_exists || !status.is_git_worktree;
        let changed_files = diff_stats
            .as_ref()
            .map(|stats| stats.files_changed)
            .unwrap_or_else(|| {
                status.staged_count + status.unstaged_count + status.untracked_count
            });
        let committed_files = diff_stats
            .as_ref()
            .map(|stats| stats.committed_files)
            .unwrap_or(0);
        let staged_files = diff_stats
            .as_ref()
            .map(|stats| stats.staged_files)
            .unwrap_or(status.staged_count);
        let unstaged_files = diff_stats
            .as_ref()
            .map(|stats| stats.unstaged_files)
            .unwrap_or(status.unstaged_count);
        let untracked_files = diff_stats
            .as_ref()
            .map(|stats| stats.untracked_files)
            .unwrap_or(status.untracked_count);
        let additions = diff_stats
            .as_ref()
            .map(|stats| stats.additions)
            .unwrap_or(0);
        let deletions = diff_stats
            .as_ref()
            .map(|stats| stats.deletions)
            .unwrap_or(0);
        let branch_merged = branch.as_deref().and_then(|branch| {
            base_branch
                .as_deref()
                .map(|base| git::branch_merged_into(&self.source_workspace_root, branch, base))
        });
        let mut item = WorktreeInspection {
            id,
            source,
            root: root.clone(),
            branch,
            base_branch,
            base_commit,
            status,
            reference_count: references.len(),
            references,
            shared,
            stale,
            conflicted,
            changed_files,
            committed_files,
            staged_files,
            unstaged_files,
            untracked_files,
            additions,
            deletions,
            cleanup_candidate: false,
            cleanup_blockers: Vec::new(),
            disk_usage_bytes: cheap_disk_usage(&root),
            age_hours,
            last_used_at,
            branch_merged,
            registry_missing,
            cache_dir_missing_from_registry,
            attached_chat_ids: Vec::new(),
            attached_task_ids: Vec::new(),
        };
        item.reference_count = item.references.len();
        item.shared = item.reference_count > 1;
        let request = WorktreeCleanupRequest {
            ids: vec![item.id.clone()],
            min_age_hours,
            ..WorktreeCleanupRequest::default()
        };
        item.cleanup_blockers = cleanup_blockers_for_item(&item, &request);
        item.cleanup_candidate = item.cleanup_blockers.is_empty();
        item
    }

    fn cleanup_plan_from_inventory(
        &self,
        inventory: &WorktreeInventory,
        request: WorktreeCleanupRequest,
    ) -> WorktreeCleanupPlan {
        let by_id: HashMap<&str, &WorktreeInspection> = inventory
            .worktrees
            .iter()
            .map(|item| (item.id.as_str(), item))
            .collect();
        let mut candidates = Vec::new();
        let mut skipped = Vec::new();
        let mut seen = HashSet::new();
        for id in &request.ids {
            if !seen.insert(id.clone()) {
                continue;
            }
            let Some(item) = by_id.get(id.as_str()) else {
                skipped.push(WorktreeCleanupSkipped {
                    id: id.clone(),
                    root: None,
                    reason: "not_found".to_string(),
                    details: vec!["worktree id was not found in registry or discovery".to_string()],
                });
                continue;
            };
            let blockers = cleanup_blockers_for_item(item, &request);
            if blockers.is_empty() {
                candidates.push(WorktreeCleanupTarget {
                    id: item.id.clone(),
                    root: item.root.clone(),
                    branch: item.branch.clone(),
                    shared: item.shared,
                    stale: item.stale,
                    changed_files: item.changed_files,
                    additions: item.additions,
                    deletions: item.deletions,
                    delete_branch: request.delete_branches,
                    references: item.references.clone(),
                    disk_usage_bytes: item.disk_usage_bytes,
                });
            } else {
                skipped.push(WorktreeCleanupSkipped {
                    id: item.id.clone(),
                    root: Some(item.root.clone()),
                    reason: blockers
                        .first()
                        .cloned()
                        .unwrap_or_else(|| "blocked".to_string()),
                    details: blockers,
                });
            }
        }
        candidates.sort_by(|a, b| a.id.cmp(&b.id));
        skipped.sort_by(|a, b| a.id.cmp(&b.id));
        WorktreeCleanupPlan {
            generated_at: Utc::now().to_rfc3339(),
            request,
            candidates,
            skipped,
        }
    }

    fn cleanup_registered_worktree_blocking(
        &self,
        registry: &mut WorktreeRegistry,
        index: usize,
        record: &WorktreeRegistryRecord,
        delete_branch: bool,
    ) -> Result<WorktreeRemovalResult, String> {
        if self.merge_permit.is_some() {
            self.validate_merge_phase()?;
        }
        validate_registered_record_checkout(record)?;
        let stale_path = !record.meta.root.exists();
        let mut warnings = git::remove_worktree(
            &record.meta.source_workspace_root,
            &record.meta.id,
            &record.meta.root,
        );
        let worktree_deleted = !record.meta.root.exists();
        let mut branch_deleted = false;
        if self.merge_permit.is_some() {
            self.validate_merge_phase()?;
        }
        if delete_branch {
            if let Some(branch) = record.meta.branch.as_deref() {
                match git::delete_branch(&record.meta.source_workspace_root, branch) {
                    Ok(deleted) => branch_deleted = deleted,
                    Err(e) => warnings.push(e),
                }
            }
        }
        let registry_deleted = worktree_deleted;
        if registry_deleted {
            registry.records.remove(index);
        } else if let Some(record) = registry.records.get_mut(index) {
            record.last_known_status = Some(git::status_for_path(&record.meta.root));
            record.updated_at = Utc::now().to_rfc3339();
        }
        Ok(WorktreeRemovalResult {
            worktree_deleted,
            branch_deleted,
            registry_deleted,
            stale_path,
            warnings,
        })
    }

    async fn record_view(
        &self,
        record: &WorktreeRegistryRecord,
    ) -> Result<WorktreeRecordView, String> {
        let root = record.meta.root.clone();
        let status = tokio::task::spawn_blocking(move || git::status_for_path(&root))
            .await
            .map_err(|e| format!("Worktree status task failed: {}", e))?;
        self.record_view_with_status(record, status).await
    }

    async fn validate_registered_record_checkout_record(
        &self,
        record: &WorktreeRegistryRecord,
    ) -> Result<(), String> {
        let record = record.clone();
        tokio::task::spawn_blocking(move || validate_registered_record_checkout(&record))
            .await
            .map_err(|e| format!("Worktree validation task failed: {}", e))?
    }

    async fn validate_registered_record_diff_record(
        &self,
        record: &WorktreeRegistryRecord,
    ) -> Result<(), String> {
        let record = record.clone();
        tokio::task::spawn_blocking(move || validate_registered_record_diff(&record))
            .await
            .map_err(|e| format!("Worktree validation task failed: {}", e))?
    }

    async fn record_view_with_status(
        &self,
        record: &WorktreeRegistryRecord,
        status: WorktreeStatus,
    ) -> Result<WorktreeRecordView, String> {
        validate_worktree_id(&record.meta.id)?;
        validate_registry_root_async(&self.registry_dir(), &record.meta.id, &record.meta.root)
            .await?;
        let mut seen = HashSet::new();
        let references = record
            .references
            .iter()
            .filter(|reference| reference.has_identity())
            .filter(|reference| seen.insert(reference_key(reference)))
            .cloned()
            .collect::<Vec<_>>();
        let reference_count = references.len();
        let cache_meta = record.meta.clone();
        let cache_report =
            tokio::task::spawn_blocking(move || crate::cache::read_report(&cache_meta))
                .await
                .unwrap_or_else(|_| {
                    Some(crate::cache::degraded(crate::types::CacheReason::IoFailure))
                });
        Ok(WorktreeRecordView {
            meta: record.meta.clone(),
            created_at: record.created_at.clone(),
            updated_at: record.updated_at.clone(),
            last_seen_at: record.last_seen_at.clone(),
            references,
            reference_count,
            status,
            cache_report,
        })
    }

    pub async fn cache_execution_context(
        &self,
        meta: &WorktreeMeta,
    ) -> Result<crate::cache::CacheExecutionContext, crate::types::CacheReason> {
        let service = self.clone();
        let meta = meta.clone();
        tokio::task::spawn_blocking(move || service.cache_execution_context_blocking(&meta))
            .await
            .map_err(|_| crate::types::CacheReason::IoFailure)?
    }

    fn cache_execution_context_blocking(
        &self,
        meta: &WorktreeMeta,
    ) -> Result<crate::cache::CacheExecutionContext, crate::types::CacheReason> {
        if meta.root
            != self
                .worktree_path_for_id(&meta.id)
                .map_err(|_| crate::types::CacheReason::IdentityChanged)?
            || meta.source_workspace_root != self.source_workspace_root
            || meta.root == self.source_workspace_root
            || !meta.enforce
        {
            return Err(crate::types::CacheReason::IdentityChanged);
        }
        validate_worktree_meta_checkout(meta)
            .map_err(|_| crate::types::CacheReason::IdentityChanged)?;
        crate::cache::CacheExecutionContext::from_registered(meta.clone(), self.registry_path())
    }

    async fn load_registry_unlocked(&self) -> Result<WorktreeRegistry, String> {
        let path = self.registry_path();
        if !tokio::fs::try_exists(&path)
            .await
            .map_err(|e| format!("Cannot inspect registry: {e}"))?
        {
            return Ok(WorktreeRegistry {
                schema_version: 1,
                source_workspace_root: self.source_workspace_root.clone(),
                project_hash: self.project_hash.clone(),
                records: Vec::new(),
            });
        }
        let content = tokio::fs::read_to_string(&path).await.map_err(|e| {
            format!(
                "Failed to read worktree registry '{}': {}",
                path.display(),
                e
            )
        })?;
        let parse_path = path.clone();
        let registry: WorktreeRegistry = tokio::task::spawn_blocking(move || {
            serde_json::from_str(&content).map_err(|e| {
                format!(
                    "Failed to parse worktree registry '{}': {}",
                    parse_path.display(),
                    e
                )
            })
        })
        .await
        .map_err(|e| format!("Worktree registry parse task failed: {}", e))??;
        self.validate_registry(&registry).await?;
        Ok(registry)
    }

    async fn save_registry_unlocked(
        &self,
        registry: &WorktreeRegistry,
        guard: &Arc<RegistryGuard>,
    ) -> Result<(), String> {
        self.validate_registry(registry).await?;
        let path = self.registry_path();
        let registry = registry.clone();
        let guard = guard.clone();
        let generation = registry_generation(&self.cache_dir);
        #[cfg(test)]
        let pause = REGISTRY_PUBLICATION_PAUSE.try_with(Arc::clone).ok();
        let published = tokio::task::spawn_blocking(move || {
            let _guard = guard;
            let _invalidation = RegistryPublicationInvalidation(generation);
            #[cfg(test)]
            if let Some(pause) = pause {
                pause.entered.lock().unwrap().take().unwrap().send(()).unwrap();
                pause.release.lock().unwrap().recv_timeout(Duration::from_secs(10)).unwrap();
            }
            use std::io::Write;
            let dir = path.parent().ok_or("Registry directory missing")?;
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            let content = serde_json::to_vec_pretty(&registry).map_err(|e| e.to_string())?;
            let mut temporary = tempfile::NamedTempFile::new_in(dir).map_err(|e| e.to_string())?;
            temporary.write_all(&content).map_err(|e| e.to_string())?;
            temporary.as_file().sync_all().map_err(|e| e.to_string())?;
            if path.exists() {
                let mut backup = tempfile::NamedTempFile::new_in(dir).map_err(|e| e.to_string())?;
                let mut previous = std::fs::File::open(&path).map_err(|e| e.to_string())?;
                std::io::copy(&mut previous, backup.as_file_mut()).map_err(|e| e.to_string())?;
                backup.as_file().sync_all().map_err(|e| e.to_string())?;
                backup.persist(path.with_extension("json.bak")).map_err(|e| e.to_string())?;
            }
            temporary.persist(&path).map_err(|e| format!("Failed to publish registry without removing previous index: {e}"))?;
            #[cfg(unix)]
            std::fs::File::open(dir).and_then(|directory| directory.sync_all())
                .map_err(|e| format!("Registry published but directory durability unknown; inspect before retrying: {e}"))?;
            Ok(())
        }).await;
        published.map_err(|e| format!("Registry publication task failed: {e}"))?
    }

    async fn validate_registry(&self, registry: &WorktreeRegistry) -> Result<(), String> {
        if registry.project_hash != self.project_hash {
            return Err("Worktree registry project hash mismatch".to_string());
        }
        let registry_root = normalized_path_key_async(&registry.source_workspace_root).await?;
        if registry_root != normalized_path_key_async(&self.source_workspace_root).await? {
            return Err("Worktree registry source root mismatch".to_string());
        }
        for record in &registry.records {
            validate_worktree_id(&record.meta.id)?;
            validate_kind(&record.meta.kind)?;
            if let Some(branch) = record.meta.branch.as_deref() {
                validate_branch_name(branch)?;
            }
            if let Some(base_branch) = record.meta.base_branch.as_deref() {
                validate_branch_name(base_branch)?;
            }
            validate_registry_root_async(&self.registry_dir(), &record.meta.id, &record.meta.root)
                .await?;
        }
        Ok(())
    }

    async fn next_worktree_id(&self, registry: &WorktreeRegistry) -> Result<String, String> {
        for _ in 0..16 {
            let id = Uuid::new_v4().to_string();
            if registry.records.iter().all(|record| record.meta.id != id)
                && !tokio::fs::try_exists(self.worktree_path_for_id(&id)?)
                    .await
                    .unwrap_or(false)
            {
                return Ok(id);
            }
        }
        Err("Failed to allocate unique worktree id".to_string())
    }
}

pub fn project_hash_for_path(path: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    let hash = format!("{:x}", hasher.finalize());
    hash.chars().take(16).collect()
}

pub fn validate_worktree_id(id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err("Worktree ID cannot be empty".to_string());
    }
    if id.len() > 128 {
        return Err("Worktree ID is too long".to_string());
    }
    if !id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(
            "Worktree ID must contain only ASCII alphanumeric characters, hyphens, or underscores"
                .to_string(),
        );
    }
    Ok(())
}

pub fn validate_branch_name(branch: &str) -> Result<(), String> {
    if branch.is_empty() {
        return Err("Branch name cannot be empty".to_string());
    }
    if branch.len() > 240 {
        return Err("Branch name is too long".to_string());
    }
    if branch.starts_with('/') || branch.ends_with('/') || branch.starts_with('-') {
        return Err(format!("Invalid branch name '{}'", branch));
    }
    if branch.contains("..")
        || branch.contains("//")
        || branch.contains("@{")
        || branch.ends_with('.')
        || branch.ends_with(".lock")
    {
        return Err(format!("Invalid branch name '{}'", branch));
    }
    if branch.chars().any(|c| {
        c.is_control() || c.is_whitespace() || matches!(c, '~' | '^' | ':' | '?' | '*' | '[' | '\\')
    }) {
        return Err(format!("Invalid branch name '{}'", branch));
    }
    Ok(())
}

fn validate_kind(kind: &str) -> Result<String, String> {
    if kind.is_empty() || kind.len() > 64 {
        return Err("Worktree kind must be 1-64 characters".to_string());
    }
    if !kind
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("Worktree kind contains invalid characters".to_string());
    }
    Ok(kind.to_string())
}

async fn validate_registry_root_async(
    registry_dir: &Path,
    id: &str,
    root: &Path,
) -> Result<(), String> {
    let registry_dir = normalize_lexical(registry_dir)?;
    let root = normalize_lexical(root)?;
    let expected = normalize_lexical(&registry_dir.join(id))?;
    if root != expected {
        return Err(format!(
            "Worktree root '{}' must exactly match registry path '{}'",
            root.display(),
            expected.display()
        ));
    }
    if let Ok(metadata) = tokio::fs::symlink_metadata(&expected).await {
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "Worktree root '{}' must not be a symlink",
                expected.display()
            ));
        }
    }
    Ok(())
}

fn canonicalize_existing_dir(path: &Path) -> Result<PathBuf, String> {
    let canonical = std::fs::canonicalize(path).map_err(|e| {
        format!(
            "Failed to resolve source workspace root '{}': {}",
            path.display(),
            e
        )
    })?;
    if !canonical.is_dir() {
        return Err(format!(
            "Source workspace root '{}' is not a directory",
            canonical.display()
        ));
    }
    Ok(dunce::simplified(&canonical).to_path_buf())
}

async fn canonicalize_existing_dir_async(path: &Path) -> Result<PathBuf, String> {
    let canonical = tokio::fs::canonicalize(path).await.map_err(|e| {
        format!(
            "Failed to resolve source workspace root '{}': {}",
            path.display(),
            e
        )
    })?;
    let metadata = tokio::fs::metadata(&canonical)
        .await
        .map_err(|e| format!("Failed to inspect source workspace root: {}", e))?;
    if !metadata.is_dir() {
        return Err(format!(
            "Source workspace root '{}' is not a directory",
            canonical.display()
        ));
    }
    Ok(dunce::simplified(&canonical).to_path_buf())
}

fn normalize_existing_or_parent(path: &Path) -> PathBuf {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return dunce::simplified(&canonical).to_path_buf();
    }
    let Some(parent) = path.parent() else {
        return normalize_lexical(path).unwrap_or_else(|_| path.to_path_buf());
    };
    let Some(file_name) = path.file_name() else {
        return normalize_lexical(path).unwrap_or_else(|_| path.to_path_buf());
    };
    match std::fs::canonicalize(parent) {
        Ok(canonical_parent) => dunce::simplified(&canonical_parent).join(file_name),
        Err(_) => normalize_lexical(path).unwrap_or_else(|_| path.to_path_buf()),
    }
}

async fn normalize_existing_or_parent_async(path: &Path) -> PathBuf {
    if let Ok(canonical) = tokio::fs::canonicalize(path).await {
        return dunce::simplified(&canonical).to_path_buf();
    }
    let Some(parent) = path.parent() else {
        return normalize_lexical(path).unwrap_or_else(|_| path.to_path_buf());
    };
    let Some(file_name) = path.file_name() else {
        return normalize_lexical(path).unwrap_or_else(|_| path.to_path_buf());
    };
    match tokio::fs::canonicalize(parent).await {
        Ok(canonical_parent) => dunce::simplified(&canonical_parent).join(file_name),
        Err(_) => normalize_lexical(path).unwrap_or_else(|_| path.to_path_buf()),
    }
}

fn normalized_path_key(path: &Path) -> Result<PathBuf, String> {
    std::fs::canonicalize(path)
        .map(|path| dunce::simplified(&path).to_path_buf())
        .or_else(|_| normalize_lexical(path).map(|path| dunce::simplified(&path).to_path_buf()))
}

async fn normalized_path_key_async(path: &Path) -> Result<PathBuf, String> {
    match tokio::fs::canonicalize(path).await {
        Ok(path) => Ok(dunce::simplified(&path).to_path_buf()),
        Err(_) => normalize_lexical(path).map(|path| dunce::simplified(&path).to_path_buf()),
    }
}

fn normalize_lexical(path: &Path) -> Result<PathBuf, String> {
    let mut normalized = if path.is_absolute() {
        PathBuf::new()
    } else {
        std::env::current_dir().map_err(|e| format!("Failed to read current dir: {}", e))?
    };
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::RootDir | Component::Prefix(_) | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    Ok(normalized)
}

fn sanitize_branch_component(value: &str) -> String {
    let component = value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if component.is_empty() {
        "chat".to_string()
    } else {
        component.chars().take(32).collect()
    }
}

fn default_branch_name(kind: &str, chat_id: Option<&str>) -> String {
    let seed = chat_id
        .map(sanitize_branch_component)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let short = seed.chars().take(12).collect::<String>();
    format!("refact/{}/{}", sanitize_branch_component(kind), short)
}

fn request_to_reference(kind: &str, request: &CreateWorktreeRequest) -> Option<WorktreeReference> {
    let reference = WorktreeReference {
        kind: kind.to_string(),
        chat_id: request.chat_id.clone(),
        task_id: request.task_id.clone(),
        card_id: request.card_id.clone(),
        agent_id: request.agent_id.clone(),
    };
    reference.has_identity().then_some(reference)
}

fn reference_key(reference: &WorktreeReference) -> String {
    format!(
        "{}\n{}\n{}\n{}\n{}",
        reference.kind,
        reference.chat_id.as_deref().unwrap_or_default(),
        reference.task_id.as_deref().unwrap_or_default(),
        reference.card_id.as_deref().unwrap_or_default(),
        reference.agent_id.as_deref().unwrap_or_default()
    )
}

fn validate_registered_record_checkout(record: &WorktreeRegistryRecord) -> Result<(), String> {
    validate_worktree_meta_checkout(&record.meta)
}

fn validate_registered_record_diff(record: &WorktreeRegistryRecord) -> Result<(), String> {
    validate_worktree_meta_repository(&record.meta).map(|_| ())
}

fn validate_worktree_meta_checkout(meta: &WorktreeMeta) -> Result<(), String> {
    let source_common_dir = source_common_dir(&meta.source_workspace_root)?;
    let (_, repo) = open_registered_repository(meta, &source_common_dir)?;
    ensure_registered_branch(meta, &repo)
}

fn validate_worktree_meta_repository(meta: &WorktreeMeta) -> Result<PathBuf, String> {
    let source_common_dir = source_common_dir(&meta.source_workspace_root)?;
    open_registered_repository(meta, &source_common_dir).map(|(root, _)| root)
}

fn source_common_dir(source_root: &Path) -> Result<PathBuf, String> {
    git::canonical_common_dir(&git::discover_repo(source_root)?)
}

fn ensure_registered_branch(meta: &WorktreeMeta, repo: &git2::Repository) -> Result<(), String> {
    let source_branch = meta
        .branch
        .as_deref()
        .ok_or_else(|| format!("Worktree '{}' has no source branch", meta.id))?;
    git::ensure_repo_on_branch(repo, "Source worktree", source_branch)
}

fn open_registered_repository(
    meta: &WorktreeMeta,
    source_common_dir: &Path,
) -> Result<(PathBuf, git2::Repository), String> {
    if !meta.root.exists() {
        return Err(format!(
            "Worktree '{}' path '{}' does not exist",
            meta.id,
            meta.root.display()
        ));
    }
    let registered_root = meta
        .root
        .canonicalize()
        .map_err(|e| format!("Failed to canonicalize worktree root: {}", e))?;
    let repo = git::discover_repo(&registered_root)?;
    let top_level = git::repo_root(&repo)?
        .canonicalize()
        .map_err(|e| format!("Failed to canonicalize worktree top level: {}", e))?;
    if top_level != registered_root {
        return Err(format!(
            "Worktree '{}' path '{}' is not the git worktree root",
            meta.id,
            meta.root.display()
        ));
    }
    if git::canonical_common_dir(&repo)? != source_common_dir {
        return Err(format!(
            "Worktree '{}' path '{}' belongs to a different repository",
            meta.id,
            meta.root.display()
        ));
    }
    Ok((registered_root, repo))
}

fn inspect_listed_record(
    meta: &WorktreeMeta,
    source_common_dir: &Path,
) -> Result<WorktreeStatus, String> {
    let (_, repo) = open_registered_repository(meta, source_common_dir)?;
    ensure_registered_branch(meta, &repo)?;
    Ok(git::status_for_repo(&repo))
}

fn fallback_merge_message(record: &WorktreeRegistryRecord, branch: &str) -> String {
    if let Some(card_id) = record.meta.card_id.as_deref() {
        format!("Merge worktree {} for card {}", branch, card_id)
    } else if let Some(task_id) = record.meta.task_id.as_deref() {
        format!("Merge worktree {} for task {}", branch, task_id)
    } else {
        format!("Merge worktree {}", branch)
    }
}

fn restore_target_stash(
    source_root: &Path,
    stash: &git::DirtyWorktreeStash,
    warnings: &mut Vec<String>,
) -> bool {
    let restore_result = git::restore_stashed_changes(source_root, stash);
    let restored = restore_result.critical_ok
        && git::conflict_files_for_path(source_root).is_empty()
        && !git::has_merge_in_progress(source_root);
    warnings.extend(restore_result.warnings);
    restored
}

fn post_cleanup_conflict_state(
    source_root: &Path,
    original_files: Vec<String>,
    cleanup_ok: bool,
) -> WorktreeConflictState {
    let current_files = git::conflict_files_for_path(source_root);
    let merge_in_progress = git::has_merge_in_progress(source_root);
    let current_files_empty = current_files.is_empty();
    let files = if current_files.is_empty() {
        original_files
    } else {
        current_files
    };
    conflict_state(
        files,
        cleanup_ok && !merge_in_progress && current_files_empty,
        merge_in_progress,
    )
}

fn format_merge_failure(prefix: &str, error: &str, cleanup_warnings: Vec<String>) -> String {
    if cleanup_warnings.is_empty() {
        format!("{}: {}", prefix, error)
    } else {
        format!(
            "{}: {}; cleanup warnings: {}",
            prefix,
            error,
            cleanup_warnings.join("; ")
        )
    }
}

fn conflict_state(
    files: Vec<String>,
    aborted: bool,
    merge_in_progress: bool,
) -> WorktreeConflictState {
    let instructions = if aborted {
        "Merge conflicts were detected during preflight or the merge was aborted and cleaned up; no conflict markers were left behind. Merge the source branch changes against the target branch yourself, then retry."
    } else if merge_in_progress {
        "Merge conflicts remain in the target workspace; resolve or abort the merge before retrying."
    } else {
        "The merge was stopped, but cleanup warnings were reported. Inspect the target workspace and warnings before retrying."
    };
    WorktreeConflictState {
        files,
        aborted,
        merge_in_progress,
        instructions: instructions.to_string(),
    }
}

fn validate_cleanup_request(request: &WorktreeCleanupRequest) -> Result<(), String> {
    if request.ids.is_empty() {
        return Err("Cleanup requires explicit worktree ids".to_string());
    }
    for id in &request.ids {
        validate_worktree_id(id)?;
    }
    Ok(())
}

fn cleanup_blockers_for_item(
    item: &WorktreeInspection,
    request: &WorktreeCleanupRequest,
) -> Vec<String> {
    let mut blockers = Vec::new();
    if !item.status.path_exists {
        blockers.push("missing_path".to_string());
    }
    if !item.status.is_git_worktree {
        blockers.push("not_git_worktree".to_string());
    }
    if item.conflicted {
        blockers.push("conflicted".to_string());
    }
    if item.base_commit.is_none() && item.base_branch.is_none() {
        blockers.push("base_unknown".to_string());
    }
    if item.status.error.is_some() {
        blockers.push("diff_error".to_string());
    }
    if request.clean_only && (item.changed_files > 0 || item.status.dirty) {
        blockers.push("dirty".to_string());
    }
    if !request.allow_shared && item.shared {
        blockers.push("shared".to_string());
    }
    if item.reference_count > 0 {
        blockers.push("referenced".to_string());
    }
    match item.age_hours {
        Some(age) if age < request.min_age_hours => blockers.push("too_recent".to_string()),
        None if request.min_age_hours > 0 => blockers.push("age_unknown".to_string()),
        _ => {}
    }
    if request.delete_branches {
        match (&item.branch, item.branch_merged) {
            (Some(_), Some(true)) => {}
            (Some(_), Some(false)) => blockers.push("branch_not_merged".to_string()),
            (Some(_), None) => blockers.push("branch_safety_unknown".to_string()),
            (None, _) => {}
        }
    }
    blockers
}

fn summarize_inventory(worktrees: &[WorktreeInspection]) -> WorktreeInventorySummary {
    let mut summary = WorktreeInventorySummary::default();
    let mut disk_usage = 0u64;
    let mut has_disk_usage = false;
    for item in worktrees {
        if item.source == "registered" {
            summary.total_registered += 1;
        } else {
            summary.total_discovered += 1;
        }
        if item.stale {
            summary.stale += 1;
        }
        if item.conflicted {
            summary.conflicted += 1;
        }
        if item.shared {
            summary.shared += 1;
        }
        let unknown = item.status.error.is_some()
            || (item.base_commit.is_none() && item.base_branch.is_none());
        if unknown {
            summary.unknown += 1;
        }
        if item.changed_files > 0 || item.status.dirty {
            summary.dirty += 1;
        } else if !item.stale && !item.conflicted && !unknown {
            summary.clean += 1;
        }
        if item.cleanup_candidate {
            summary.abandoned_clean += 1;
        }
        if item.source == "registered" && !item.status.path_exists {
            summary.missing_registry_paths += 1;
        }
        if item.cache_dir_missing_from_registry {
            summary.unregistered_cache_dirs += 1;
        }
        if item.branch_merged == Some(true) {
            summary.merged_branches += 1;
        }
        summary.changed_files += item.changed_files;
        summary.additions += item.additions;
        summary.deletions += item.deletions;
        if let Some(bytes) = item.disk_usage_bytes {
            has_disk_usage = true;
            disk_usage = disk_usage.saturating_add(bytes);
        }
        if let Some(age) = item.age_hours {
            summary.newest_age_hours = Some(
                summary
                    .newest_age_hours
                    .map(|current| current.min(age))
                    .unwrap_or(age),
            );
            summary.oldest_age_hours = Some(
                summary
                    .oldest_age_hours
                    .map(|current| current.max(age))
                    .unwrap_or(age),
            );
        }
    }
    summary.total = worktrees.len();
    if has_disk_usage {
        summary.disk_usage_bytes = Some(disk_usage);
    }
    summary
}

fn worktree_id_from_path(path: &Path) -> String {
    if let Some(name) = path.file_name().and_then(|value| value.to_str()) {
        if validate_worktree_id(name).is_ok() {
            return name.to_string();
        }
    }
    format!("discovered_{}", project_hash_for_path(path))
}

fn age_hours_since(timestamp: &str) -> Option<u64> {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .ok()
        .map(|dt| Utc::now().signed_duration_since(dt.with_timezone(&Utc)))
        .map(|duration| duration.num_hours().max(0) as u64)
}

fn filesystem_age_hours(path: &Path) -> Option<u64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    std::time::SystemTime::now()
        .duration_since(modified)
        .ok()
        .map(|duration| duration.as_secs() / 3600)
}

fn cheap_disk_usage(path: &Path) -> Option<u64> {
    if !path.exists() {
        return None;
    }
    let mut total = 0u64;
    let mut seen = 0usize;
    let mut stack = vec![path.to_path_buf()];
    while let Some(path) = stack.pop() {
        seen += 1;
        if seen > 5000 {
            return None;
        }
        let metadata = std::fs::symlink_metadata(&path).ok()?;
        if metadata.is_file() {
            total = total.saturating_add(metadata.len());
        } else if metadata.is_dir() {
            for entry in std::fs::read_dir(&path).ok()? {
                stack.push(entry.ok()?.path());
            }
        }
    }
    Some(total)
}

fn attached_chat_ids(references: &[WorktreeReference]) -> Vec<String> {
    let mut ids = references
        .iter()
        .filter_map(|reference| reference.chat_id.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

fn attached_task_ids(references: &[WorktreeReference]) -> Vec<String> {
    let mut ids = references
        .iter()
        .filter_map(|reference| reference.task_id.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

#[cfg(test)]
mod worktree_registry_tests {
    use std::process::Command;

    use super::*;
    use crate::types::WorktreeMergeStrategy;

    fn run_git(cwd: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap_or_else(|e| panic!("failed to run git {:?}: {}", args, e));
        if !output.status.success() {
            panic!(
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        String::from_utf8_lossy(&output.stdout).to_string()
    }

    #[derive(Debug)]
    struct IsolatedTestFence;
    impl WorktreeEffectFence for IsolatedTestFence {
        fn begin(
            &self,
            _project: PathBuf,
            _roots: Vec<PathBuf>,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<EffectGuard, String>> + Send + '_>,
        > {
            Box::pin(async { Ok(Arc::new(()) as EffectGuard) })
        }
    }
    pub(super) fn test_service(cache: PathBuf, source: PathBuf) -> Result<WorktreeService, String> {
        WorktreeService::new(cache, source)
            .map(|service| service.with_effect_fence(Arc::new(IsolatedTestFence)))
    }

    fn init_repo(root: &Path) {
        run_git(root, &["init"]);
        run_git(root, &["checkout", "-b", "main"]);
        run_git(root, &["config", "core.autocrlf", "false"]);
        run_git(root, &["config", "user.email", "test@example.com"]);
        run_git(root, &["config", "user.name", "Test User"]);
        std::fs::write(root.join("file.txt"), "hello\n").unwrap();
        run_git(root, &["add", "file.txt"]);
        run_git(root, &["commit", "-m", "initial"]);
    }

    #[tokio::test]
    async fn merge_target_seal_refuses_drift_and_new_git_target_after_drain() {
        #[derive(Debug)]
        struct AppearDuringDrain {
            source: PathBuf,
            target: PathBuf,
        }
        impl WorktreeEffectFence for AppearDuringDrain {
            fn begin(
                &self,
                _project: PathBuf,
                roots: Vec<PathBuf>,
            ) -> std::pin::Pin<
                Box<dyn std::future::Future<Output = Result<EffectGuard, String>> + Send + '_>,
            > {
                Box::pin(async move {
                    assert!(!roots.contains(&self.target));
                    run_git(
                        &self.source,
                        &["worktree", "add", self.target.to_str().unwrap(), "landing"],
                    );
                    Ok(Arc::new(()) as EffectGuard)
                })
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir(&source).unwrap();
        init_repo(&source);
        run_git(&source, &["branch", "landing"]);
        let service = test_service(temp.path().join("cache"), source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest::default())
            .await
            .unwrap();
        let target = temp.path().join("unregistered-target");
        let appearing = service.with_effect_fence(Arc::new(AppearDuringDrain {
            source: source.clone(),
            target: target.clone(),
        }));
        let error = appearing
            .fence_merge_worktree(&created.worktree.meta.id, Some("landing"))
            .await
            .unwrap_err();
        assert!(
            error.contains("identity changed after effect drain"),
            "{error}"
        );
        assert!(target.exists());
        let fenced = service
            .fence_merge_worktree(&created.worktree.meta.id, Some("landing"))
            .await
            .unwrap()
            .acquire_merge_permit()
            .await
            .unwrap()
            .for_merge_target("landing")
            .await
            .unwrap();
        run_git(&target, &["checkout", "-b", "drift"]);
        assert!(fenced
            .materialize_merge_target()
            .await
            .unwrap_err()
            .contains("identity changed after effect drain"));
        assert!(created.worktree.meta.root.exists());
    }

    #[tokio::test]
    async fn materialize_revalidates_sealed_identity_but_allows_owned_head_changes() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir(&source).unwrap();
        init_repo(&source);
        let service = test_service(temp.path().join("cache"), source).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/materialize-seal".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        let fenced = service
            .fence_merge_worktree(&created.worktree.meta.id, Some("main"))
            .await
            .unwrap()
            .acquire_merge_permit()
            .await
            .unwrap()
            .for_merge_target("main")
            .await
            .unwrap();
        let original = std::fs::read(service.registry_path()).unwrap();
        let mut registry: WorktreeRegistry = serde_json::from_slice(&original).unwrap();
        registry.records[0].meta.kind = "changed-owner".into();
        std::fs::write(
            service.registry_path(),
            serde_json::to_vec(&registry).unwrap(),
        )
        .unwrap();
        let error = fenced.materialize_merge_target().await.unwrap_err();
        assert!(error.contains("registration changed"), "{error}");
        std::fs::write(service.registry_path(), original).unwrap();
        let before = git::head_rev(&created.worktree.meta.root).unwrap();
        // Like an owned auto-commit, this advances contents, not checkout identity.
        commit_file(
            &created.worktree.meta.root,
            "owned.txt",
            "owned\n",
            "owned commit",
        );
        assert_ne!(git::head_rev(&created.worktree.meta.root).unwrap(), before);
        fenced.materialize_merge_target().await.unwrap();
    }

    #[tokio::test]
    async fn cleanup_retains_skipped_checkout_that_becomes_eligible_during_drain() {
        #[derive(Debug)]
        struct CleanDuringDrain {
            dirty_file: PathBuf,
            expected_roots: Vec<PathBuf>,
        }
        impl WorktreeEffectFence for CleanDuringDrain {
            fn begin(
                &self,
                _project: PathBuf,
                roots: Vec<PathBuf>,
            ) -> std::pin::Pin<
                Box<dyn std::future::Future<Output = Result<EffectGuard, String>> + Send + '_>,
            > {
                Box::pin(async move {
                    assert_eq!(roots, self.expected_roots);
                    // Deterministic barrier: eligibility changes only after the
                    // pre-plan has selected and sealed its drain roots.
                    std::fs::remove_file(&self.dirty_file).unwrap();
                    Ok(Arc::new(()) as EffectGuard)
                })
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir(&source).unwrap();
        init_repo(&source);
        let service = test_service(temp.path().join("cache"), source).unwrap();
        let clean = service
            .create_worktree(CreateWorktreeRequest::default())
            .await
            .unwrap();
        let dirty = service
            .create_worktree(CreateWorktreeRequest::default())
            .await
            .unwrap();
        mark_worktree_old(&service, &clean.worktree.meta.id, 48).await;
        mark_worktree_old(&service, &dirty.worktree.meta.id, 48).await;
        let dirty_file = dirty.worktree.meta.root.join("dirty.txt");
        std::fs::write(&dirty_file, "dirty\n").unwrap();
        let service = service.with_effect_fence(Arc::new(CleanDuringDrain {
            dirty_file,
            expected_roots: vec![clean.worktree.meta.root.clone()],
        }));
        let result = service
            .cleanup_worktrees(WorktreeCleanupRequest {
                ids: vec![
                    clean.worktree.meta.id.clone(),
                    dirty.worktree.meta.id.clone(),
                ],
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(result.deleted.len(), 1);
        assert_eq!(result.deleted[0].id, clean.worktree.meta.id);
        assert!(result
            .skipped
            .iter()
            .any(|item| { item.id == dirty.worktree.meta.id && item.reason == "not_fenced" }));
        assert!(dirty.worktree.meta.root.exists());
        assert!(service.get_worktree(&dirty.worktree.meta.id).await.is_ok());
    }

    #[tokio::test]
    async fn canceled_registry_publication_still_invalidates_retained_revision() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        std::fs::create_dir(&source).unwrap();
        let service = WorktreeService::new(temp.path().join("cache"), source).unwrap();
        let registry = service.load_registry().await.unwrap();
        assert!(!service.registry_path().exists());
        let before = registry_revision(&service.cache_dir);
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let pause = Arc::new(RegistryPublicationPause {
            entered: Mutex::new(Some(entered_tx)),
            release: Mutex::new(release_rx),
        });
        let publisher = service.clone();
        let expected = registry.clone();
        let publication = tokio::spawn(REGISTRY_PUBLICATION_PAUSE.scope(pause, async move {
            publisher
                .compare_and_save_registry(&registry, &registry)
                .await
        }));
        tokio::time::timeout(Duration::from_secs(10), entered_rx)
            .await
            .unwrap()
            .unwrap();
        publication.abort();
        assert!(publication.await.unwrap_err().is_cancelled());
        assert_eq!(before, registry_revision(&service.cache_dir));
        release_tx.send(()).unwrap();
        let _published =
            tokio::time::timeout(Duration::from_secs(10), service.registry_write_lock())
                .await
                .unwrap()
                .unwrap();
        assert!(service.registry_path().is_file());
        assert_eq!(service.load_registry().await.unwrap(), expected);
        assert_ne!(before, registry_revision(&service.cache_dir));
    }

    #[tokio::test]
    async fn checked_landing_rejects_drift_and_preserves_source_and_unrelated_edits() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        init_repo(&root);
        let service = test_service(temp.path().join("cache"), root.clone()).unwrap();
        let authorized_root = root.clone();
        let owner_path = temp.path().join("landing-owner.json");
        let owner = serde_json::json!({"owner":"fixture-owner", "generation":1});
        std::fs::write(&owner_path, serde_json::to_vec(&owner).unwrap()).unwrap();
        let service = service.with_merge_authority(Arc::new(move || {
            let current: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&owner_path).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            if current != owner
                || git::current_branch(&git::discover_repo(&authorized_root)?).as_deref()
                    != Some("main")
            {
                return Err("Fixture persisted owner or target binding changed".into());
            }
            Ok(())
        }));
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/checked".into()),
                ..Default::default()
            })
            .await
            .unwrap();
        let id = &created.worktree.meta.id;
        commit_file(
            &created.worktree.meta.root,
            "file.txt",
            "reviewed\n",
            "candidate",
        );
        let diff = service.diff_worktree(id).await.unwrap();
        let exact = crate::types::CheckedMergeIdentity {
            source_head: diff.status.head_commit.clone().unwrap(),
            target_head: created.worktree.meta.base_commit.clone().unwrap(),
            target_branch: "main".into(),
            patch_digest: format!("{:x}", Sha256::digest(serde_json::to_vec(&diff).unwrap())),
            target_files: vec!["file.txt".into()],
        };
        let request = MergeWorktreeRequest {
            target_branch: Some("main".into()),
            delete_after_merge: false,
            include_uncommitted: false,
            ..Default::default()
        };
        for field in ["source", "target", "patch", "files"] {
            let mut stale = exact.clone();
            match field {
                "source" => stale.source_head = "0".repeat(40),
                "target" => stale.target_head = "0".repeat(40),
                "patch" => stale.patch_digest = "0".repeat(64),
                _ => stale.target_files = vec!["unrelated.txt".into()],
            }
            assert!(service
                .merge_worktree_checked(id, request.clone(), stale)
                .await
                .is_err());
            assert_eq!(
                std::fs::read_to_string(root.join("file.txt")).unwrap(),
                "hello\n"
            );
        }
        std::fs::write(root.join("unrelated.txt"), "retain\n").unwrap();
        assert!(service
            .merge_worktree_checked(id, request.clone(), exact.clone())
            .await
            .is_err());
        assert_eq!(
            std::fs::read_to_string(root.join("unrelated.txt")).unwrap(),
            "retain\n"
        );
        std::fs::remove_file(root.join("unrelated.txt")).unwrap();
        let result = service
            .merge_worktree_checked(id, request.clone(), exact.clone())
            .await
            .unwrap();
        assert!(result.merged);
        assert!(created.worktree.meta.root.exists());
        assert_eq!(
            std::fs::read_to_string(root.join("file.txt")).unwrap(),
            "reviewed\n"
        );
        assert!(service
            .merge_worktree_checked(id, request, exact.clone())
            .await
            .is_err());
        let post = result.merge_commit.unwrap();
        std::fs::write(root.join("unrelated.txt"), "retain recovery\n").unwrap();
        assert!(service
            .recover_checked_landing(id, exact.clone(), post.clone())
            .await
            .is_err());
        assert_eq!(
            std::fs::read_to_string(root.join("unrelated.txt")).unwrap(),
            "retain recovery\n"
        );
        std::fs::remove_file(root.join("unrelated.txt")).unwrap();
        service
            .recover_checked_landing(id, exact.clone(), post.clone())
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("file.txt")).unwrap(),
            "hello\n"
        );
        assert!(service
            .recover_checked_landing(id, exact, post)
            .await
            .is_err());
    }

    fn unset_user_config(root: &Path) {
        let _ = Command::new("git")
            .args(["config", "--unset", "user.email"])
            .current_dir(root)
            .output();
        let _ = Command::new("git")
            .args(["config", "--unset", "user.name"])
            .current_dir(root)
            .output();
    }

    fn commit_file(root: &Path, file: &str, content: &str, message: &str) {
        std::fs::write(root.join(file), content).unwrap();
        run_git(root, &["add", file]);
        run_git(root, &["commit", "-m", message]);
    }

    fn branch_exists(root: &Path, branch: &str) -> bool {
        !run_git(root, &["branch", "--list", branch])
            .trim()
            .is_empty()
    }

    fn head_parent_count(root: &Path) -> usize {
        run_git(root, &["rev-list", "--parents", "-n", "1", "HEAD"])
            .split_whitespace()
            .count()
            .saturating_sub(1)
    }

    fn sample_record(service: &WorktreeService, id: &str) -> WorktreeRegistryRecord {
        let now = Utc::now().to_rfc3339();
        WorktreeRegistryRecord {
            creator_chat_id: None,
            meta: WorktreeMeta {
                id: id.to_string(),
                kind: "chat".to_string(),
                root: service.worktree_path_for_id(id).unwrap(),
                source_workspace_root: service.source_workspace_root().to_path_buf(),
                repo_root: service.source_workspace_root().to_path_buf(),
                branch: Some(format!("refact/chat/{}", id)),
                base_branch: Some("main".to_string()),
                base_commit: Some("abc".to_string()),
                task_id: None,
                card_id: None,
                agent_id: None,
                enforce: true,
            },
            created_at: now.clone(),
            updated_at: now,
            last_seen_at: None,
            references: Vec::new(),
            last_known_status: None,
        }
    }

    struct RegistryChild {
        child: std::process::Child,
        lines: std::sync::mpsc::Receiver<String>,
        reader: Option<std::thread::JoinHandle<()>>,
    }

    impl RegistryChild {
        fn start(source: &Path, cache: &Path, id: &str, mode: &str) -> Self {
            use std::io::BufRead;
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "service::worktree_registry_tests::worktree_registry_cooperating_process",
                    "--nocapture",
                ])
                .env("REFACT_REGISTRY_CHILD", mode)
                .env("REFACT_REGISTRY_SOURCE", source)
                .env("REFACT_REGISTRY_CACHE", cache)
                .env("REFACT_REGISTRY_ID", id)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::inherit())
                .spawn()
                .unwrap();
            let output = child.stdout.take().unwrap();
            let (tx, lines) = std::sync::mpsc::channel();
            let reader = std::thread::spawn(move || {
                for line in std::io::BufReader::new(output).lines() {
                    if tx.send(line.unwrap()).is_err() {
                        break;
                    }
                }
            });
            Self {
                child,
                lines,
                reader: Some(reader),
            }
        }

        fn expect(&self, expected: &str) {
            loop {
                let line = self
                    .lines
                    .recv_timeout(std::time::Duration::from_secs(15))
                    .unwrap();
                if line == expected {
                    return;
                }
            }
        }

        fn release(&mut self) {
            use std::io::Write;
            self.child
                .stdin
                .as_mut()
                .unwrap()
                .write_all(b"release\n")
                .unwrap();
        }

        fn finish(&mut self) {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
            loop {
                if let Some(status) = self.child.try_wait().unwrap() {
                    assert!(status.success());
                    self.reader.take().unwrap().join().unwrap();
                    return;
                }
                assert!(std::time::Instant::now() < deadline, "child did not finish");
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    }

    impl Drop for RegistryChild {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
            if let Some(reader) = self.reader.take() {
                let _ = reader.join();
            }
        }
    }

    #[tokio::test]
    async fn worktree_registry_cooperating_process() {
        use std::io::Write;
        let Some(mode) = std::env::var_os("REFACT_REGISTRY_CHILD") else {
            return;
        };
        let source = PathBuf::from(std::env::var_os("REFACT_REGISTRY_SOURCE").unwrap());
        let cache = PathBuf::from(std::env::var_os("REFACT_REGISTRY_CACHE").unwrap());
        let id = std::env::var("REFACT_REGISTRY_ID").unwrap();
        let service = test_service(cache, source.clone()).unwrap();
        let reference = WorktreeReference {
            kind: "chat".into(),
            chat_id: Some(mode.to_string_lossy().to_string()),
            ..Default::default()
        };
        if mode == "first" {
            let guard = service.registry_write_lock().await.unwrap();
            let mut registry = service.load_registry_unlocked().await.unwrap();
            println!("first-snapshot-loaded");
            std::io::stdout().flush().unwrap();
            let mut release = String::new();
            std::io::stdin().read_line(&mut release).unwrap();
            registry.records[0].references.push(reference);
            service
                .save_registry_unlocked(&registry, &guard)
                .await
                .unwrap();
        } else if mode == "second" {
            service.add_reference(&id, reference).await.unwrap();
        } else {
            let service = service.for_merge_target("main").await.unwrap();
            println!("merge-target-resolved");
            std::io::stdout().flush().unwrap();
            let mut release = String::new();
            std::io::stdin().read_line(&mut release).unwrap();
            let error = service
                .merge_worktree(
                    &id,
                    MergeWorktreeRequest {
                        target_branch: Some("main".into()),
                        delete_after_merge: true,
                        ..Default::default()
                    },
                )
                .await
                .unwrap_err();
            assert!(error.contains("HEAD changed"), "{error}");
        }
    }

    #[tokio::test]
    async fn worktree_registry_two_processes_preserve_exact_references_and_reject_stale_snapshot() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir(&source).unwrap();
        init_repo(&source);
        let cache = temp.path().join("cache");
        let service = test_service(cache.clone(), source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest::default())
            .await
            .unwrap();
        let before = service.load_registry().await.unwrap();
        let id = &created.worktree.meta.id;
        let mut first = RegistryChild::start(&source, &cache, id, "first");
        first.expect("first-snapshot-loaded");
        let mut second = RegistryChild::start(&source, &cache, id, "second");
        second.expect("registry-lock-contended");
        first.release();
        first.finish();
        second.finish();
        let after = service.load_registry().await.unwrap();
        assert_eq!(after.records.len(), 1);
        let mut references = after.records[0]
            .references
            .iter()
            .map(|r| r.chat_id.clone().unwrap())
            .collect::<Vec<_>>();
        references.sort();
        assert_eq!(references, ["first", "second"]);
        let bytes = std::fs::read(service.registry_path()).unwrap();
        assert!(service
            .compare_and_save_registry(&before, &before)
            .await
            .unwrap_err()
            .contains("snapshot changed"));
        assert_eq!(std::fs::read(service.registry_path()).unwrap(), bytes);
        assert!(crate::locking::repository_identity(&source)
            .unwrap()
            .join("refact-worktree-registry.lock")
            .is_file());
    }

    #[tokio::test]
    async fn worktree_merge_process_rechecks_head_after_wait_and_retains_source() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir(&source).unwrap();
        init_repo(&source);
        let cache = temp.path().join("cache");
        let service = test_service(cache.clone(), source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest::default())
            .await
            .unwrap();
        commit_file(
            &created.worktree.meta.root,
            "candidate.txt",
            "candidate\n",
            "candidate",
        );
        let held = service.acquire_merge_permit().await.unwrap();
        let mut child =
            RegistryChild::start(&source, &cache, &created.worktree.meta.id, "stale-merge");
        child.expect("merge-target-resolved");
        child.release();
        child.expect("registry-lock-contended");
        commit_file(&source, "parent.txt", "parent\n", "parent");
        let head = git::head_rev(&source).unwrap();
        let registry = std::fs::read(service.registry_path()).unwrap();
        drop(held);
        child.finish();
        assert_eq!(git::head_rev(&source).unwrap(), head);
        assert_eq!(std::fs::read(service.registry_path()).unwrap(), registry);
        assert_eq!(
            std::fs::read_to_string(source.join("parent.txt")).unwrap(),
            "parent\n"
        );
        assert!(!source.join("candidate.txt").exists());
        assert!(created.worktree.meta.root.exists());
    }

    #[tokio::test]
    async fn worktree_registry_multiple_references_share_metadata_without_duplication() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        let service = test_service(cache, source).unwrap();
        let mut registry = service.load_registry().await.unwrap();
        let expected_registry = registry.clone();
        registry.records.push(sample_record(&service, "wt_1"));
        service
            .compare_and_save_registry(&expected_registry, &registry)
            .await
            .unwrap();
        let first = WorktreeReference {
            kind: "chat".to_string(),
            chat_id: Some("chat-1".to_string()),
            task_id: None,
            card_id: None,
            agent_id: None,
        };
        let second = WorktreeReference {
            kind: "chat".to_string(),
            chat_id: Some("chat-2".to_string()),
            task_id: None,
            card_id: None,
            agent_id: None,
        };
        service.add_reference("wt_1", first.clone()).await.unwrap();
        service.add_reference("wt_1", first).await.unwrap();
        let view = service.add_reference("wt_1", second).await.unwrap();
        assert_eq!(view.meta.id, "wt_1");
        assert_eq!(view.reference_count, 2);
        assert_eq!(service.load_registry().await.unwrap().records.len(), 1);
    }

    #[tokio::test]
    async fn list_worktrees_prunes_missing_record_even_with_stale_references() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                kind: Some("task_agent".to_string()),
                chat_id: Some("chat-1".to_string()),
                task_id: Some("task-1".to_string()),
                card_id: Some("T-1".to_string()),
                agent_id: Some("agent-1".to_string()),
                branch: Some("refact/task/task-1/card/T-1/agent-1".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let id = created.worktree.meta.id.clone();
        let root = created.worktree.meta.root.clone();

        std::fs::remove_dir_all(&root).unwrap();

        let list = service.list_worktrees().await.unwrap();
        assert!(list.worktrees.is_empty());
        assert!(service.get_worktree(&id).await.is_err());

        let registry = service.load_registry().await.unwrap();
        assert!(registry.records.is_empty());
    }

    #[tokio::test]
    async fn list_worktrees_cache_drops_deleted_worktree() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/cached-list".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let id = created.worktree.meta.id.clone();
        let listed = service.list_worktrees_with_branches(false).await.unwrap();
        assert_eq!(
            listed
                .worktrees
                .iter()
                .map(|view| view.meta.id.as_str())
                .collect::<Vec<_>>(),
            vec![id.as_str()]
        );
        assert!(listed.source_branches.is_empty());

        service.delete_worktree(&id, true, false).await.unwrap();

        let listed = service.list_worktrees().await.unwrap();
        assert!(listed.worktrees.is_empty(), "{:?}", listed.worktrees);
        assert!(listed.source_branches.contains(&"main".to_string()));
        assert!(!listed
            .source_branches
            .contains(&"refact/chat/cached-list".to_string()));
    }

    #[tokio::test]
    async fn worktree_registry_create_without_base_uses_current_branch_head() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        run_git(&source, &["checkout", "-b", "dev"]);
        commit_file(&source, "dev_only.txt", "only on dev\n", "dev-only");
        let dev_head = run_git(&source, &["rev-parse", "HEAD"]).trim().to_string();
        let service = test_service(cache, source).unwrap();

        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/current-branch".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();

        assert_eq!(created.worktree.meta.base_branch.as_deref(), Some("dev"));
        assert_eq!(
            created.worktree.meta.base_commit.as_deref(),
            Some(dev_head.as_str())
        );
        assert!(created.worktree.meta.root.join("dev_only.txt").is_file());
        let report = created.worktree.cache_report.clone().unwrap();
        assert_eq!(
            report.compiler_cache.reason,
            crate::types::CacheReason::UnsafeDaemonBoundary
        );
        let loaded = service
            .get_worktree(&created.worktree.meta.id)
            .await
            .unwrap();
        #[cfg(target_os = "linux")]
        assert_eq!(loaded.cache_report, Some(report));
        let listed = service.list_worktrees().await.unwrap();
        assert_eq!(listed.worktrees.len(), 1);
        assert_eq!(listed.worktrees[0].cache_report, loaded.cache_report);
    }

    #[tokio::test]
    async fn worktree_create_uses_explicit_commit_but_keeps_merge_target_branch() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let exact = run_git(&source, &["rev-parse", "HEAD"]).trim().to_string();
        commit_file(&source, "later.txt", "later\n", "later");
        let service = test_service(cache, source).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/exact-base".to_string()),
                base_branch: Some("main".to_string()),
                base_commit: Some(exact.clone()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();

        assert_eq!(created.worktree.meta.base_branch.as_deref(), Some("main"));
        assert_eq!(
            created.worktree.meta.base_commit.as_deref(),
            Some(exact.as_str())
        );
        assert!(!created.worktree.meta.root.join("later.txt").exists());
    }

    #[tokio::test]
    async fn worktree_registry_create_rejects_existing_branch_reuse() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        run_git(&source, &["branch", "refact/chat/reused"]);
        run_git(&source, &["checkout", "-b", "dev"]);
        commit_file(&source, "dev_only.txt", "only on dev\n", "dev-only");
        let service = test_service(cache, source).unwrap();

        let error = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/reused".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap_err();

        assert!(error.contains("already exists"));
    }

    #[tokio::test]
    async fn worktree_registry_create_cleans_branch_after_worktree_failure() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let invalid_worktree_path = source.join("file.txt");

        let error = match git::create_worktree(
            &source,
            &invalid_worktree_path,
            "cleanup-failure",
            "refact/chat/cleanup-failure",
            None,
            None,
        ) {
            Ok(_) => panic!("worktree creation unexpectedly succeeded"),
            Err(error) => error,
        };

        assert!(error.contains("Failed to create worktree"));
        assert!(!branch_exists(&source, "refact/chat/cleanup-failure"));
    }

    #[tokio::test]
    async fn worktree_registry_diff_returns_changed_files_and_patch() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/diff-test".to_string()),
                chat_id: Some("diff-chat".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let root = created.worktree.meta.root.clone();
        std::fs::write(root.join("file.txt"), "hello committed\n").unwrap();
        run_git(&root, &["add", "file.txt"]);
        run_git(&root, &["commit", "-m", "change tracked"]);
        std::fs::write(root.join("untracked.txt"), "new content\n").unwrap();
        let diff = service
            .diff_worktree_with_limit(&created.worktree.meta.id, 50_000)
            .await
            .unwrap();
        assert!(diff.files.iter().any(|file| file.path == "file.txt"));
        assert!(diff.files.iter().any(|file| file.path == "untracked.txt"));
        assert!(diff.patch.contains("hello committed"));
        assert!(diff.patch.contains("new content"));
        assert!(!diff.patch_truncated);
    }

    #[tokio::test]
    async fn worktree_merge_diff_includes_uncommitted_change() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/merge-diff-uncommitted".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        std::fs::write(
            created.worktree.meta.root.join("file.txt"),
            "dirty change\n",
        )
        .unwrap();
        std::fs::write(created.worktree.meta.root.join("new.txt"), "new dirty\n").unwrap();

        let diff = service
            .diff_worktree_with_limit(&created.worktree.meta.id, 50_000)
            .await
            .unwrap();

        assert!(diff.status.dirty);
        assert!(diff
            .files
            .iter()
            .any(|file| file.path == "file.txt" && file.source == "unstaged"));
        assert!(diff
            .files
            .iter()
            .any(|file| file.path == "new.txt" && file.source == "untracked"));
        assert!(diff.patch.contains("dirty change"));
        assert!(diff.patch.contains("new dirty"));
    }

    #[tokio::test]
    async fn worktree_merge_diff_invalid_base_blocks_diff_and_cleanup() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/invalid-base".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        mark_worktree_old(&service, &created.worktree.meta.id, 48).await;
        let mut registry = service.load_registry().await.unwrap();
        let expected_registry = registry.clone();
        registry.records[0].meta.base_commit = Some("not-a-commit".to_string());
        service
            .compare_and_save_registry(&expected_registry, &registry)
            .await
            .unwrap();

        assert!(service
            .diff_worktree_with_limit(&created.worktree.meta.id, 50_000)
            .await
            .is_err());
        let plan = service
            .cleanup_worktrees_dry_run(WorktreeCleanupRequest {
                ids: vec![created.worktree.meta.id.clone()],
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(plan.candidates.is_empty());
        assert!(plan.skipped.iter().any(|item| item.reason == "diff_error"));
        let inventory = service.inspect_worktrees_with_min_age(24).await.unwrap();
        assert_eq!(inventory.summary.clean, 0);
        assert_eq!(inventory.summary.unknown, 1);
    }

    #[tokio::test]
    async fn worktree_merge_diff_large_untracked_file_truncates_safely() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/large-untracked".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        std::fs::write(
            created.worktree.meta.root.join("large.txt"),
            "x".repeat(300_000),
        )
        .unwrap();

        let diff = service
            .diff_worktree_with_limit(&created.worktree.meta.id, 1_000)
            .await
            .unwrap();

        assert!(diff.files.iter().any(|file| file.path == "large.txt"));
        assert!(diff.patch_truncated);
        assert!(diff.patch.len() <= 1_128);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn worktree_merge_diff_untracked_symlink_does_not_read_external_target() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        let outside = temp.path().join("outside-secret.txt");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(&outside, "secret outside repo\n").unwrap();
        init_repo(&source);
        let service = test_service(cache, source).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/symlink-untracked".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        std::os::unix::fs::symlink(&outside, created.worktree.meta.root.join("leak.txt")).unwrap();

        let diff = service
            .diff_worktree(&created.worktree.meta.id)
            .await
            .unwrap();

        assert!(diff.patch.contains("new file mode 120000"));
        assert!(!diff.patch.contains("secret outside repo"));
    }

    #[tokio::test]
    async fn worktree_merge_squash_creates_single_parent_change_on_base() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/squash-merge".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        commit_file(
            &created.worktree.meta.root,
            "file.txt",
            "squash change\n",
            "agent change",
        );
        let check_source = source.clone();
        let preflight_path = Arc::new(std::sync::Mutex::new(None::<PathBuf>));
        let observed_path = preflight_path.clone();
        let service = service.with_merge_authority(Arc::new(move || {
            for entry in git::list_git_worktrees(&check_source) {
                if entry.branch.is_none() && entry.root.ends_with("worktree") {
                    *observed_path.lock().unwrap() = Some(entry.root);
                }
            }
            Ok(())
        }));

        let merged = service
            .merge_worktree(
                &created.worktree.meta.id,
                MergeWorktreeRequest {
                    strategy: WorktreeMergeStrategy::Squash,
                    delete_after_merge: false,
                    target_branch: Some("main".to_string()),
                    commit_message: Some("squash merge".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert!(merged.merged);
        assert!(merged.warnings.is_empty());
        let preflight_path = preflight_path.lock().unwrap().take().unwrap();
        assert!(!preflight_path.exists());
        assert!(!preflight_path.parent().unwrap().exists());
        assert_eq!(
            std::fs::read_to_string(source.join("file.txt")).unwrap(),
            "squash change\n"
        );
        assert_eq!(head_parent_count(&source), 1);
        assert!(created.worktree.meta.root.exists());
        assert!(service
            .get_worktree(&created.worktree.meta.id)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn worktree_merge_auto_commits_dirty_source_worktree_by_default() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/dirty-source-default".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        std::fs::write(
            created.worktree.meta.root.join("file.txt"),
            "dirty source\n",
        )
        .unwrap();

        let merged = service
            .merge_worktree(
                &created.worktree.meta.id,
                MergeWorktreeRequest {
                    strategy: WorktreeMergeStrategy::Squash,
                    delete_after_merge: false,
                    target_branch: Some("main".to_string()),
                    commit_message: Some("dirty source merge".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert!(merged.merged);
        assert!(merged.committed_uncommitted.is_some());
        assert_eq!(
            std::fs::read_to_string(source.join("file.txt")).unwrap(),
            "dirty source\n"
        );
        assert!(!git::status_for_path(&created.worktree.meta.root).dirty);
    }

    #[tokio::test]
    async fn worktree_merge_refuses_dirty_registered_source_on_different_branch() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/source-branch-guard".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        run_git(
            &created.worktree.meta.root,
            &["checkout", "-b", "manual-branch"],
        );
        std::fs::write(
            created.worktree.meta.root.join("file.txt"),
            "dirty wrong branch\n",
        )
        .unwrap();

        let error = service
            .merge_worktree(
                &created.worktree.meta.id,
                MergeWorktreeRequest {
                    strategy: WorktreeMergeStrategy::Squash,
                    delete_after_merge: false,
                    target_branch: Some("main".to_string()),
                    commit_message: Some("should not commit wrong branch".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();

        assert!(error.contains("different branch"), "{error}");
        assert_eq!(
            run_git(&created.worktree.meta.root, &["branch", "--show-current"]).trim(),
            "manual-branch"
        );
        assert_eq!(
            std::fs::read_to_string(created.worktree.meta.root.join("file.txt")).unwrap(),
            "dirty wrong branch\n"
        );
    }

    #[tokio::test]
    async fn worktree_merge_refuses_dirty_registered_source_on_detached_head() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/source-detached-guard".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let head = run_git(&created.worktree.meta.root, &["rev-parse", "HEAD"]);
        run_git(
            &created.worktree.meta.root,
            &["checkout", "--detach", head.trim()],
        );
        std::fs::write(
            created.worktree.meta.root.join("file.txt"),
            "dirty detached\n",
        )
        .unwrap();

        let error = service
            .merge_worktree(
                &created.worktree.meta.id,
                MergeWorktreeRequest {
                    strategy: WorktreeMergeStrategy::Squash,
                    delete_after_merge: false,
                    target_branch: Some("main".to_string()),
                    commit_message: Some("should not commit detached".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();

        assert!(error.contains("detached HEAD"), "{error}");
        assert_eq!(
            std::fs::read_to_string(created.worktree.meta.root.join("file.txt")).unwrap(),
            "dirty detached\n"
        );
    }

    #[tokio::test]
    async fn worktree_merge_refuses_registered_path_replaced_by_unrelated_repo() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/replaced-root".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let source_branch = created.worktree.meta.branch.clone().unwrap();
        std::fs::remove_dir_all(&created.worktree.meta.root).unwrap();
        std::fs::create_dir_all(&created.worktree.meta.root).unwrap();
        init_repo(&created.worktree.meta.root);
        run_git(
            &created.worktree.meta.root,
            &["checkout", "-b", &source_branch],
        );
        std::fs::write(created.worktree.meta.root.join("file.txt"), "unrelated\n").unwrap();

        let error = service
            .merge_worktree(
                &created.worktree.meta.id,
                MergeWorktreeRequest {
                    strategy: WorktreeMergeStrategy::Squash,
                    delete_after_merge: false,
                    target_branch: Some("main".to_string()),
                    commit_message: Some("should reject unrelated repo".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();

        assert!(error.contains("different repository"), "{error}");
    }

    #[tokio::test]
    async fn worktree_reads_refuse_registered_path_replaced_by_unrelated_repo() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/replaced-read-root".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let id = created.worktree.meta.id.clone();
        let root = created.worktree.meta.root.clone();
        let branch = created.worktree.meta.branch.clone().unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        std::fs::create_dir_all(&root).unwrap();
        init_repo(&root);
        run_git(&root, &["checkout", "-b", &branch]);

        for error in [
            service.list_worktrees().await.unwrap_err(),
            service.get_worktree(&id).await.unwrap_err(),
            service.open_worktree(&id).await.unwrap_err(),
            service.diff_worktree(&id).await.unwrap_err(),
        ] {
            assert!(error.contains("different repository"), "{error}");
        }
    }

    #[tokio::test]
    async fn worktree_merge_refuses_dirty_target_before_source_auto_commit() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/target-guard-before-source-commit".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        std::fs::write(
            created.worktree.meta.root.join("source-dirty.txt"),
            "source dirty\n",
        )
        .unwrap();
        run_git(&source, &["checkout", "-b", "target-user-dirty"]);
        std::fs::write(source.join("target-dirty.txt"), "target dirty\n").unwrap();

        let error = service
            .merge_worktree(
                &created.worktree.meta.id,
                MergeWorktreeRequest {
                    strategy: WorktreeMergeStrategy::Squash,
                    delete_after_merge: false,
                    target_branch: Some("main".to_string()),
                    commit_message: Some("should not auto commit source".to_string()),
                    acknowledge_dirty_baseline: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();

        assert!(error.contains("different branch"), "{error}");
        assert_eq!(
            run_git(
                &created.worktree.meta.root,
                &["rev-list", "--count", "HEAD"]
            )
            .trim(),
            "1"
        );
        assert!(
            run_git(&created.worktree.meta.root, &["status", "--porcelain"])
                .contains("source-dirty.txt")
        );
    }

    #[tokio::test]
    async fn worktree_merge_refuses_dirty_target_before_noop_cleanup() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/target-guard-before-noop-cleanup".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let source_branch = created.worktree.meta.branch.clone().unwrap();
        run_git(&source, &["checkout", "-b", "target-user-dirty-noop"]);
        std::fs::write(source.join("target-dirty.txt"), "target dirty\n").unwrap();

        let error = service
            .merge_worktree(
                &created.worktree.meta.id,
                MergeWorktreeRequest {
                    strategy: WorktreeMergeStrategy::Squash,
                    delete_after_merge: true,
                    target_branch: Some("main".to_string()),
                    commit_message: Some("should not cleanup".to_string()),
                    acknowledge_dirty_baseline: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();

        assert!(error.contains("different branch"), "{error}");
        assert!(created.worktree.meta.root.exists());
        assert!(branch_exists(&source, &source_branch));
        assert!(service
            .get_worktree(&created.worktree.meta.id)
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn worktree_merge_auto_commit_failure_restores_source_index() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/dirty-source-hook-failure".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        commit_file(
            &created.worktree.meta.root,
            "agent.txt",
            "agent change\n",
            "agent change",
        );
        std::fs::write(created.worktree.meta.root.join("staged.txt"), "staged\n").unwrap();
        run_git(&created.worktree.meta.root, &["add", "staged.txt"]);
        std::fs::write(
            created.worktree.meta.root.join("unstaged.txt"),
            "unstaged\n",
        )
        .unwrap();
        let git_common_dir = run_git(
            &created.worktree.meta.root,
            &["rev-parse", "--git-common-dir"],
        );
        let git_common_dir = PathBuf::from(git_common_dir.trim());
        let hook = git_common_dir.join("hooks/pre-commit");
        std::fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&hook).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&hook, perms).unwrap();
        }

        let error = service
            .merge_worktree(
                &created.worktree.meta.id,
                MergeWorktreeRequest {
                    strategy: WorktreeMergeStrategy::Squash,
                    delete_after_merge: false,
                    target_branch: Some("main".to_string()),
                    commit_message: Some("source hook failure".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();

        assert!(error.contains("Failed to auto-commit source worktree changes"));
        assert!(run_git(
            &created.worktree.meta.root,
            &["diff", "--cached", "--name-only"]
        )
        .lines()
        .any(|line| line == "staged.txt"));
        assert!(!run_git(
            &created.worktree.meta.root,
            &["diff", "--cached", "--name-only"]
        )
        .lines()
        .any(|line| line == "unstaged.txt"));
        assert!(
            run_git(&created.worktree.meta.root, &["status", "--porcelain"])
                .lines()
                .any(|line| line == "?? unstaged.txt")
        );
    }

    #[tokio::test]
    async fn worktree_merge_refuses_dirty_source_with_merge_in_progress() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/source-merge-in-progress".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        commit_file(
            &created.worktree.meta.root,
            "file.txt",
            "agent side\n",
            "agent side",
        );
        commit_file(&source, "file.txt", "main side\n", "main side");
        let merge_output = Command::new("git")
            .args(["merge", "main"])
            .current_dir(&created.worktree.meta.root)
            .output()
            .unwrap();
        assert!(!merge_output.status.success());
        assert!(git::has_merge_in_progress(&created.worktree.meta.root));

        let error = service
            .merge_worktree(
                &created.worktree.meta.id,
                MergeWorktreeRequest {
                    strategy: WorktreeMergeStrategy::Squash,
                    delete_after_merge: false,
                    target_branch: Some("main".to_string()),
                    commit_message: Some("should not commit source merge".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();

        assert!(error.contains("Source worktree has a merge in progress"));
        assert!(git::has_merge_in_progress(&created.worktree.meta.root));
    }

    #[tokio::test]
    async fn worktree_merge_regular_no_ff_ignores_gpgsign_config() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/regular-gpgsign".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        commit_file(
            &created.worktree.meta.root,
            "file.txt",
            "regular signed config change\n",
            "agent change",
        );
        run_git(&source, &["config", "commit.gpgsign", "true"]);

        let merged = service
            .merge_worktree(
                &created.worktree.meta.id,
                MergeWorktreeRequest {
                    strategy: WorktreeMergeStrategy::Merge,
                    delete_after_merge: false,
                    target_branch: Some("main".to_string()),
                    commit_message: Some("regular merge without gpg signing".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert!(merged.merged);
        assert_eq!(
            std::fs::read_to_string(source.join("file.txt")).unwrap(),
            "regular signed config change\n"
        );
        assert_eq!(head_parent_count(&source), 2);
    }

    #[tokio::test]
    async fn worktree_merge_squash_commit_failure_resets_target_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/hook-failure".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        commit_file(
            &created.worktree.meta.root,
            "file.txt",
            "hook change\n",
            "agent change",
        );
        let hook = source.join(".git/hooks/pre-commit");
        std::fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&hook).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&hook, perms).unwrap();
        }

        let error = service
            .merge_worktree(
                &created.worktree.meta.id,
                MergeWorktreeRequest {
                    strategy: WorktreeMergeStrategy::Squash,
                    delete_after_merge: false,
                    target_branch: Some("main".to_string()),
                    commit_message: Some("hook failure".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();

        assert!(error.contains("Failed to commit squash merge"));
        git::ensure_clean_worktree(&source, "Target workspace").unwrap();
        assert_eq!(
            std::fs::read_to_string(source.join("file.txt")).unwrap(),
            "hello\n"
        );
    }

    #[tokio::test]
    async fn worktree_merge_defaults_to_delete_after_merge_and_reports_references() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/delete-after-merge".to_string()),
                kind: Some("task_agent".to_string()),
                chat_id: Some("chat-1".to_string()),
                task_id: Some("task-1".to_string()),
                card_id: Some("T-1".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        service
            .add_reference(
                &created.worktree.meta.id,
                WorktreeReference {
                    kind: "chat".to_string(),
                    chat_id: Some("chat-2".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        commit_file(
            &created.worktree.meta.root,
            "file.txt",
            "delete after merge\n",
            "agent change",
        );
        let branch = created.worktree.meta.branch.clone().unwrap();
        let root = created.worktree.meta.root.clone();

        let merged = service
            .merge_worktree(
                &created.worktree.meta.id,
                MergeWorktreeRequest {
                    strategy: WorktreeMergeStrategy::Squash,
                    target_branch: Some("main".to_string()),
                    commit_message: Some("delete after merge".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let cleanup = merged.cleanup.unwrap();
        assert!(cleanup.worktree_deleted);
        assert!(cleanup.registry_deleted);
        assert!(cleanup.branch_deleted);
        assert_eq!(merged.affected_reference_count, 2);
        assert!(!root.exists());
        assert!(!branch_exists(&source, &branch));
        assert!(service
            .get_worktree(&created.worktree.meta.id)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn worktree_merge_discard_delete_unmerged_worktree_removes_path_and_branch() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/delete-unmerged".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        commit_file(
            &created.worktree.meta.root,
            "file.txt",
            "unmerged change\n",
            "agent change",
        );
        let branch = created.worktree.meta.branch.clone().unwrap();
        let root = created.worktree.meta.root.clone();

        let deleted = service
            .delete_worktree(&created.worktree.meta.id, true, true)
            .await
            .unwrap();

        assert!(deleted.deleted);
        assert!(deleted.branch_deleted);
        assert!(!root.exists());
        assert!(!branch_exists(&source, &branch));
        assert!(service
            .get_worktree(&created.worktree.meta.id)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn worktree_merge_refuses_an_unacknowledged_dirty_target_and_changes_survive() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/unacknowledged-dirty".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        commit_file(
            &created.worktree.meta.root,
            "agent.txt",
            "agent change\n",
            "agent change",
        );
        std::fs::write(source.join("file.txt"), "user edit\n").unwrap();
        run_git(&source, &["add", "file.txt"]);
        std::fs::write(source.join("target-notes.txt"), "dirty target\n").unwrap();

        let refused = service
            .merge_worktree(
                &created.worktree.meta.id,
                MergeWorktreeRequest {
                    strategy: WorktreeMergeStrategy::Squash,
                    delete_after_merge: true,
                    target_branch: Some("main".to_string()),
                    commit_message: Some("should be refused".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(refused.status, "dirty_baseline");
        assert!(!refused.merged);
        let baseline = refused.dirty_baseline.expect("dirty baseline reported");
        assert_eq!(baseline.branch.as_deref(), Some("main"));
        assert_eq!(baseline.staged_count, 1);
        assert_eq!(baseline.untracked_count, 1);
        assert_eq!(baseline.files.len(), 2);
        assert_eq!(
            std::fs::read_to_string(source.join("file.txt")).unwrap(),
            "user edit\n"
        );
        assert_eq!(
            std::fs::read_to_string(source.join("target-notes.txt")).unwrap(),
            "dirty target\n"
        );
        assert!(!source.join("agent.txt").exists());
        assert!(run_git(&source, &["stash", "list"]).trim().is_empty());
        assert!(created.worktree.meta.root.exists());
    }

    #[tokio::test]
    async fn worktree_merge_preserves_dirty_target_without_repo_identity() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/dirty-target-no-identity".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        commit_file(
            &created.worktree.meta.root,
            "file.txt",
            "agent change\n",
            "agent change",
        );
        std::fs::write(source.join("target-notes.txt"), "dirty target\n").unwrap();
        unset_user_config(&source);

        let merged = service
            .merge_worktree(
                &created.worktree.meta.id,
                MergeWorktreeRequest {
                    strategy: WorktreeMergeStrategy::Squash,
                    delete_after_merge: false,
                    target_branch: Some("main".to_string()),
                    commit_message: Some("dirty target merge without identity".to_string()),
                    acknowledge_dirty_baseline: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert!(merged.merged);
        assert_eq!(
            std::fs::read_to_string(source.join("file.txt")).unwrap(),
            "agent change\n"
        );
        assert_eq!(
            std::fs::read_to_string(source.join("target-notes.txt")).unwrap(),
            "dirty target\n"
        );
        assert!(git::status_for_path(&source).dirty);
        assert!(run_git(&source, &["stash", "list"]).trim().is_empty());
    }

    #[tokio::test]
    async fn worktree_merge_preserves_dirty_target_staged_and_unstaged_state() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        commit_file(&source, "target-base.txt", "target base\n", "target base");
        let service = test_service(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/dirty-target-index".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        commit_file(
            &created.worktree.meta.root,
            "file.txt",
            "agent change\n",
            "agent change",
        );
        std::fs::write(source.join("staged-target.txt"), "staged target\n").unwrap();
        run_git(&source, &["add", "staged-target.txt"]);
        std::fs::write(source.join("target-base.txt"), "unstaged target\n").unwrap();

        let merged = service
            .merge_worktree(
                &created.worktree.meta.id,
                MergeWorktreeRequest {
                    strategy: WorktreeMergeStrategy::Squash,
                    delete_after_merge: false,
                    target_branch: Some("main".to_string()),
                    commit_message: Some("dirty target index merge".to_string()),
                    acknowledge_dirty_baseline: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert!(merged.merged);
        assert_eq!(
            std::fs::read_to_string(source.join("file.txt")).unwrap(),
            "agent change\n"
        );
        assert_eq!(
            std::fs::read_to_string(source.join("staged-target.txt")).unwrap(),
            "staged target\n"
        );
        assert_eq!(
            std::fs::read_to_string(source.join("target-base.txt")).unwrap(),
            "unstaged target\n"
        );
        assert!(run_git(&source, &["diff", "--cached", "--name-only"])
            .lines()
            .any(|line| line == "staged-target.txt"));
        assert!(run_git(&source, &["diff", "--name-only"])
            .lines()
            .any(|line| line == "target-base.txt"));
        assert!(run_git(&source, &["stash", "list"]).trim().is_empty());
    }

    #[tokio::test]
    async fn worktree_merge_blocks_dirty_target_changes_from_different_branch() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/dirty-target-cross-branch".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        commit_file(
            &created.worktree.meta.root,
            "file.txt",
            "agent change\n",
            "agent change",
        );
        run_git(&source, &["checkout", "-b", "user-dirty"]);
        std::fs::write(source.join("target-notes.txt"), "dirty target\n").unwrap();

        let err = service
            .merge_worktree(
                &created.worktree.meta.id,
                MergeWorktreeRequest {
                    strategy: WorktreeMergeStrategy::Squash,
                    delete_after_merge: false,
                    target_branch: Some("main".to_string()),
                    commit_message: Some("dirty target cross branch".to_string()),
                    acknowledge_dirty_baseline: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();

        assert!(err.contains("different branch"));
        assert!(err.contains("user-dirty"));
        assert_eq!(
            run_git(&source, &["branch", "--show-current"]).trim(),
            "user-dirty"
        );
        assert_eq!(
            std::fs::read_to_string(source.join("target-notes.txt")).unwrap(),
            "dirty target\n"
        );
        assert!(git::status_for_path(&source).dirty);
        assert!(run_git(&source, &["stash", "list"]).trim().is_empty());
    }

    #[tokio::test]
    async fn worktree_merge_blocks_dirty_target_changes_from_detached_head() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/dirty-target-detached".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        commit_file(
            &created.worktree.meta.root,
            "file.txt",
            "agent change\n",
            "agent change",
        );
        run_git(&source, &["checkout", "--detach", "HEAD"]);
        let detached_head = run_git(&source, &["rev-parse", "HEAD"]).trim().to_string();
        std::fs::write(source.join("target-notes.txt"), "dirty detached target\n").unwrap();

        let err = service
            .merge_worktree(
                &created.worktree.meta.id,
                MergeWorktreeRequest {
                    strategy: WorktreeMergeStrategy::Squash,
                    delete_after_merge: false,
                    target_branch: Some("main".to_string()),
                    commit_message: Some("dirty target detached".to_string()),
                    acknowledge_dirty_baseline: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();

        assert!(err.contains("detached HEAD"));
        assert_eq!(run_git(&source, &["branch", "--show-current"]).trim(), "");
        assert_eq!(
            run_git(&source, &["rev-parse", "HEAD"]).trim(),
            detached_head
        );
        assert_eq!(
            std::fs::read_to_string(source.join("target-notes.txt")).unwrap(),
            "dirty detached target\n"
        );
        assert!(git::status_for_path(&source).dirty);
        assert!(run_git(&source, &["stash", "list"]).trim().is_empty());
    }

    #[tokio::test]
    async fn worktree_merge_conflicting_dirty_target_change_rolls_back_and_restores() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/dirty-target-conflict".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        commit_file(
            &created.worktree.meta.root,
            "file.txt",
            "agent change\n",
            "agent change",
        );
        std::fs::write(source.join("file.txt"), "dirty target\n").unwrap();

        let merged = service
            .merge_worktree(
                &created.worktree.meta.id,
                MergeWorktreeRequest {
                    strategy: WorktreeMergeStrategy::Squash,
                    delete_after_merge: false,
                    target_branch: Some("main".to_string()),
                    commit_message: Some("dirty target conflict".to_string()),
                    acknowledge_dirty_baseline: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(merged.status, "conflict");
        assert!(!merged.merged);
        let conflict = merged.conflict.unwrap();
        assert!(conflict.aborted);
        assert!(!conflict.merge_in_progress);
        assert!(conflict.files.contains(&"file.txt".to_string()));
        assert_eq!(
            std::fs::read_to_string(source.join("file.txt")).unwrap(),
            "dirty target\n"
        );
        assert!(git::status_for_path(&source).dirty);
        assert!(run_git(&source, &["stash", "list"]).trim().is_empty());
    }

    #[tokio::test]
    async fn worktree_merge_untracked_dirty_target_conflict_rolls_back_and_restores() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/dirty-target-untracked-conflict".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        commit_file(
            &created.worktree.meta.root,
            "new-file.txt",
            "agent file\n",
            "agent adds file",
        );
        std::fs::write(source.join("new-file.txt"), "dirty target file\n").unwrap();

        let merged = service
            .merge_worktree(
                &created.worktree.meta.id,
                MergeWorktreeRequest {
                    strategy: WorktreeMergeStrategy::Squash,
                    delete_after_merge: false,
                    target_branch: Some("main".to_string()),
                    commit_message: Some("dirty target untracked conflict".to_string()),
                    acknowledge_dirty_baseline: true,
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(merged.status, "conflict");
        assert!(!merged.merged);
        let conflict = merged.conflict.unwrap();
        assert!(conflict.aborted);
        assert!(!conflict.merge_in_progress);
        assert_eq!(
            std::fs::read_to_string(source.join("new-file.txt")).unwrap(),
            "dirty target file\n"
        );
        assert!(merged
            .warnings
            .iter()
            .any(|warning| warning.contains("merge was rolled back")));
        assert!(git::status_for_path(&source).dirty);
        assert!(run_git(&source, &["stash", "list"]).trim().is_empty());
    }

    #[tokio::test]
    async fn worktree_merge_conflict_returns_files_and_keeps_worktree() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/conflict-merge".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        commit_file(&source, "file.txt", "target change\n", "target change");
        commit_file(
            &created.worktree.meta.root,
            "file.txt",
            "source change\n",
            "source change",
        );
        let target_head = git::head_rev(&source).unwrap();
        let source_head = git::head_rev(&created.worktree.meta.root).unwrap();
        let worktrees_before = run_git(&source, &["worktree", "list", "--porcelain"]);
        let owner_path = temp.path().join("preflight-owner.json");
        let owner = serde_json::json!({"owner":"preflight-fixture", "generation":1});
        let revoked_owner = serde_json::json!({"owner":"replacement", "generation":2});
        std::fs::write(&owner_path, serde_json::to_vec(&owner).unwrap()).unwrap();
        let preflight_path = Arc::new(std::sync::Mutex::new(None::<PathBuf>));
        let revoke = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let observed_path = preflight_path.clone();
        let revoke_at_conflict = revoke.clone();
        let check_source = source.clone();
        let check_owner_path = owner_path.clone();
        let replacement = revoked_owner.clone();
        let service = service.with_merge_authority(Arc::new(move || {
            let current: serde_json::Value = serde_json::from_slice(
                &std::fs::read(&check_owner_path).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            if current != owner {
                return Err("Fixture persisted preflight owner changed".into());
            }
            for entry in git::list_git_worktrees(&check_source) {
                if entry.branch.is_some() || !entry.root.ends_with("worktree") {
                    continue;
                }
                assert!(entry.root.join(".git").is_file());
                {
                    let mut observed = observed_path.lock().unwrap();
                    if observed.is_none() {
                        assert_eq!(
                            std::fs::read_to_string(entry.root.join("file.txt")).unwrap(),
                            "target change\n"
                        );
                        std::fs::write(
                            entry.root.join("preflight-evidence.txt"),
                            "retain evidence\n",
                        )
                        .unwrap();
                        *observed = Some(entry.root.clone());
                    }
                }
                if revoke_at_conflict.load(std::sync::atomic::Ordering::SeqCst)
                    && git::discover_repo(&entry.root)?
                        .index()
                        .map_err(|e| e.to_string())?
                        .has_conflicts()
                {
                    assert_eq!(
                        std::fs::read_to_string(entry.root.join("preflight-evidence.txt")).unwrap(),
                        "retain evidence\n"
                    );
                    std::fs::write(&check_owner_path, serde_json::to_vec(&replacement).unwrap())
                        .map_err(|e| e.to_string())?;
                    return Err("Fixture persisted preflight owner changed".into());
                }
            }
            Ok(())
        }));

        let merged = service
            .merge_worktree(
                &created.worktree.meta.id,
                MergeWorktreeRequest {
                    strategy: WorktreeMergeStrategy::Merge,
                    delete_after_merge: false,
                    target_branch: Some("main".to_string()),
                    commit_message: Some("conflict merge".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(merged.status, "conflict");
        assert!(!merged.merged);
        assert!(merged.warnings.is_empty());
        let cleaned_path = preflight_path.lock().unwrap().take().unwrap();
        assert!(!cleaned_path.exists());
        assert!(!cleaned_path.parent().unwrap().exists());
        assert_eq!(
            run_git(&source, &["worktree", "list", "--porcelain"]),
            worktrees_before
        );
        assert!(merged
            .conflict
            .unwrap()
            .files
            .contains(&"file.txt".to_string()));
        assert!(created.worktree.meta.root.exists());
        assert_eq!(
            std::fs::read_to_string(source.join("file.txt")).unwrap(),
            "target change\n"
        );
        assert!(service
            .get_worktree(&created.worktree.meta.id)
            .await
            .is_ok());

        revoke.store(true, std::sync::atomic::Ordering::SeqCst);
        let error = service
            .merge_worktree(
                &created.worktree.meta.id,
                MergeWorktreeRequest {
                    strategy: WorktreeMergeStrategy::Merge,
                    delete_after_merge: false,
                    target_branch: Some("main".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();

        let retained_path = preflight_path.lock().unwrap().take().unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(&owner_path).unwrap())
                .unwrap(),
            revoked_owner
        );
        assert_eq!(
            std::fs::read_to_string(retained_path.join("preflight-evidence.txt")).unwrap(),
            "retain evidence\n"
        );
        assert!(retained_path.join(".git").is_file());
        assert_eq!(
            git::conflict_files_for_path(&retained_path),
            vec!["file.txt".to_string()]
        );
        assert!(
            error.contains(&format!("worktree '{}'", retained_path.display())),
            "{error}"
        );
        assert!(error.contains("Git phase authority lost"), "{error}");
        assert!(error.contains("do not replay"), "{error}");
        assert_eq!(git::head_rev(&source).unwrap(), target_head);
        assert_eq!(
            git::head_rev(&created.worktree.meta.root).unwrap(),
            source_head
        );
        assert_eq!(
            std::fs::read_to_string(source.join("file.txt")).unwrap(),
            "target change\n"
        );
        assert_eq!(
            std::fs::read_to_string(created.worktree.meta.root.join("file.txt")).unwrap(),
            "source change\n"
        );
        assert!(run_git(&source, &["status", "--porcelain"]).is_empty());
        assert!(run_git(&created.worktree.meta.root, &["status", "--porcelain"]).is_empty());
        assert!(git::list_git_worktrees(&source)
            .iter()
            .any(|entry| entry.root == retained_path));
        run_git(
            &source,
            &[
                "worktree",
                "remove",
                "--force",
                retained_path.to_str().unwrap(),
            ],
        );
        std::fs::remove_dir(retained_path.parent().unwrap()).unwrap();
    }

    async fn mark_worktree_old(service: &WorktreeService, id: &str, hours: i64) {
        let mut registry = service.load_registry().await.unwrap();
        let expected_registry = registry.clone();
        let record = registry
            .records
            .iter_mut()
            .find(|record| record.meta.id == id)
            .unwrap();
        let ts = (Utc::now() - chrono::Duration::hours(hours)).to_rfc3339();
        record.created_at = ts.clone();
        record.updated_at = ts.clone();
        record.last_seen_at = Some(ts);
        service
            .compare_and_save_registry(&expected_registry, &registry)
            .await
            .unwrap();
    }

    fn replace_registered_root_with_unrelated_repo(source: &Path, root: &Path, branch: &str) {
        std::fs::remove_dir_all(root).unwrap();
        std::fs::create_dir_all(root).unwrap();
        run_git(root, &["init"]);
        run_git(root, &["config", "core.autocrlf", "false"]);
        run_git(root, &["config", "user.email", "test@example.com"]);
        run_git(root, &["config", "user.name", "Test User"]);
        run_git(root, &["fetch", source.to_str().unwrap(), "main"]);
        run_git(root, &["checkout", "-b", branch, "FETCH_HEAD"]);
        std::fs::write(root.join("replacement-only.txt"), "replacement contents\n").unwrap();
        run_git(root, &["add", "replacement-only.txt"]);
        run_git(root, &["commit", "-m", "replacement contents"]);
    }

    #[tokio::test]
    async fn worktree_summary_marks_replaced_registered_root_invalid_without_analyzing_it() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/replaced-summary-root".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let id = created.worktree.meta.id.clone();
        let root = created.worktree.meta.root.clone();
        let branch = created.worktree.meta.branch.clone().unwrap();
        replace_registered_root_with_unrelated_repo(&source, &root, &branch);

        let inventory = service.inspect_worktrees_with_min_age(24).await.unwrap();

        let item = inventory
            .worktrees
            .iter()
            .find(|item| item.id == id)
            .unwrap();
        assert!(item.stale);
        assert!(!item.status.is_git_worktree);
        assert_eq!(item.changed_files, 0);
        assert_eq!(item.additions, 0);
        assert_eq!(item.deletions, 0);
        assert!(item.disk_usage_bytes.is_none());
        assert!(item.branch_merged.is_none());
        assert!(item
            .status
            .error
            .as_deref()
            .is_some_and(|error| error.contains("different repository")));
        assert!(!item.cleanup_candidate);
        assert!(!inventory.cleanup_candidates.contains(&id));
        assert_eq!(inventory.summary.dirty, 0);
        assert_eq!(inventory.summary.changed_files, 0);
        assert_eq!(inventory.summary.additions, 0);
        assert_eq!(inventory.summary.deletions, 0);
        assert!(root.join("replacement-only.txt").is_file());
    }

    #[tokio::test]
    async fn worktree_cleanup_dry_run_skips_replaced_registered_root() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/replaced-cleanup-dry-run".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let id = created.worktree.meta.id.clone();
        let root = created.worktree.meta.root.clone();
        let branch = created.worktree.meta.branch.clone().unwrap();
        mark_worktree_old(&service, &id, 48).await;
        replace_registered_root_with_unrelated_repo(&source, &root, &branch);

        let plan = service
            .cleanup_worktrees_dry_run(WorktreeCleanupRequest {
                ids: vec![id.clone()],
                clean_only: false,
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(plan.candidates.is_empty());
        assert!(plan.skipped.iter().any(|item| {
            item.id == id
                && item.reason == "not_git_worktree"
                && item.details.contains(&"diff_error".to_string())
        }));
        assert!(root.join("replacement-only.txt").is_file());
    }

    #[tokio::test]
    async fn worktree_cleanup_execution_skips_replaced_registered_root() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source.clone()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/replaced-cleanup-execute".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let id = created.worktree.meta.id.clone();
        let root = created.worktree.meta.root.clone();
        let branch = created.worktree.meta.branch.clone().unwrap();
        mark_worktree_old(&service, &id, 48).await;
        replace_registered_root_with_unrelated_repo(&source, &root, &branch);

        let result = service
            .cleanup_worktrees(WorktreeCleanupRequest {
                ids: vec![id.clone()],
                clean_only: false,
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(result.deleted.is_empty());
        assert!(result.skipped.iter().any(|item| {
            item.id == id
                && item.reason == "not_git_worktree"
                && item.details.contains(&"diff_error".to_string())
        }));
        assert!(root.join("replacement-only.txt").is_file());
        assert!(service
            .load_registry()
            .await
            .unwrap()
            .records
            .iter()
            .any(|record| record.meta.id == id));
    }

    #[tokio::test]
    async fn worktree_hygiene_cleanup_dry_run_does_not_delete_and_blocks_shared_dirty() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source).unwrap();
        let clean = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/hygiene-dry-clean".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        mark_worktree_old(&service, &clean.worktree.meta.id, 48).await;
        let shared = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/hygiene-dry-shared".to_string()),
                chat_id: Some("chat-shared".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        mark_worktree_old(&service, &shared.worktree.meta.id, 48).await;
        service
            .add_reference(
                &shared.worktree.meta.id,
                WorktreeReference {
                    kind: "chat".to_string(),
                    chat_id: Some("chat-shared-2".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let dirty = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/hygiene-dry-dirty".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        mark_worktree_old(&service, &dirty.worktree.meta.id, 48).await;
        std::fs::write(dirty.worktree.meta.root.join("dirty.txt"), "dirty\n").unwrap();

        let plan = service
            .cleanup_worktrees_dry_run(WorktreeCleanupRequest {
                ids: vec![
                    clean.worktree.meta.id.clone(),
                    shared.worktree.meta.id.clone(),
                    dirty.worktree.meta.id.clone(),
                ],
                ..Default::default()
            })
            .await
            .unwrap();

        assert_eq!(plan.candidates.len(), 1);
        assert_eq!(plan.candidates[0].id, clean.worktree.meta.id);
        assert!(plan.skipped.iter().any(|item| item.reason == "shared"));
        assert!(plan.skipped.iter().any(|item| item.reason == "dirty"));
        assert!(clean.worktree.meta.root.exists());
        assert!(service.get_worktree(&clean.worktree.meta.id).await.is_ok());
    }

    #[tokio::test]
    async fn worktree_hygiene_cleanup_actual_deletes_only_selected_safe_worktrees() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source.clone()).unwrap();
        let clean = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/hygiene-delete-clean".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        mark_worktree_old(&service, &clean.worktree.meta.id, 48).await;
        let dirty = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/hygiene-delete-dirty".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        mark_worktree_old(&service, &dirty.worktree.meta.id, 48).await;
        std::fs::write(dirty.worktree.meta.root.join("dirty.txt"), "dirty\n").unwrap();
        let clean_root = clean.worktree.meta.root.clone();
        let dirty_root = dirty.worktree.meta.root.clone();
        let branch = clean.worktree.meta.branch.clone().unwrap();

        let result = service
            .cleanup_worktrees(WorktreeCleanupRequest {
                ids: vec![
                    clean.worktree.meta.id.clone(),
                    dirty.worktree.meta.id.clone(),
                ],
                delete_branches: true,
                ..Default::default()
            })
            .await
            .unwrap();

        assert_eq!(result.deleted.len(), 1);
        assert_eq!(result.deleted[0].id, clean.worktree.meta.id);
        assert!(result.deleted[0].worktree_deleted);
        assert!(result.deleted[0].branch_deleted);
        assert!(result.skipped.iter().any(|item| item.reason == "dirty"));
        assert!(!clean_root.exists());
        assert!(dirty_root.exists());
        assert!(!branch_exists(&source, &branch));
        assert!(service.get_worktree(&clean.worktree.meta.id).await.is_err());
    }

    #[tokio::test]
    async fn worktree_hygiene_cleanup_blocks_single_referenced_worktree() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source).unwrap();
        let referenced = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/referenced-clean".to_string()),
                chat_id: Some("chat-referenced".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        mark_worktree_old(&service, &referenced.worktree.meta.id, 48).await;

        let plan = service
            .cleanup_worktrees_dry_run(WorktreeCleanupRequest {
                ids: vec![referenced.worktree.meta.id.clone()],
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(plan.candidates.is_empty());
        assert!(plan.skipped.iter().any(|item| item.reason == "referenced"));
    }

    #[tokio::test]
    async fn worktree_hygiene_allow_shared_still_blocks_single_reference() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source).unwrap();
        let referenced = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/allow-shared-single".to_string()),
                chat_id: Some("chat-referenced".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        mark_worktree_old(&service, &referenced.worktree.meta.id, 48).await;

        let plan = service
            .cleanup_worktrees_dry_run(WorktreeCleanupRequest {
                ids: vec![referenced.worktree.meta.id.clone()],
                allow_shared: true,
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(plan.candidates.is_empty());
        assert!(plan.skipped.iter().any(|item| item.reason == "referenced"));
    }

    #[tokio::test]
    async fn worktree_hygiene_allow_shared_still_blocks_shared_references() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source).unwrap();
        let referenced = service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/allow-shared-many".to_string()),
                chat_id: Some("chat-referenced".to_string()),
                kind: Some("chat".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        service
            .add_reference(
                &referenced.worktree.meta.id,
                WorktreeReference {
                    kind: "chat".to_string(),
                    chat_id: Some("chat-referenced-2".to_string()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        mark_worktree_old(&service, &referenced.worktree.meta.id, 48).await;

        let plan = service
            .cleanup_worktrees_dry_run(WorktreeCleanupRequest {
                ids: vec![referenced.worktree.meta.id.clone()],
                allow_shared: true,
                ..Default::default()
            })
            .await
            .unwrap();

        assert!(plan.candidates.is_empty());
        assert!(plan.skipped.iter().any(|item| item.reason == "referenced"));
    }

    #[tokio::test]
    async fn worktree_registry_invalid_ids_paths_and_non_git_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        let service = test_service(cache, source).unwrap();
        assert!(service.get_worktree("../bad").await.is_err());
        assert!(service.worktree_path_for_id("../bad").is_err());
        assert!(service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("../bad".to_string()),
                ..Default::default()
            })
            .await
            .is_err());
        assert!(service
            .create_worktree(CreateWorktreeRequest {
                branch: Some("refact/chat/non-git".to_string()),
                ..Default::default()
            })
            .await
            .unwrap_err()
            .contains("not a git repository"));
        let mut registry = service.load_registry().await.unwrap();
        let expected_registry = registry.clone();
        let mut record = sample_record(&service, "wt_1");
        record.meta.root = temp.path().join("outside");
        registry.records.push(record);
        assert!(service
            .compare_and_save_registry(&expected_registry, &registry)
            .await
            .is_err());
        let mut registry = service.load_registry().await.unwrap();
        let expected_registry = registry.clone();
        let mut record = sample_record(&service, "wt_2");
        record.meta.root = service.registry_dir();
        registry.records.push(record);
        let error = service
            .compare_and_save_registry(&expected_registry, &registry)
            .await
            .unwrap_err();
        assert!(error.contains("must exactly match registry path"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn worktree_registry_rejects_symlink_escape_root() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let cache = temp.path().join("cache");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let service = test_service(cache, source).unwrap();
        std::fs::create_dir_all(service.registry_dir()).unwrap();
        std::os::unix::fs::symlink(&outside, service.registry_dir().join("wt_1")).unwrap();
        let mut registry = service.load_registry().await.unwrap();
        let expected_registry = registry.clone();
        let mut record = sample_record(&service, "wt_1");
        record.meta.root = outside;
        registry.records.push(record);

        let error = service
            .compare_and_save_registry(&expected_registry, &registry)
            .await
            .unwrap_err();

        assert!(error.contains("must exactly match registry path"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn worktree_registry_rejects_expected_root_symlink() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let cache = temp.path().join("cache");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let service = test_service(cache, source).unwrap();
        std::fs::create_dir_all(service.registry_dir()).unwrap();
        std::os::unix::fs::symlink(&outside, service.registry_dir().join("wt_1")).unwrap();
        let mut registry = service.load_registry().await.unwrap();
        let expected_registry = registry.clone();
        registry.records.push(sample_record(&service, "wt_1"));

        let error = service
            .compare_and_save_registry(&expected_registry, &registry)
            .await
            .unwrap_err();

        assert!(error.contains("must not be a symlink"));
    }

    #[tokio::test]
    async fn delete_worktree_refuses_referenced_unless_forced() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source.canonicalize().unwrap()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                kind: Some("task_agent".to_string()),
                chat_id: Some("agent-chat-1".to_string()),
                task_id: Some("task-1".to_string()),
                card_id: Some("T-1".to_string()),
                agent_id: Some("agent-1".to_string()),
                branch: Some("refact/task/task-1/card/T-1/agent-1".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let id = created.worktree.meta.id.clone();
        let root = created.worktree.meta.root.clone();

        let err = service.delete_worktree(&id, true, false).await.unwrap_err();
        assert!(
            err.contains("active reference") && err.contains("force_referenced=true"),
            "{err}"
        );
        assert!(root.exists(), "worktree must not be deleted on refusal");
        assert!(service.get_worktree(&id).await.is_ok());

        let deleted = service.delete_worktree(&id, true, true).await.unwrap();
        assert!(deleted.deleted);
        assert!(!root.exists());
    }

    #[tokio::test]
    async fn validate_worktree_meta_strict_rejects_missing_root() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let service = test_service(cache, source.canonicalize().unwrap()).unwrap();
        let created = service
            .create_worktree(CreateWorktreeRequest {
                kind: Some("task_agent".to_string()),
                chat_id: Some("agent-chat-2".to_string()),
                task_id: Some("task-1".to_string()),
                card_id: Some("T-2".to_string()),
                agent_id: Some("agent-2".to_string()),
                branch: Some("refact/task/task-1/card/T-2/agent-2".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        let meta = created.worktree.meta.clone();
        std::fs::remove_dir_all(&meta.root).unwrap();

        let err = service
            .validate_worktree_meta_strict(&meta)
            .await
            .unwrap_err();
        assert!(
            err.contains("no longer exists on disk"),
            "expected missing-root error, got: {err}"
        );
    }

    #[tokio::test]
    async fn validate_legacy_task_agent_worktree_meta_rejects_source_workspace_root() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let source = source.canonicalize().unwrap();
        let service = test_service(cache, source.clone()).unwrap();
        let meta = WorktreeMeta {
            id: "00000000-0000-0000-0000-000000000001".to_string(),
            kind: "task_agent".to_string(),
            root: source.clone(),
            source_workspace_root: source.clone(),
            repo_root: source.clone(),
            branch: Some("refact/task/task-1/card/T-1/legacy".to_string()),
            base_branch: Some("main".to_string()),
            base_commit: None,
            task_id: Some("task-1".to_string()),
            card_id: Some("T-1".to_string()),
            agent_id: Some("legacy".to_string()),
            enforce: true,
        };

        let err = service
            .validate_legacy_task_agent_worktree_meta(&meta)
            .await
            .unwrap_err();
        assert!(
            err.contains("equals the source workspace root"),
            "expected source-workspace rejection, got: {err}"
        );
    }

    #[tokio::test]
    async fn validate_legacy_task_agent_worktree_meta_rejects_root_outside_cache() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        let outside = temp.path().join("outside-worktree");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        init_repo(&source);
        let source = source.canonicalize().unwrap();
        let outside = outside.canonicalize().unwrap();
        let service = test_service(cache, source.clone()).unwrap();
        let meta = WorktreeMeta {
            id: "00000000-0000-0000-0000-000000000002".to_string(),
            kind: "task_agent".to_string(),
            root: outside,
            source_workspace_root: source.clone(),
            repo_root: source,
            branch: Some("refact/task/task-1/card/T-1/legacy".to_string()),
            base_branch: Some("main".to_string()),
            base_commit: None,
            task_id: Some("task-1".to_string()),
            card_id: Some("T-1".to_string()),
            agent_id: Some("legacy".to_string()),
            enforce: true,
        };

        let err = service
            .validate_legacy_task_agent_worktree_meta(&meta)
            .await
            .unwrap_err();
        assert!(
            err.contains("outside the Refact worktree cache"),
            "expected outside-cache rejection, got: {err}"
        );
    }

    #[tokio::test]
    async fn legacy_merge_identity_accepts_old_checkout_without_session_authority() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let source = source.canonicalize().unwrap();
        let service = test_service(temp.path().join("cache"), source.clone()).unwrap();
        let root = temp.path().join("legacy-checkout");
        let branch = "refact/task/task-1/card/T-1/legacy";
        run_git(
            &source,
            &[
                "worktree",
                "add",
                "-b",
                branch,
                root.to_str().unwrap(),
                "main",
            ],
        );
        let meta = WorktreeMeta {
            id: branch.into(),
            kind: "task_agent".into(),
            root: root.canonicalize().unwrap(),
            source_workspace_root: source.clone(),
            repo_root: source.clone(),
            branch: Some(branch.into()),
            base_branch: Some("main".into()),
            base_commit: None,
            task_id: Some("task-1".into()),
            card_id: Some("T-1".into()),
            agent_id: None,
            enforce: true,
        };
        assert!(service.validate_legacy_merge_identity(&meta).is_ok());
        assert!(service
            .validate_legacy_task_agent_worktree_meta(&meta)
            .await
            .is_err());
        let mut wrong_branch = meta.clone();
        wrong_branch.branch = Some("main".into());
        assert!(service
            .validate_legacy_merge_identity(&wrong_branch)
            .is_err());
        let foreign = temp.path().join("foreign");
        std::fs::create_dir_all(&foreign).unwrap();
        init_repo(&foreign);
        let mut impostor = meta.clone();
        impostor.root = foreign.canonicalize().unwrap();
        assert!(service.validate_legacy_merge_identity(&impostor).is_err());
        let fenced = service
            .fence_legacy_merge_worktree(&meta, "main")
            .await
            .unwrap()
            .acquire_merge_permit()
            .await
            .unwrap();
        run_git(&root, &["checkout", "-b", "legacy-cleanup-drift"]);
        assert!(fenced.cleanup_legacy_merge_source().await.is_err());
        assert!(root.exists());
        assert!(branch_exists(&source, branch));
        run_git(&root, &["checkout", branch]);
        assert_eq!(
            fenced.cleanup_legacy_merge_source().await.unwrap(),
            (true, true)
        );
        fenced.validate_effect_token().await.unwrap();
        assert!(!root.exists());
    }

    #[tokio::test]
    async fn validate_legacy_task_agent_worktree_meta_rejects_wrong_branch() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let cache = temp.path().join("cache");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let source = source.canonicalize().unwrap();
        let service = test_service(cache, source.clone()).unwrap();
        let id = "00000000-0000-0000-0000-000000000003";
        let root = service.worktree_path_for_id(id).unwrap();
        let expected_branch = "refact/task/task-1/card/T-1/legacy";
        std::fs::create_dir_all(service.registry_dir()).unwrap();
        run_git(
            &source,
            &[
                "worktree",
                "add",
                "-b",
                expected_branch,
                root.to_str().unwrap(),
                "main",
            ],
        );
        let meta = WorktreeMeta {
            id: id.to_string(),
            kind: "task_agent".to_string(),
            root: root.clone(),
            source_workspace_root: source.clone(),
            repo_root: source,
            branch: Some(expected_branch.to_string()),
            base_branch: Some("main".to_string()),
            base_commit: None,
            task_id: Some("task-1".to_string()),
            card_id: Some("T-1".to_string()),
            agent_id: Some("legacy".to_string()),
            enforce: true,
        };
        assert_eq!(
            service
                .validate_legacy_task_agent_worktree_meta(&meta)
                .await
                .unwrap(),
            meta
        );
        run_git(&root, &["checkout", "-b", "legacy-wrong-branch"]);

        let err = service
            .validate_legacy_task_agent_worktree_meta(&meta)
            .await
            .unwrap_err();

        assert!(err.contains("different branch"), "{err}");
        assert!(err.contains("legacy-wrong-branch"), "{err}");
        assert!(err.contains(expected_branch), "{err}");
    }
}
