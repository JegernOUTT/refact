use std::path::PathBuf;

use serde::{Deserialize, Serialize};

fn serialize_path<S: serde::Serializer>(path: &PathBuf, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&path.to_string_lossy())
}

fn deserialize_path<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<PathBuf, D::Error> {
    Ok(PathBuf::from(String::deserialize(deserializer)?))
}

fn default_enforce() -> bool {
    false
}

fn default_registry_schema_version() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorktreeMeta {
    pub id: String,
    pub kind: String,
    #[serde(
        serialize_with = "serialize_path",
        deserialize_with = "deserialize_path"
    )]
    pub root: PathBuf,
    #[serde(
        serialize_with = "serialize_path",
        deserialize_with = "deserialize_path"
    )]
    pub source_workspace_root: PathBuf,
    #[serde(
        serialize_with = "serialize_path",
        deserialize_with = "deserialize_path"
    )]
    pub repo_root: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default = "default_enforce")]
    pub enforce: bool,
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
pub struct WorktreeCleanupResult {
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct MergeWorktreeRequest {
    #[serde(default)]
    pub strategy: WorktreeMergeStrategy,
    #[serde(default)]
    pub delete_after_merge: bool,
    #[serde(default)]
    pub include_uncommitted: bool,
    #[serde(default)]
    pub target_branch: Option<String>,
    #[serde(default)]
    pub commit_message: Option<String>,
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
    pub cleanup: Option<WorktreeCleanupResult>,
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
}
