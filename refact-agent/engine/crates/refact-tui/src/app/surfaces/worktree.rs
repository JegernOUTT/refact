use crate::client::{
    MergeWorktreeResponse, WorktreeCleanupPlanResponse, WorktreeCleanupRequest,
    WorktreeCleanupResultResponse, WorktreeDiffResponse, WorktreeInventoryResponse,
    WorktreeListResponse, WorktreeRecordResponse,
};
use crate::overlay::PagerOverlay;
use crate::text_safety::{sanitize_tool_inline, sanitize_tool_text};

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeAction {
    List,
    Summary,
    Inspect {
        id: String,
    },
    Diff {
        id: String,
    },
    Create {
        branch: Option<String>,
    },
    Open {
        id: String,
    },
    Delete {
        id: String,
    },
    CleanupPlan {
        request: WorktreeCleanupRequest,
    },
    Cleanup {
        request: WorktreeCleanupRequest,
    },
    Merge {
        confirmation: WorktreeMergeConfirmation,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeMergeConfirmation {
    pub id: String,
    pub strategy: String,
    pub target_branch: String,
    pub include_uncommitted: bool,
    pub delete_after_merge: bool,
}

fn worktree_surfaces_enabled() -> bool {
    worktree_surfaces_enabled_from(std::env::var("REFACT_TUI_SURFACES").ok().as_deref())
}

fn worktree_surfaces_enabled_from(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

impl App {
    pub(in crate::app) fn start_worktree_command(&mut self, args: &str) -> AppAction {
        self.start_worktree_command_with_enabled(args, worktree_surfaces_enabled())
    }

    fn start_worktree_command_with_enabled(&mut self, args: &str, enabled: bool) -> AppAction {
        self.composer.clear();
        if !enabled {
            self.add_notice("/worktrees requires REFACT_TUI_SURFACES=1");
            return AppAction::None;
        }

        let mut parts = args.split_whitespace();
        let action = match parts.next() {
            None | Some("list") => WorktreeAction::List,
            Some("summary") => WorktreeAction::Summary,
            Some("create") => WorktreeAction::Create {
                branch: parts.next().map(str::to_string),
            },
            Some("show") | Some("inspect") => match parts.next() {
                Some(id) => WorktreeAction::Inspect { id: id.to_string() },
                None => return self.worktree_usage(),
            },
            Some("diff") => match parts.next() {
                Some(id) => WorktreeAction::Diff { id: id.to_string() },
                None => return self.worktree_usage(),
            },
            Some("open") => match parts.next() {
                Some(id) => WorktreeAction::Open { id: id.to_string() },
                None => return self.worktree_usage(),
            },
            Some("delete") => match parts.next() {
                Some(id) => WorktreeAction::Delete { id: id.to_string() },
                None => return self.worktree_usage(),
            },
            Some("cleanup") => {
                let mut ids = Vec::new();
                let apply = match parts.next() {
                    Some("apply") => true,
                    Some(id) => {
                        ids.push(id.to_string());
                        false
                    }
                    None => false,
                };
                ids.extend(parts.map(str::to_string));
                let request = WorktreeCleanupRequest {
                    ids,
                    ..WorktreeCleanupRequest::default()
                };
                if apply {
                    WorktreeAction::Cleanup { request }
                } else {
                    WorktreeAction::CleanupPlan { request }
                }
            }
            Some("merge") => {
                let Some(id) = parts.next() else {
                    return self.worktree_usage();
                };
                let strategy = parts.next().unwrap_or("squash");
                if !matches!(strategy, "squash" | "merge") {
                    self.add_notice("/worktrees merge strategy must be squash or merge");
                    return AppAction::None;
                }
                let Some(target_branch) = parts.next() else {
                    self.add_notice("/worktrees merge requires an exact target branch");
                    return AppAction::None;
                };
                let mut include_uncommitted = false;
                let mut delete_after_merge = true;
                for option in parts {
                    match option {
                        "include-uncommitted" => include_uncommitted = true,
                        "committed-only" => include_uncommitted = false,
                        "delete-source" => delete_after_merge = true,
                        "keep-source" => delete_after_merge = false,
                        _ => return self.worktree_usage(),
                    }
                }
                let confirmation = WorktreeMergeConfirmation {
                    id: id.to_string(),
                    strategy: strategy.to_string(),
                    target_branch: target_branch.to_string(),
                    include_uncommitted,
                    delete_after_merge,
                };
                self.add_notice(worktree_merge_confirmation_notice(&confirmation));
                self.pending_worktree_merge = Some(confirmation);
                return AppAction::None;
            }
            Some(_) => return self.worktree_usage(),
        };
        AppAction::Worktree { action }
    }

    pub(in crate::app) fn handle_worktree_list_loaded(
        &mut self,
        result: Result<WorktreeListResponse, String>,
    ) {
        match result {
            Ok(response) => self.open_worktree_overlay(worktree_list_lines(&response)),
            Err(error) => self.handle_worktree_error("list worktrees", error),
        }
    }

    pub(in crate::app) fn handle_worktree_summary_loaded(
        &mut self,
        result: Result<WorktreeInventoryResponse, String>,
    ) {
        match result {
            Ok(response) => self.open_worktree_overlay(worktree_summary_lines(&response)),
            Err(error) => self.handle_worktree_error("load worktree summary", error),
        }
    }

    pub(in crate::app) fn handle_worktree_loaded(
        &mut self,
        result: Result<WorktreeRecordResponse, String>,
    ) {
        match result {
            Ok(response) => self.open_worktree_overlay(worktree_record_lines(&response)),
            Err(error) => self.handle_worktree_error("load worktree", error),
        }
    }

    pub(in crate::app) fn handle_worktree_diff_loaded(
        &mut self,
        result: Result<WorktreeDiffResponse, String>,
    ) {
        match result {
            Ok(response) => self.open_worktree_overlay(worktree_diff_lines(&response)),
            Err(error) => self.handle_worktree_error("load worktree diff", error),
        }
    }

    pub(in crate::app) fn handle_worktree_cleanup_plan_loaded(
        &mut self,
        result: Result<WorktreeCleanupPlanResponse, String>,
    ) {
        match result {
            Ok(plan) => {
                let request = plan.request.clone();
                self.open_worktree_overlay(worktree_cleanup_plan_lines(&plan));
                if plan.candidates.is_empty() {
                    self.add_notice("Worktree cleanup found no deletable candidates");
                } else {
                    self.add_notice(format!(
                        "Cleanup plan has {} candidate(s); run /worktrees cleanup apply {} to execute",
                        plan.candidates.len(),
                        request.ids.join(" ")
                    ));
                }
            }
            Err(error) => self.handle_worktree_error("plan worktree cleanup", error),
        }
    }

    pub(in crate::app) fn handle_worktree_cleanup_finished(
        &mut self,
        result: Result<WorktreeCleanupResultResponse, String>,
    ) {
        match result {
            Ok(response) => {
                self.open_worktree_overlay(worktree_cleanup_result_lines(&response));
                if !response.warnings.is_empty() {
                    self.add_notice(format!(
                        "Worktree cleanup completed with {} warning(s); inspect the cleanup result",
                        response.warnings.len()
                    ));
                }
            }
            Err(error) => self.handle_worktree_error("clean up worktrees", error),
        }
    }

    pub(in crate::app) fn handle_worktree_created(
        &mut self,
        result: Result<crate::client::CreateWorktreeResponse, String>,
    ) {
        match result {
            Ok(response) => {
                self.open_worktree_overlay(worktree_record_lines(&response.worktree));
                self.add_notice(format!(
                    "Created worktree {} on branch {}",
                    response.worktree.meta.id,
                    unknown(&response.worktree.meta.branch)
                ));
            }
            Err(error) => self.handle_worktree_error("create worktree", error),
        }
    }

    pub(in crate::app) fn handle_worktree_merge_finished(
        &mut self,
        result: Result<MergeWorktreeResponse, String>,
    ) {
        match result {
            Ok(response) => {
                let lines = worktree_merge_result_lines(&response);
                self.open_worktree_overlay(lines);
                self.add_notice(worktree_merge_result_notice(&response));
            }
            Err(error) => self.handle_worktree_error("merge worktree", error),
        }
    }

    pub(in crate::app) fn handle_worktree_opened(
        &mut self,
        result: Result<crate::client::OpenWorktreeResponse, String>,
    ) {
        match result {
            Ok(response) if response.can_open_folder => self.add_notice(format!(
                "Worktree {} is at {}. Open that folder in your IDE to work there.",
                response.id,
                response.path.display()
            )),
            Ok(response) => self.add_notice(format!(
                "Worktree {} is at {}, but the server cannot open folders here.",
                response.id,
                response.path.display()
            )),
            Err(error) => self.handle_worktree_error("open worktree", error),
        }
    }

    pub(in crate::app) fn handle_worktree_deleted(
        &mut self,
        result: Result<crate::client::DeleteWorktreeResponse, String>,
    ) {
        match result {
            Ok(response) if response.deleted => {
                let warning = (!response.warnings.is_empty())
                    .then(|| format!(" with {} warning(s)", response.warnings.len()));
                self.add_notice(format!("Worktree deleted{}", warning.unwrap_or_default()));
            }
            Ok(response) => self.add_notice(format!(
                "Worktree was not deleted; {} affected reference(s) still need attention",
                response.affected_reference_count
            )),
            Err(error) => self.handle_worktree_error("delete worktree", error),
        }
    }

    pub(crate) fn worktree_merge_confirmation_lines(&self) -> Option<Vec<String>> {
        self.pending_worktree_merge.as_ref().map(|confirmation| {
            vec![
                "Confirm worktree merge".to_string(),
                format!("worktree: {}", confirmation.id),
                format!("strategy: {}", confirmation.strategy),
                format!("target branch: {}", confirmation.target_branch),
                format!(
                    "uncommitted files: {}",
                    if confirmation.include_uncommitted {
                        "include and commit"
                    } else {
                        "exclude"
                    }
                ),
                format!(
                    "after merge: {}",
                    if confirmation.delete_after_merge {
                        "delete source worktree"
                    } else {
                        "keep source worktree"
                    }
                ),
                "This is a local merge, not a push. Press Enter to merge locally or Esc to cancel."
                    .to_string(),
            ]
        })
    }

    fn open_worktree_overlay(&mut self, lines: Vec<String>) {
        let raw_lines = lines.clone();
        self.transcript_overlay = Some(PagerOverlay::new("Worktrees", lines, raw_lines));
    }

    fn handle_worktree_error(&mut self, operation: &str, error: String) {
        self.retry_hint = retry_hint_from_message(&error);
        let notice = if operation == "merge worktree" {
            worktree_merge_error_notice(&error)
        } else {
            format!("Failed to {operation}: {error}")
        };
        self.add_notice(notice);
    }

    fn worktree_usage(&mut self) -> AppAction {
        self.add_notice(
            "/worktrees [list|summary|create|show ID|diff ID|open ID|delete ID|cleanup [apply] [ID...]|merge ID [squash|merge] TARGET [include-uncommitted|committed-only] [delete-source|keep-source]]",
        );
        AppAction::None
    }
}

fn worktree_list_lines(response: &WorktreeListResponse) -> Vec<String> {
    let mut lines = vec![
        "Worktrees".to_string(),
        format!("source: {}", response.source_workspace_root.display()),
    ];
    if let Some(branch) = response.source_current_branch.as_deref() {
        lines.push(format!("source branch: {branch}"));
    }
    lines.push(format!("registered: {}", response.worktrees.len()));
    for worktree in &response.worktrees {
        let branch = unknown(&worktree.meta.branch);
        let state = worktree_state(&worktree.status);
        lines.push(format!(
            "{} · {branch} · {state} · {}",
            worktree.meta.id,
            worktree.meta.root.display()
        ));
    }
    if response.worktrees.is_empty() {
        lines.push("No registered worktrees.".to_string());
    }
    lines
}

fn worktree_summary_lines(response: &WorktreeInventoryResponse) -> Vec<String> {
    vec![
        "Worktree summary".to_string(),
        format!("source: {}", response.source_workspace_root.display()),
        format!(
            "total: {} · clean: {} · dirty: {} · stale: {} · conflicted: {}",
            optional_count(response.summary.total),
            optional_count(response.summary.clean),
            optional_count(response.summary.dirty),
            optional_count(response.summary.stale),
            optional_count(response.summary.conflicted)
        ),
        format!("cleanup candidates: {}", response.cleanup_candidates.len()),
    ]
}

fn worktree_record_lines(record: &WorktreeRecordResponse) -> Vec<String> {
    let meta = &record.meta;
    let mut lines = vec![
        format!("Worktree {}", meta.id),
        format!("root: {}", meta.root.display()),
        format!("branch: {}", unknown(&meta.branch)),
        format!("base branch: {}", optional(&meta.base_branch)),
        format!("base commit: {}", optional(&meta.base_commit)),
        format!("state: {}", worktree_state(&record.status)),
        format!("references: {}", record.reference_count),
    ];
    append_optional_identity(&mut lines, "task", &meta.task_id);
    append_optional_identity(&mut lines, "card", &meta.card_id);
    append_optional_identity(&mut lines, "agent", &meta.agent_id);
    lines
}

fn worktree_diff_lines(response: &WorktreeDiffResponse) -> Vec<String> {
    let mut lines = vec![
        format!("Worktree diff: {}", response.id),
        format!("branch: {}", optional(&response.branch)),
        format!("base: {}", optional(&response.base_branch)),
        format!("state: {}", worktree_state(&response.status)),
        format!(
            "changed: {} files · +{} -{}",
            optional_count(response.stats.files_changed),
            optional_count(response.stats.additions),
            optional_count(response.stats.deletions)
        ),
    ];
    if let (Some(ahead), Some(behind)) = (response.ahead, response.behind) {
        lines.push(format!("commits: {ahead} ahead · {behind} behind"));
    }
    if response.patch_truncated {
        lines.push(
            "Patch is truncated; inspect the worktree directly for the complete diff.".to_string(),
        );
    }
    for file in &response.files {
        lines.push(format!(
            "{} {}",
            file.status,
            sanitize_tool_inline(&file.path)
        ));
    }
    let patch = sanitize_tool_text(&response.patch);
    if !patch.is_empty() {
        lines.push(String::new());
        lines.extend(patch.lines().map(str::to_string));
    }
    lines
}

fn worktree_cleanup_plan_lines(plan: &WorktreeCleanupPlanResponse) -> Vec<String> {
    let mut lines = vec![
        "Worktree cleanup plan".to_string(),
        format!(
            "candidates: {} · skipped: {}",
            plan.candidates.len(),
            plan.skipped.len()
        ),
    ];
    for candidate in &plan.candidates {
        lines.push(format!(
            "delete {} · {} · {} changed file(s)",
            candidate.id,
            optional(&candidate.branch),
            candidate.changed_files
        ));
    }
    for skipped in &plan.skipped {
        lines.push(format!("skip {}: {}", skipped.id, skipped.reason));
    }
    lines
}

fn worktree_cleanup_result_lines(result: &WorktreeCleanupResultResponse) -> Vec<String> {
    let mut lines = vec![
        "Worktree cleanup result".to_string(),
        format!(
            "deleted: {} · skipped: {}",
            result.deleted.len(),
            result.skipped.len()
        ),
    ];
    for deleted in &result.deleted {
        lines.push(format!(
            "deleted {} · worktree={} branch={} registry={}",
            deleted.id, deleted.worktree_deleted, deleted.branch_deleted, deleted.registry_deleted
        ));
        lines.extend(
            deleted
                .warnings
                .iter()
                .map(|warning| format!("warning: {warning}")),
        );
    }
    for skipped in &result.skipped {
        lines.push(format!("skipped {}: {}", skipped.id, skipped.reason));
    }
    lines.extend(
        result
            .warnings
            .iter()
            .map(|warning| format!("warning: {warning}")),
    );
    lines
}

fn worktree_merge_confirmation_notice(confirmation: &WorktreeMergeConfirmation) -> String {
    let uncommitted = if confirmation.include_uncommitted {
        "include and commit uncommitted files"
    } else {
        "exclude uncommitted files"
    };
    let cleanup = if confirmation.delete_after_merge {
        "delete the source worktree afterward"
    } else {
        "keep the source worktree afterward"
    };
    format!(
        "Confirm local {} merge of {} into {}: {}; {}. This merge does not push. Push {} separately after it succeeds.",
        confirmation.strategy,
        confirmation.id,
        confirmation.target_branch,
        uncommitted,
        cleanup,
        confirmation.target_branch
    )
}

fn worktree_merge_result_lines(response: &MergeWorktreeResponse) -> Vec<String> {
    let mut lines = vec![
        format!("Worktree merge: {}", response.id),
        format!(
            "strategy: {} · {} -> {}",
            response.strategy, response.source_branch, response.target_branch
        ),
    ];
    match response.status.as_str() {
        "merged" => lines.push("Merge completed locally.".to_string()),
        "nothing_to_merge" => lines.push(
            "Nothing to merge: the target already contains this worktree's commits.".to_string(),
        ),
        "conflict" => {
            lines.push("Merge did not complete because conflicts were found.".to_string());
            if let Some(conflict) = response.conflict.as_ref() {
                lines.push(format!(
                    "rollback: {} · merge in progress: {}",
                    if conflict.aborted {
                        "completed"
                    } else {
                        "needs attention"
                    },
                    conflict.merge_in_progress
                ));
                lines.extend(
                    conflict
                        .files
                        .iter()
                        .map(|file| format!("conflict: {file}")),
                );
                if !conflict.instructions.is_empty() {
                    lines.push(conflict.instructions.clone());
                }
            }
        }
        status => lines.push(format!("Merge returned status: {status}")),
    }
    if let Some(cleanup) = response.cleanup.as_ref() {
        lines.push(format!(
            "cleanup: worktree={} branch={} registry={}",
            cleanup.worktree_deleted, cleanup.branch_deleted, cleanup.registry_deleted
        ));
        lines.extend(
            cleanup
                .warnings
                .iter()
                .map(|warning| format!("cleanup warning: {warning}")),
        );
    }
    lines.extend(
        response
            .warnings
            .iter()
            .map(|warning| format!("warning: {warning}")),
    );
    lines
}

fn worktree_merge_result_notice(response: &MergeWorktreeResponse) -> String {
    match response.status.as_str() {
        "merged" => {
            let cleanup_warning = response.cleanup.as_ref().is_some_and(|cleanup| !cleanup.warnings.is_empty())
                || !response.warnings.is_empty();
            if cleanup_warning {
                format!(
                    "Merged locally into {}, but cleanup reported warnings. Inspect the worktree result before further cleanup. This does not push; push {} separately when ready.",
                    response.target_branch, response.target_branch
                )
            } else {
                format!(
                    "Merged locally into {}. This does not push; {} may now be ahead of origin. Push {} separately.",
                    response.target_branch, response.target_branch, response.target_branch
                )
            }
        }
        "nothing_to_merge" => format!(
            "Nothing to merge into {}. No push was performed.",
            response.target_branch
        ),
        "conflict" => {
            match response.conflict.as_ref() {
                Some(conflict) if conflict.aborted && response.warnings.is_empty() => {
                    "Preflight conflict blocked the merge before changes were applied. Resolve the listed conflicts before retrying. No push was performed.".to_string()
                }
                Some(conflict) if conflict.aborted => {
                    "Merge conflict rollback reported warnings. Inspect the target workspace before retrying. No push was performed.".to_string()
                }
                Some(conflict) if conflict.merge_in_progress => {
                    "Merge conflict remains in progress in the target workspace. Resolve or abort it before retrying. No push was performed.".to_string()
                }
                Some(_) => {
                    "Merge conflict cleanup reported warnings. Inspect the target workspace before retrying. No push was performed.".to_string()
                }
                None => "Merge conflict returned without cleanup details. Inspect the target workspace before retrying. No push was performed.".to_string(),
            }
        }
        status => format!("Merge ended with status {status}. No push was performed."),
    }
}

fn worktree_merge_error_notice(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if lower.contains("uncommitted changes") || lower.contains("merge in progress") {
        format!("Merge preflight guard blocked the local merge: {error}. No push was performed.")
    } else if lower.contains("preflight") {
        format!("Merge preflight could not complete: {error}. Inspect the target workspace before retrying. No push was performed.")
    } else {
        format!("Failed to merge worktree: {error}. No push was performed.")
    }
}

fn worktree_state(status: &crate::client::WorktreeStatusResponse) -> &'static str {
    if status.path_exists == Some(false) {
        "missing"
    } else if status.conflicted == Some(true) {
        "conflicted"
    } else if status.dirty == Some(true) {
        "dirty"
    } else if status.is_git_worktree == Some(true) {
        "clean"
    } else {
        "unknown"
    }
}

fn append_optional_identity(lines: &mut Vec<String>, label: &str, value: &Option<String>) {
    if let Some(value) = value.as_deref().filter(|value| !value.is_empty()) {
        lines.push(format!("{label}: {value}"));
    }
}

fn unknown(value: &str) -> &str {
    if value.is_empty() {
        "unknown"
    } else {
        value
    }
}

fn optional(value: &Option<String>) -> &str {
    value
        .as_deref()
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown")
}

fn optional_count(value: Option<usize>) -> String {
    value.map_or_else(|| "unknown".to_string(), |value| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_confirmation_names_every_destructive_choice_and_separate_push() {
        let notice = worktree_merge_confirmation_notice(&WorktreeMergeConfirmation {
            id: "wt-1".to_string(),
            strategy: "squash".to_string(),
            target_branch: "main".to_string(),
            include_uncommitted: true,
            delete_after_merge: true,
        });

        assert!(notice.contains("squash"));
        assert!(notice.contains("main"));
        assert!(notice.contains("include and commit uncommitted files"));
        assert!(notice.contains("does not push"));
    }

    #[test]
    fn merge_failure_modes_are_actionable_and_keep_push_distinct() {
        for (response, expected) in [
            (
                MergeWorktreeResponse {
                    status: "nothing_to_merge".to_string(),
                    target_branch: "main".to_string(),
                    ..MergeWorktreeResponse::default()
                },
                "Nothing to merge",
            ),
            (
                MergeWorktreeResponse {
                    status: "conflict".to_string(),
                    conflict: Some(crate::client::WorktreeConflictStateResponse {
                        aborted: true,
                        ..Default::default()
                    }),
                    ..MergeWorktreeResponse::default()
                },
                "Preflight conflict",
            ),
            (
                MergeWorktreeResponse {
                    status: "conflict".to_string(),
                    conflict: Some(crate::client::WorktreeConflictStateResponse {
                        aborted: true,
                        ..Default::default()
                    }),
                    warnings: vec!["rollback warning".to_string()],
                    ..MergeWorktreeResponse::default()
                },
                "rollback reported warnings",
            ),
            (
                MergeWorktreeResponse {
                    status: "merged".to_string(),
                    target_branch: "main".to_string(),
                    cleanup: Some(crate::client::WorktreeRemovalResponse {
                        warnings: vec!["branch deletion failed".to_string()],
                        ..Default::default()
                    }),
                    ..MergeWorktreeResponse::default()
                },
                "cleanup reported warnings",
            ),
        ] {
            let notice = worktree_merge_result_notice(&MergeWorktreeResponse { ..response });
            assert!(notice.contains(expected), "{notice}");
            assert!(
                notice.contains("No push") || notice.contains("does not push"),
                "{notice}"
            );
        }
    }

    #[test]
    fn merge_preflight_guard_has_distinct_actionable_notice() {
        let notice = worktree_merge_error_notice("Target workspace has uncommitted changes");
        assert!(notice.contains("preflight guard"));
        assert!(notice.contains("No push"));
    }

    #[test]
    fn surface_gate_requires_explicit_truthy_value() {
        assert!(!worktree_surfaces_enabled_from(None));
        assert!(!worktree_surfaces_enabled_from(Some("0")));
        assert!(worktree_surfaces_enabled_from(Some("true")));
    }

    #[test]
    fn cleanup_command_parses_apply_only_as_the_first_token() {
        let cases = [
            (
                "cleanup",
                WorktreeAction::CleanupPlan {
                    request: WorktreeCleanupRequest::default(),
                },
            ),
            (
                "cleanup apply",
                WorktreeAction::Cleanup {
                    request: WorktreeCleanupRequest::default(),
                },
            ),
            (
                "cleanup wt-1 wt-2",
                WorktreeAction::CleanupPlan {
                    request: WorktreeCleanupRequest {
                        ids: vec!["wt-1".to_string(), "wt-2".to_string()],
                        ..WorktreeCleanupRequest::default()
                    },
                },
            ),
            (
                "cleanup apply wt-1 wt-2",
                WorktreeAction::Cleanup {
                    request: WorktreeCleanupRequest {
                        ids: vec!["wt-1".to_string(), "wt-2".to_string()],
                        ..WorktreeCleanupRequest::default()
                    },
                },
            ),
        ];

        for (command, action) in cases {
            let mut app = App::notice_only("test");
            assert_eq!(
                app.start_worktree_command_with_enabled(command, true),
                AppAction::Worktree { action }
            );
        }
    }

    #[test]
    fn aborted_conflict_with_warnings_does_not_claim_clean_rollback() {
        let notice = worktree_merge_result_notice(&MergeWorktreeResponse {
            status: "conflict".to_string(),
            conflict: Some(crate::client::WorktreeConflictStateResponse {
                aborted: true,
                ..Default::default()
            }),
            warnings: vec!["rollback warning".to_string()],
            ..MergeWorktreeResponse::default()
        });

        assert!(notice.contains("rollback reported warnings"));
        assert!(!notice.contains("cleanly"));
        assert!(notice.contains("No push"));
    }

    #[test]
    fn diff_pager_sanitizes_paths_and_patch_text() {
        let lines = worktree_diff_lines(&WorktreeDiffResponse {
            id: "wt-1".to_string(),
            files: vec![crate::client::WorktreeDiffFileResponse {
                status: "M".to_string(),
                path: "src/\x1b[31mname\0.rs".to_string(),
                ..Default::default()
            }],
            patch: "--- a/src/\x1b[31mname\0.rs\n+\x1b[2Jnew".to_string(),
            ..WorktreeDiffResponse::default()
        });

        assert!(lines.iter().any(|line| line == "M src/name .rs"));
        assert!(lines.iter().any(|line| line == "--- a/src/name .rs"));
        assert!(lines.iter().any(|line| line == "+new"));
        assert!(lines
            .iter()
            .all(|line| !line.chars().any(|character| character.is_control())));
    }

    #[test]
    fn merge_command_opens_explicit_confirmation_before_dispatch() {
        let mut app = App::notice_only("test");
        let action = app.start_worktree_command_with_enabled(
            "merge wt-1 squash main include-uncommitted",
            true,
        );

        assert_eq!(action, AppAction::None);
        let confirmation = app.pending_worktree_merge().unwrap();
        assert_eq!(confirmation.strategy, "squash");
        assert_eq!(confirmation.target_branch, "main");
        assert!(confirmation.include_uncommitted);
        assert!(confirmation.delete_after_merge);
        let lines = app.worktree_merge_confirmation_lines().unwrap();
        assert!(lines.iter().any(|line| line.contains("not a push")));
    }

    #[test]
    fn merge_command_can_keep_the_source_worktree() {
        let mut app = App::notice_only("test");
        app.start_worktree_command_with_enabled("merge wt-1 merge main keep-source", true);

        assert!(!app.pending_worktree_merge().unwrap().delete_after_merge);
    }

    #[test]
    fn absent_commit_distances_are_not_rendered_as_zero() {
        let lines = worktree_diff_lines(&WorktreeDiffResponse {
            id: "wt-1".to_string(),
            ..WorktreeDiffResponse::default()
        });
        assert!(!lines
            .iter()
            .any(|line| line.contains("ahead") || line.contains("behind")));
    }

    #[test]
    fn absent_worktree_counts_render_as_unknown() {
        let lines = worktree_diff_lines(&WorktreeDiffResponse {
            id: "wt-1".to_string(),
            ..WorktreeDiffResponse::default()
        });
        assert!(lines
            .iter()
            .any(|line| line.contains("changed: unknown files · +unknown -unknown")));
    }
}
