use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub use refact_core::worktree_meta::WorktreeMeta;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorktreeSyncRequest {
    Status {
        #[serde(default)]
        target_branch: Option<String>,
    },
    Recover {
        expected_journal_digest: String,
    },
    Start {
        target_branch: Option<String>,
        expected_source_head: String,
        expected_target_head: String,
        #[serde(default)]
        checkpoint: bool,
    },
    Resolve {
        operation_id: String,
        expected_revision: u64,
        resolutions: Vec<WorktreeSyncResolution>,
    },
    Continue {
        operation_id: String,
        expected_revision: u64,
        commit_message: Option<String>,
    },
    Restore {
        operation_id: String,
        expected_revision: u64,
        archive_id: Option<String>,
    },
    Abort {
        operation_id: String,
        expected_revision: u64,
        #[serde(default)]
        force: bool,
    },
}

impl<'de> Deserialize<'de> for WorktreeSyncRequest {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
        enum Wire {
            Status {
                #[serde(default)]
                target_branch: Option<String>,
            },
            Recover {
                expected_journal_digest: String,
            },
            Start {
                target_branch: Option<String>,
                expected_source_head: String,
                expected_target_head: String,
                #[serde(default)]
                checkpoint: bool,
            },
            Resolve {
                operation_id: String,
                expected_revision: u64,
                resolutions: Vec<WorktreeSyncResolution>,
            },
            Continue {
                operation_id: String,
                expected_revision: u64,
                commit_message: Option<String>,
            },
            Restore {
                operation_id: String,
                expected_revision: u64,
                archive_id: Option<String>,
            },
            Abort {
                operation_id: String,
                expected_revision: u64,
                #[serde(default)]
                force: bool,
            },
        }
        Ok(match Wire::deserialize(deserializer)? {
            Wire::Status {
                target_branch,
            } => Self::Status {
                target_branch,
            },
            Wire::Recover {
                expected_journal_digest,
            } => Self::Recover {
                expected_journal_digest,
            },
            Wire::Start {
                target_branch,
                expected_source_head,
                expected_target_head,
                checkpoint,
            } => Self::Start {
                target_branch,
                expected_source_head,
                expected_target_head,
                checkpoint,
            },
            Wire::Resolve {
                operation_id,
                expected_revision,
                resolutions,
            } => Self::Resolve {
                operation_id,
                expected_revision,
                resolutions,
            },
            Wire::Continue {
                operation_id,
                expected_revision,
                commit_message,
            } => Self::Continue {
                operation_id,
                expected_revision,
                commit_message,
            },
            Wire::Restore {
                operation_id,
                expected_revision,
                archive_id,
            } => Self::Restore {
                operation_id,
                expected_revision,
                archive_id,
            },
            Wire::Abort {
                operation_id,
                expected_revision,
                force,
            } => Self::Abort {
                operation_id,
                expected_revision,
                force,
            },
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorktreeSyncResolution {
    pub path: String,
    pub choice: WorktreeSyncResolutionChoice,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeSyncResolutionChoice {
    WorkingTree,
    Source,
    Target,
    Delete,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeSyncStatus {
    Idle,
    Conflicts,
    Ready,
    Completed,
    Aborted,
    RecoveryRequired,
    TargetAdvanced,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorktreeSyncOperation {
    #[serde(default)]
    pub initiator: Option<String>,
    pub operation_id: String,
    pub revision: u64,
    pub source_head: String,
    pub target_branch: String,
    pub target_head: String,
    pub phase: String,
    pub checkpoint_commit: Option<String>,
    pub recovery_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorktreeSyncStage {
    pub oid: String,
    pub mode: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorktreeSyncConflict {
    pub path: String,
    pub kind: String,
    pub base: Option<WorktreeSyncStage>,
    pub source: Option<WorktreeSyncStage>,
    pub target: Option<WorktreeSyncStage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorktreeSyncArchive {
    pub archive_id: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorktreeSyncResponse {
    #[serde(default)]
    pub recovery_journal_digest: Option<String>,
    #[serde(default)]
    pub recovery_archives: Vec<WorktreeSyncArchive>,
    #[serde(default)]
    pub restore_archive_id: Option<String>,
    pub id: String,
    pub status: WorktreeSyncStatus,
    pub source_head: String,
    pub target_branch: Option<String>,
    pub target_head: Option<String>,
    pub operation: Option<WorktreeSyncOperation>,
    pub conflicts: Vec<WorktreeSyncConflict>,
    pub warnings: Vec<String>,
    pub instructions: String,
}

fn serialize_path<S: serde::Serializer>(path: &PathBuf, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&path.to_string_lossy())
}

fn deserialize_path<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<PathBuf, D::Error> {
    Ok(PathBuf::from(String::deserialize(deserializer)?))
}

fn default_registry_schema_version() -> u32 {
    1
}

fn default_cleanup_clean_only() -> bool {
    true
}

fn default_cleanup_min_age_hours() -> u64 {
    24
}

fn default_delete_after_merge() -> bool {
    true
}

fn default_include_uncommitted() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct WorktreeReference {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

impl WorktreeReference {
    pub fn has_identity(&self) -> bool {
        self.chat_id.is_some()
            || self.task_id.is_some()
            || self.card_id.is_some()
            || self.agent_id.is_some()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct WorktreeStatus {
    pub path_exists: bool,
    pub is_git_worktree: bool,
    pub dirty: bool,
    #[serde(default)]
    pub conflicted: bool,
    pub staged_count: usize,
    pub unstaged_count: usize,
    pub untracked_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorktreeRegistryRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub creator_chat_id: Option<String>,
    pub meta: WorktreeMeta,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<String>,
    #[serde(default)]
    pub references: Vec<WorktreeReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_known_status: Option<WorktreeStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorktreeRegistry {
    #[serde(default = "default_registry_schema_version")]
    pub schema_version: u32,
    #[serde(
        serialize_with = "serialize_path",
        deserialize_with = "deserialize_path"
    )]
    pub source_workspace_root: PathBuf,
    pub project_hash: String,
    #[serde(default)]
    pub records: Vec<WorktreeRegistryRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorktreeRecordView {
    pub meta: WorktreeMeta,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<String>,
    pub references: Vec<WorktreeReference>,
    pub reference_count: usize,
    pub status: WorktreeStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_report: Option<CachePreparationReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CacheOutcome {
    Applied,
    Reused,
    Miss,
    Degraded,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CacheReason {
    CheckoutLocal,
    VerifiedCopy,
    VerifiedExisting,
    NoTrustedEntry,
    PackageAbsent,
    UnsafeDaemonBoundary,
    UnverifiedIsolation,
    UntrustedController,
    UntrustedProducer,
    InputMismatch,
    CorruptBlob,
    UnsafePath,
    ExistingUnownedOutput,
    IdentityChanged,
    LimitExceeded,
    Cancelled,
    IoFailure,
    UnsupportedPlatform,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CacheComponentReport {
    pub outcome: CacheOutcome,
    pub reason: CacheReason,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CacheTrustBoundary {
    Unverified,
    RootProtectedHostStore,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CachePreparationReport {
    pub schema_version: u32,
    pub cargo_target: CacheComponentReport,
    pub compiler_cache: CacheComponentReport,
    pub dependencies: CacheComponentReport,
    pub trust: CacheTrustBoundary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorktreeListResponse {
    pub project_hash: String,
    #[serde(
        serialize_with = "serialize_path",
        deserialize_with = "deserialize_path"
    )]
    pub source_workspace_root: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_current_branch: Option<String>,
    #[serde(default)]
    pub source_branches: Vec<String>,
    pub worktrees: Vec<WorktreeRecordView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct CreateWorktreeRequest {
    #[serde(default)]
    pub source_workspace_root: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub base_branch: Option<String>,
    #[serde(default)]
    pub base_commit: Option<String>,
    #[serde(default)]
    pub chat_id: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub card_id: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreateWorktreeResponse {
    pub worktree: WorktreeRecordView,
    pub branch_was_created: bool,
    pub dirty_source_warning: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DeleteWorktreeRequest {
    #[serde(default)]
    pub delete_branch: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeleteWorktreeResponse {
    pub deleted: bool,
    pub branch_deleted: bool,
    pub stale_path: bool,
    pub affected_references: Vec<WorktreeReference>,
    pub affected_reference_count: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorktreeRemovalResult {
    pub worktree_deleted: bool,
    pub branch_deleted: bool,
    pub registry_deleted: bool,
    pub stale_path: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorktreeMergeStrategy {
    Merge,
    Squash,
}

impl Default for WorktreeMergeStrategy {
    fn default() -> Self {
        Self::Squash
    }
}

impl WorktreeMergeStrategy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Squash => "squash",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CheckedMergeIdentity {
    pub source_head: String,
    pub target_head: String,
    pub target_branch: String,
    pub patch_digest: String,
    pub target_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MergeWorktreeRequest {
    #[serde(default)]
    pub strategy: WorktreeMergeStrategy,
    #[serde(default = "default_delete_after_merge")]
    pub delete_after_merge: bool,
    #[serde(default = "default_include_uncommitted")]
    pub include_uncommitted: bool,
    #[serde(default)]
    pub target_branch: Option<String>,
    #[serde(default)]
    pub commit_message: Option<String>,
    #[serde(default)]
    pub generate_commit_message: bool,
    /// Uncommitted work in the target checkout refuses the merge unless the
    /// caller acknowledges it here; the acknowledged path stashes and restores
    /// it rather than discarding it.
    #[serde(default)]
    pub acknowledge_dirty_baseline: bool,
}

impl Default for MergeWorktreeRequest {
    fn default() -> Self {
        Self {
            strategy: WorktreeMergeStrategy::default(),
            delete_after_merge: default_delete_after_merge(),
            include_uncommitted: default_include_uncommitted(),
            target_branch: None,
            commit_message: None,
            generate_commit_message: false,
            acknowledge_dirty_baseline: false,
        }
    }
}

/// Uncommitted work found in a target checkout before a merge touched it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DirtyBaseline {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub files: Vec<String>,
    #[serde(default)]
    pub files_truncated: bool,
    pub staged_count: usize,
    pub unstaged_count: usize,
    pub untracked_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorktreeConflictState {
    pub files: Vec<String>,
    pub aborted: bool,
    pub merge_in_progress: bool,
    pub instructions: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MergeWorktreeResponse {
    pub id: String,
    pub status: String,
    pub merged: bool,
    pub strategy: String,
    pub source_branch: String,
    pub target_branch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub committed_uncommitted: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cleanup: Option<WorktreeRemovalResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflict: Option<WorktreeConflictState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dirty_baseline: Option<DirtyBaseline>,
    pub affected_references: Vec<WorktreeReference>,
    pub affected_reference_count: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OpenWorktreeResponse {
    pub id: String,
    #[serde(
        serialize_with = "serialize_path",
        deserialize_with = "deserialize_path"
    )]
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub can_open_folder: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorktreeDiffFile {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_path: Option<String>,
    pub status: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additions: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deletions: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct WorktreeDiffStats {
    pub committed_files: usize,
    pub staged_files: usize,
    pub unstaged_files: usize,
    pub untracked_files: usize,
    pub files_changed: usize,
    #[serde(default)]
    pub additions: usize,
    #[serde(default)]
    pub deletions: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorktreeDiffResponse {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_base_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ahead: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behind: Option<usize>,
    pub status: WorktreeStatus,
    pub files: Vec<WorktreeDiffFile>,
    pub stats: WorktreeDiffStats,
    pub patch: String,
    pub patch_truncated: bool,
    #[serde(default)]
    pub patch_shown_bytes: usize,
    #[serde(default)]
    pub patch_total_bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorktreeInventory {
    pub project_hash: String,
    #[serde(
        serialize_with = "serialize_path",
        deserialize_with = "deserialize_path"
    )]
    pub source_workspace_root: PathBuf,
    pub generated_at: String,
    pub summary: WorktreeInventorySummary,
    pub worktrees: Vec<WorktreeInspection>,
    pub cleanup_candidates: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct WorktreeInventorySummary {
    pub total_registered: usize,
    pub total_discovered: usize,
    pub total: usize,
    pub clean: usize,
    pub dirty: usize,
    pub unknown: usize,
    pub stale: usize,
    pub conflicted: usize,
    pub shared: usize,
    pub abandoned_clean: usize,
    pub changed_files: usize,
    pub additions: usize,
    pub deletions: usize,
    pub missing_registry_paths: usize,
    pub unregistered_cache_dirs: usize,
    pub merged_branches: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub newest_age_hours: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oldest_age_hours: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk_usage_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorktreeInspection {
    pub id: String,
    pub source: String,
    #[serde(
        serialize_with = "serialize_path",
        deserialize_with = "deserialize_path"
    )]
    pub root: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_commit: Option<String>,
    pub status: WorktreeStatus,
    pub references: Vec<WorktreeReference>,
    pub reference_count: usize,
    pub shared: bool,
    pub stale: bool,
    pub conflicted: bool,
    pub changed_files: usize,
    pub committed_files: usize,
    pub staged_files: usize,
    pub unstaged_files: usize,
    pub untracked_files: usize,
    pub additions: usize,
    pub deletions: usize,
    pub cleanup_candidate: bool,
    pub cleanup_blockers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk_usage_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age_hours: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch_merged: Option<bool>,
    #[serde(default)]
    pub registry_missing: bool,
    #[serde(default)]
    pub cache_dir_missing_from_registry: bool,
    #[serde(default)]
    pub attached_chat_ids: Vec<String>,
    #[serde(default)]
    pub attached_task_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct WorktreeCleanupRequest {
    pub ids: Vec<String>,
    pub clean_only: bool,
    pub delete_branches: bool,
    pub allow_shared: bool,
    pub min_age_hours: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_workspace_root: Option<String>,
}

impl Default for WorktreeCleanupRequest {
    fn default() -> Self {
        Self {
            ids: Vec::new(),
            clean_only: default_cleanup_clean_only(),
            delete_branches: false,
            allow_shared: false,
            min_age_hours: default_cleanup_min_age_hours(),
            source_workspace_root: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorktreeCleanupPlan {
    pub generated_at: String,
    pub request: WorktreeCleanupRequest,
    pub candidates: Vec<WorktreeCleanupTarget>,
    pub skipped: Vec<WorktreeCleanupSkipped>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorktreeCleanupTarget {
    pub id: String,
    #[serde(
        serialize_with = "serialize_path",
        deserialize_with = "deserialize_path"
    )]
    pub root: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub shared: bool,
    pub stale: bool,
    pub changed_files: usize,
    pub additions: usize,
    pub deletions: usize,
    pub delete_branch: bool,
    pub references: Vec<WorktreeReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk_usage_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorktreeCleanupSkipped {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<PathBuf>,
    pub reason: String,
    pub details: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorktreeCleanupDeleted {
    pub id: String,
    #[serde(
        serialize_with = "serialize_path",
        deserialize_with = "deserialize_path"
    )]
    pub root: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub worktree_deleted: bool,
    pub branch_deleted: bool,
    pub registry_deleted: bool,
    pub stale_path: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorktreeCleanupResult {
    pub generated_at: String,
    pub request: WorktreeCleanupRequest,
    pub deleted: Vec<WorktreeCleanupDeleted>,
    pub skipped: Vec<WorktreeCleanupSkipped>,
    pub warnings: Vec<String>,
}

/// Exact read-only preimage for recoverable checkout quarantine. Branches are never deleted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorktreeArchiveIdentity {
    pub record: WorktreeRegistryRecord,
    pub head: String,
    pub version: String,
}
