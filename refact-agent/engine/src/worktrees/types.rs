use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub use refact_core::worktree_meta::WorktreeMeta;

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
        Self::Merge
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
pub struct MergeWorktreeRequest {
    #[serde(default)]
    pub strategy: WorktreeMergeStrategy,
    #[serde(default = "default_delete_after_merge")]
    pub delete_after_merge: bool,
    #[serde(default)]
    pub include_uncommitted: bool,
    #[serde(default)]
    pub target_branch: Option<String>,
    #[serde(default)]
    pub commit_message: Option<String>,
    #[serde(default)]
    pub generate_commit_message: bool,
}

impl Default for MergeWorktreeRequest {
    fn default() -> Self {
        Self {
            strategy: WorktreeMergeStrategy::default(),
            delete_after_merge: default_delete_after_merge(),
            include_uncommitted: false,
            target_branch: None,
            commit_message: None,
            generate_commit_message: false,
        }
    }
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
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_commit: Option<String>,
    pub status: WorktreeStatus,
    pub files: Vec<WorktreeDiffFile>,
    pub stats: WorktreeDiffStats,
    pub patch: String,
    pub patch_truncated: bool,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_worktree_meta() -> WorktreeMeta {
        WorktreeMeta {
            id: "wt-1".to_string(),
            kind: "task_agent".to_string(),
            root: PathBuf::from("/tmp/refact-worktree"),
            source_workspace_root: PathBuf::from("/tmp/refact-source"),
            repo_root: PathBuf::from("/tmp/refact-source"),
            branch: Some("refact/task/t/card/a".to_string()),
            base_branch: Some("main".to_string()),
            base_commit: Some("abc123".to_string()),
            task_id: Some("task-1".to_string()),
            card_id: Some("card-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            enforce: true,
        }
    }

    #[test]
    fn worktree_meta_serde_roundtrip() {
        let meta = sample_worktree_meta();
        let json = serde_json::to_value(&meta).unwrap();
        assert_eq!(json["id"], "wt-1");
        assert_eq!(json["kind"], "task_agent");
        assert_eq!(json["root"], "/tmp/refact-worktree");
        assert_eq!(json["source_workspace_root"], "/tmp/refact-source");
        assert_eq!(json["repo_root"], "/tmp/refact-source");
        assert_eq!(json["enforce"], true);
        let roundtrip: WorktreeMeta = serde_json::from_value(json).unwrap();
        assert_eq!(roundtrip, meta);
    }

    #[test]
    fn worktree_meta_enforce_defaults_false() {
        let json = serde_json::json!({
            "id": "wt-1",
            "kind": "manual",
            "root": "/tmp/wt",
            "source_workspace_root": "/tmp/src",
            "repo_root": "/tmp/src"
        });
        let meta: WorktreeMeta = serde_json::from_value(json).unwrap();
        assert!(!meta.enforce);
    }

    #[test]
    fn merge_worktree_request_defaults_delete_after_merge_true() {
        let request: MergeWorktreeRequest = serde_json::from_value(serde_json::json!({})).unwrap();

        assert!(request.delete_after_merge);
    }

    #[test]
    fn merge_worktree_request_respects_explicit_keep_after_merge() {
        let request: MergeWorktreeRequest = serde_json::from_value(serde_json::json!({
            "delete_after_merge": false
        }))
        .unwrap();

        assert!(!request.delete_after_merge);
    }
}
