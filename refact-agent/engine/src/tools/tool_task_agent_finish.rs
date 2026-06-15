use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock};
use async_trait::async_trait;
use chrono::Utc;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::chat::verifier::{schedule_card_verifier_after_finish, ExpectedCardState};
use crate::tasks::storage;
use crate::tasks::types::{
    AbVariantFinish, AbVariantInfo, BoardCard, FinalReport, StatusUpdate, SuggestedCard,
    VerificationResult,
};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};
use crate::worktrees::types::WorktreeMeta;

async fn get_task_id(ccx: &Arc<AMutex<AtCommandsContext>>) -> Result<String, String> {
    let ccx_lock = ccx.lock().await;
    ccx_lock
        .task_meta
        .as_ref()
        .map(|m| m.task_id.clone())
        .ok_or_else(|| {
            "This tool can only be used by task agents (chat not bound to a task)".to_string()
        })
}

async fn get_card_id(ccx: &Arc<AMutex<AtCommandsContext>>) -> Result<String, String> {
    let ccx_lock = ccx.lock().await;
    ccx_lock
        .task_meta
        .as_ref()
        .and_then(|m| m.card_id.clone())
        .ok_or_else(|| {
            "This tool can only be used by task agents (no card_id in task_meta)".to_string()
        })
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolvedAgentWorktree {
    root: PathBuf,
    branch: Option<String>,
    name: Option<String>,
}

fn resolve_agent_worktree(
    thread_worktree: Option<WorktreeMeta>,
    card: &BoardCard,
) -> Option<ResolvedAgentWorktree> {
    if let Some(meta) = thread_worktree {
        return Some(ResolvedAgentWorktree {
            root: meta.root,
            branch: meta.branch,
            name: Some(meta.id),
        });
    }
    card.agent_worktree
        .as_ref()
        .map(|root| ResolvedAgentWorktree {
            root: PathBuf::from(root),
            branch: card.agent_branch.clone(),
            name: card.agent_worktree_name.clone(),
        })
}

fn resolve_ab_variant_worktree(
    thread_worktree: Option<WorktreeMeta>,
    variant: &AbVariantInfo,
) -> ResolvedAgentWorktree {
    if let Some(meta) = thread_worktree {
        return ResolvedAgentWorktree {
            root: meta.root,
            branch: meta.branch,
            name: Some(meta.id),
        };
    }
    ResolvedAgentWorktree {
        root: PathBuf::from(&variant.worktree),
        branch: variant.branch.clone(),
        name: variant.worktree_name.clone(),
    }
}

static FINISH_LOCKS: OnceLock<AMutex<HashMap<String, Arc<AMutex<()>>>>> = OnceLock::new();

fn get_finish_locks() -> &'static AMutex<HashMap<String, Arc<AMutex<()>>>> {
    FINISH_LOCKS.get_or_init(|| AMutex::new(HashMap::new()))
}

async fn get_finish_lock(task_id: &str, card_id: &str) -> Arc<AMutex<()>> {
    let mut locks = get_finish_locks().lock().await;
    locks
        .entry(format!("{}:{}", task_id, card_id))
        .or_insert_with(|| Arc::new(AMutex::new(())))
        .clone()
}

async fn ensure_lock_for<T>(
    task_id: &str,
    card_id: &str,
    finish: impl std::future::Future<Output = T>,
) -> T {
    let key = format!("{}:{}", task_id, card_id);
    let finish_lock = get_finish_lock(task_id, card_id).await;
    let result = {
        let _finish_guard = finish_lock.lock().await;
        finish.await
    };
    let mut locks = get_finish_locks().lock().await;
    if let Some(entry) = locks.get(&key) {
        if Arc::ptr_eq(entry, &finish_lock) && Arc::strong_count(&finish_lock) == 2 {
            locks.remove(&key);
        }
    }
    result
}

async fn refresh_finish_heartbeat_if_current(
    gcx: Arc<crate::global_context::GlobalContext>,
    task_id: &str,
    card_id: &str,
    finish_chat_id: &str,
    expected_agent_id: Option<&str>,
) -> Result<(), String> {
    let card_id_owned = card_id.to_string();
    let finish_chat_id_owned = finish_chat_id.to_string();
    let expected_agent_id_owned = expected_agent_id.map(str::to_string);
    let heartbeat = Utc::now().to_rfc3339();
    storage::update_board_atomic(gcx, task_id, move |board| {
        let card = board
            .get_card_mut(&card_id_owned)
            .ok_or_else(|| format!("Card {} not found", card_id_owned))?;
        if card.column != "doing" {
            return Err(format!(
                "Card {} is in '{}' column. Cannot finish from stale agent state.",
                card_id_owned, card.column
            ));
        }
        if let Some(variants) = card.ab_variants.as_ref() {
            let variant = if variants.a.chat_id == finish_chat_id_owned {
                &variants.a
            } else if variants.b.chat_id == finish_chat_id_owned {
                &variants.b
            } else {
                return Err(format!(
                    "Card {} has A/B variants {:?} and {:?}, not {}",
                    card_id_owned, variants.a.chat_id, variants.b.chat_id, finish_chat_id_owned
                ));
            };
            if variant.finish.is_some() {
                return Err(format!(
                    "A/B variant {} for card {} has already finished. Cannot finish twice.",
                    finish_chat_id_owned, card_id_owned
                ));
            }
            if let Some(expected_agent_id) = expected_agent_id_owned.as_deref() {
                if variant.agent_id != expected_agent_id {
                    return Err(format!(
                        "A/B variant {} for card {} belongs to agent {}, not {}",
                        finish_chat_id_owned, card_id_owned, variant.agent_id, expected_agent_id
                    ));
                }
            }
            card.last_heartbeat_at = Some(heartbeat.clone());
            return Ok(());
        }
        if card.agent_chat_id.as_deref() != Some(finish_chat_id_owned.as_str()) {
            return Err(format!(
                "Card {} is now owned by agent_chat_id={:?}, not {}",
                card_id_owned, card.agent_chat_id, finish_chat_id_owned
            ));
        }
        if let Some(expected_agent_id) = expected_agent_id_owned.as_deref() {
            if card.assignee.as_deref() != Some(expected_agent_id) {
                return Err(format!(
                    "Card {} is now assigned to {:?}, not {}",
                    card_id_owned, card.assignee, expected_agent_id
                ));
            }
        }
        card.last_heartbeat_at = Some(heartbeat.clone());
        Ok(())
    })
    .await
    .map(|_| ())
}

fn parse_success_arg(args: &HashMap<String, Value>) -> Result<bool, String> {
    match args.get("success") {
        Some(Value::Bool(b)) => Ok(*b),
        Some(Value::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err("Invalid 'success' string; expected true or false".to_string()),
        },
        _ => Err("Missing or invalid 'success' parameter (must be boolean)".to_string()),
    }
}

fn git_failure_details(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    match (stderr.is_empty(), stdout.is_empty()) {
        (false, false) => format!("{}\n{}", stderr, stdout),
        (false, true) => stderr,
        (true, false) => stdout,
        (true, true) => format!("exit status {}", output.status),
    }
}

async fn git_output_checked(
    worktree_path: &Path,
    args: &[&str],
    action: &str,
) -> Result<std::process::Output, String> {
    let output = tokio::process::Command::new("git")
        .args(args)
        .current_dir(worktree_path)
        .output()
        .await
        .map_err(|e| {
            format!(
                "Failed to run git {} in worktree '{}': {}",
                action,
                worktree_path.display(),
                e
            )
        })?;

    if !output.status.success() {
        return Err(format!(
            "git {} failed in worktree '{}': {}",
            action,
            worktree_path.display(),
            git_failure_details(&output)
        ));
    }

    Ok(output)
}

async fn validate_git_worktree(worktree_path: &Path) -> Result<(), String> {
    if !worktree_path.exists() {
        return Err(format!(
            "Assigned worktree path '{}' does not exist",
            worktree_path.display()
        ));
    }
    if !worktree_path.is_dir() {
        return Err(format!(
            "Assigned worktree path '{}' is not a directory",
            worktree_path.display()
        ));
    }

    let output = tokio::process::Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(worktree_path)
        .output()
        .await
        .map_err(|e| {
            format!(
                "Failed to validate git worktree '{}': {}",
                worktree_path.display(),
                e
            )
        })?;

    if !output.status.success() {
        return Err(format!(
            "Assigned worktree path '{}' is not a git worktree/repo: {}",
            worktree_path.display(),
            git_failure_details(&output)
        ));
    }

    let inside_work_tree = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if inside_work_tree != "true" {
        return Err(format!(
            "Assigned worktree path '{}' is not inside a git worktree",
            worktree_path.display()
        ));
    }

    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ParsedFinishReport {
    markdown: String,
    structured: Option<FinalReport>,
}

fn agent_finish_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "success": {
                "type": "boolean",
                "description": "true if the card was completed successfully, false if it failed"
            },
            "report": {
                "description": "Legacy markdown string or structured final report object",
                "anyOf": [
                    { "type": "string" },
                    final_report_schema()
                ]
            },
            "files_changed": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Relative paths changed by this card"
            },
            "tests_added": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Test names or paths added or updated"
            },
            "tests_added_or_updated": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Test names or paths added or updated"
            },
            "verification": {
                "type": "array",
                "items": verification_schema(),
                "description": "Verification commands and results"
            },
            "followup_cards": {
                "type": "array",
                "items": suggested_card_schema(),
                "description": "Suggested follow-up task cards"
            },
            "risks": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Risks or caveats"
            },
            "assumptions": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Assumptions made while completing the card"
            }
        },
        "required": ["success", "report"]
    })
}

fn final_report_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "summary": { "type": "string" },
            "success": { "type": "boolean" },
            "files_changed": { "type": "array", "items": { "type": "string" } },
            "tests_added_or_updated": { "type": "array", "items": { "type": "string" } },
            "verification": { "type": "array", "items": verification_schema() },
            "followup_cards": { "type": "array", "items": suggested_card_schema() },
            "risks": { "type": "array", "items": { "type": "string" } },
            "assumptions": { "type": "array", "items": { "type": "string" } }
        }
    })
}

fn verification_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "command": { "type": "string" },
            "exit_code": { "type": ["integer", "null"] },
            "passed": { "type": "boolean" },
            "output_tail": { "type": "string" }
        }
    })
}

fn suggested_card_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "title": { "type": "string" },
            "instructions": { "type": "string" },
            "priority": { "type": "string" },
            "target_files": { "type": "array", "items": { "type": "string" } }
        }
    })
}

fn parse_finish_report(
    args: &HashMap<String, Value>,
    success: bool,
) -> Result<ParsedFinishReport, String> {
    let report_value = args
        .get("report")
        .ok_or_else(|| "Missing 'report' parameter".to_string())?;

    match report_value {
        Value::String(report) => {
            if has_structured_report_fields(args) {
                let structured = structured_report_from_args(report.clone(), success, args)?;
                let markdown = structured.to_markdown();
                Ok(ParsedFinishReport {
                    markdown,
                    structured: Some(structured),
                })
            } else {
                Ok(ParsedFinishReport {
                    markdown: report.clone(),
                    structured: None,
                })
            }
        }
        Value::Object(_) => {
            let mut structured: FinalReport = serde_json::from_value(report_value.clone())
                .map_err(|e| format!("Invalid structured 'report' parameter: {}", e))?;
            structured.success = success;
            apply_optional_structured_fields(&mut structured, args)?;
            let markdown = structured.to_markdown();
            Ok(ParsedFinishReport {
                markdown,
                structured: Some(structured),
            })
        }
        _ => Err("Invalid 'report' parameter (must be string or object)".to_string()),
    }
}

fn has_structured_report_fields(args: &HashMap<String, Value>) -> bool {
    [
        "files_changed",
        "tests_added",
        "tests_added_or_updated",
        "verification",
        "followup_cards",
        "risks",
        "assumptions",
    ]
    .iter()
    .any(|key| args.contains_key(*key))
}

fn structured_report_from_args(
    summary: String,
    success: bool,
    args: &HashMap<String, Value>,
) -> Result<FinalReport, String> {
    let mut report = FinalReport {
        summary,
        success,
        ..Default::default()
    };
    apply_optional_structured_fields(&mut report, args)?;
    Ok(report)
}

fn apply_optional_structured_fields(
    report: &mut FinalReport,
    args: &HashMap<String, Value>,
) -> Result<(), String> {
    if let Some(value) = args.get("files_changed") {
        report.files_changed = parse_string_vec(value, "files_changed")?;
    }
    if let Some(value) = args
        .get("tests_added_or_updated")
        .or_else(|| args.get("tests_added"))
    {
        report.tests_added_or_updated = parse_string_vec(value, "tests_added_or_updated")?;
    }
    if let Some(value) = args.get("verification") {
        report.verification = parse_json_field::<Vec<VerificationResult>>(value, "verification")?;
    }
    if let Some(value) = args.get("followup_cards") {
        report.followup_cards = parse_json_field::<Vec<SuggestedCard>>(value, "followup_cards")?;
    }
    if let Some(value) = args.get("risks") {
        report.risks = parse_string_vec(value, "risks")?;
    }
    if let Some(value) = args.get("assumptions") {
        report.assumptions = parse_string_vec(value, "assumptions")?;
    }
    Ok(())
}

fn parse_string_vec(value: &Value, field: &str) -> Result<Vec<String>, String> {
    parse_json_field::<Vec<String>>(value, field)
}

fn parse_json_field<T>(value: &Value, field: &str) -> Result<T, String>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_value(value.clone())
        .map_err(|e| format!("Invalid '{}' parameter: {}", field, e))
}

fn mark_finished_card(
    card: &mut BoardCard,
    success: bool,
    report: &ParsedFinishReport,
    commit_hash: Option<&str>,
) {
    if success {
        card.final_report = Some(report.markdown.clone());
        card.final_report_structured = report.structured.clone();
        card.column = "done".to_string();
        card.completed_at = Some(Utc::now().to_rfc3339());
        if let Some(hash) = commit_hash {
            card.status_updates.push(StatusUpdate {
                timestamp: Utc::now().to_rfc3339(),
                message: format!("Auto-committed: {}", hash),
            });
        }
        card.status_updates.push(StatusUpdate {
            timestamp: Utc::now().to_rfc3339(),
            message: "Agent completed successfully".to_string(),
        });
    } else {
        card.final_report = Some(format!("FAILED: {}", report.markdown));
        card.final_report_structured = report.structured.clone();
        card.column = "failed".to_string();
        card.completed_at = Some(Utc::now().to_rfc3339());
        card.status_updates.push(StatusUpdate {
            timestamp: Utc::now().to_rfc3339(),
            message: format!("Agent failed: {}", report.markdown),
        });
    }
}

fn mark_ab_variant_finished(
    card: &mut BoardCard,
    variant_key: &str,
    finish_chat_id: &str,
    expected_agent_id: Option<&str>,
    success: bool,
    report: &ParsedFinishReport,
    commit_hash: Option<&str>,
) -> Result<bool, String> {
    if card.column != "doing" {
        return Err(format!(
            "Card {} is in '{}' column. Cannot finish from stale agent state.",
            card.id, card.column
        ));
    }
    let variant_label = variant_key.to_ascii_uppercase();
    let all_finished = {
        let variants = card
            .ab_variants
            .as_mut()
            .ok_or_else(|| format!("Card {} has no A/B variants", card.id))?;
        let variant = variants
            .variant_mut(variant_key)
            .ok_or_else(|| format!("A/B variant {} not found", variant_key))?;
        if variant.chat_id != finish_chat_id {
            return Err(format!(
                "A/B variant {} for card {} is chat {}, not {}",
                variant_label, card.id, variant.chat_id, finish_chat_id
            ));
        }
        if variant.finish.is_some() {
            return Err(format!(
                "A/B variant {} for card {} has already finished. Cannot finish twice.",
                finish_chat_id, card.id
            ));
        }
        if let Some(expected_agent_id) = expected_agent_id {
            if variant.agent_id != expected_agent_id {
                return Err(format!(
                    "A/B variant {} for card {} belongs to agent {}, not {}",
                    finish_chat_id, card.id, variant.agent_id, expected_agent_id
                ));
            }
        }
        variant.finish = Some(AbVariantFinish {
            success,
            final_report: report.markdown.clone(),
            final_report_structured: report.structured.clone(),
            completed_at: Utc::now().to_rfc3339(),
            commit_hash: commit_hash.map(str::to_string),
        });
        variants.all_finished()
    };
    if let Some(hash) = commit_hash {
        card.status_updates.push(StatusUpdate {
            timestamp: Utc::now().to_rfc3339(),
            message: format!("A/B variant {} auto-committed: {}", variant_label, hash),
        });
    }
    card.status_updates.push(StatusUpdate {
        timestamp: Utc::now().to_rfc3339(),
        message: format!(
            "A/B variant {} {}",
            variant_label,
            if success {
                "completed successfully"
            } else {
                "failed"
            }
        ),
    });
    Ok(all_finished)
}

fn clear_finished_agent_session(card: &mut BoardCard) {
    card.agent_chat_id = None;
    card.assignee = None;
}

fn clear_finished_agent_session_if_current(
    card: &mut BoardCard,
    expected_state: &ExpectedCardState,
    board_rev: u64,
) -> Result<(), String> {
    if !expected_state.matches_board_card(board_rev, card) {
        return Err("stale finish cleanup".to_string());
    }
    clear_finished_agent_session(card);
    Ok(())
}

fn agents_active(cards: &[BoardCard]) -> usize {
    cards
        .iter()
        .filter(|c| c.column == "doing" && c.agent_chat_id.is_some())
        .count()
}

fn sanitize_commit_component(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    let mut previous_space = false;

    for ch in text.chars() {
        if out.chars().count() >= max_chars {
            break;
        }
        let ch = if ch.is_control() { ' ' } else { ch };
        if ch.is_whitespace() {
            if !previous_space && !out.is_empty() {
                out.push(' ');
                previous_space = true;
            }
        } else {
            out.push(ch);
            previous_space = false;
        }
    }

    out.trim().to_string()
}

fn deterministic_agent_commit_message(card_id: &str, card_title: &str) -> String {
    let card_id = sanitize_commit_component(card_id, 80);
    let title = sanitize_commit_component(card_title, 160);
    let card_id = if card_id.is_empty() {
        "unknown".to_string()
    } else {
        card_id
    };

    if title.is_empty() {
        format!("Card {}", card_id)
    } else {
        format!("Card {}: {}", card_id, title)
    }
}

async fn auto_commit_worktree(
    worktree_path: &Path,
    card_id: &str,
    card_title: &str,
) -> Result<Option<String>, String> {
    auto_commit_worktree_with_message(worktree_path, card_id, card_title, None).await
}

async fn auto_commit_worktree_with_message(
    worktree_path: &Path,
    card_id: &str,
    card_title: &str,
    commit_msg_override: Option<String>,
) -> Result<Option<String>, String> {
    validate_git_worktree(worktree_path).await?;

    let status_output =
        git_output_checked(worktree_path, &["status", "--porcelain"], "status").await?;

    let status = String::from_utf8_lossy(&status_output.stdout);
    if status.trim().is_empty() {
        return Ok(None);
    }

    git_output_checked(worktree_path, &["add", "-A"], "add").await?;

    let commit_msg = commit_msg_override
        .as_deref()
        .map(|msg| sanitize_commit_component(msg, 240))
        .filter(|msg| !msg.is_empty())
        .unwrap_or_else(|| deterministic_agent_commit_message(card_id, card_title));

    let commit_output = tokio::process::Command::new("git")
        .args([
            "-c",
            "user.name=Refact Agent",
            "-c",
            "user.email=agent@refact.ai",
            "commit",
            "-m",
            &commit_msg,
            "--no-gpg-sign",
        ])
        .current_dir(worktree_path)
        .output()
        .await
        .map_err(|e| {
            format!(
                "Failed to commit in worktree '{}': {}",
                worktree_path.display(),
                e
            )
        })?;

    if !commit_output.status.success() {
        let stderr = String::from_utf8_lossy(&commit_output.stderr);
        if stderr.contains("nothing to commit") {
            return Ok(None);
        }
        return Err(format!(
            "git commit failed in worktree '{}': {}",
            worktree_path.display(),
            git_failure_details(&commit_output)
        ));
    }

    let rev_output =
        git_output_checked(worktree_path, &["rev-parse", "HEAD"], "rev-parse HEAD").await?;

    let commit_hash = String::from_utf8_lossy(&rev_output.stdout)
        .trim()
        .to_string();
    if commit_hash.is_empty() {
        return Err(format!(
            "git rev-parse HEAD returned empty output in worktree '{}'",
            worktree_path.display()
        ));
    }
    Ok(Some(commit_hash))
}

pub struct ToolTaskAgentFinish;

impl ToolTaskAgentFinish {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for ToolTaskAgentFinish {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "agent_finish".to_string(),
            display_name: "Task Agent Finish".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: String::new(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Mark the current card as completed or failed. Task agents MUST call this exactly once when finished. This updates the task board and notifies the planner.".to_string(),
            input_schema: agent_finish_schema(),
            output_schema: None,
            annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let task_id = get_task_id(&ccx).await?;
        let card_id = get_card_id(&ccx).await?;
        let (planner_chat_id, finish_chat_id, finish_agent_id) = {
            let ccx_lock = ccx.lock().await;
            (
                ccx_lock
                    .task_meta
                    .as_ref()
                    .and_then(|meta| meta.planner_chat_id.clone()),
                ccx_lock.chat_id.clone(),
                ccx_lock
                    .task_meta
                    .as_ref()
                    .and_then(|meta| meta.agent_id.clone()),
            )
        };

        let success = parse_success_arg(args)?;

        let report = parse_finish_report(args, success)?;

        let gcx = {
            let ccx_lock = ccx.lock().await;
            ccx_lock.app.gcx.clone()
        };
        let lock_task_id = task_id.clone();
        let lock_card_id = card_id.clone();

        ensure_lock_for(&lock_task_id, &lock_card_id, async move {

        refresh_finish_heartbeat_if_current(
            gcx.clone(),
            &task_id,
            &card_id,
            &finish_chat_id,
            finish_agent_id.as_deref(),
        )
        .await?;

        let board_pre = storage::load_board(gcx.clone(), &task_id).await?;
        let card_pre = board_pre
            .get_card(&card_id)
            .ok_or(format!("Card {} not found", card_id))?;
        if card_pre.column == "done" || card_pre.column == "failed" {
            return Err(format!(
                "Card {} is already in '{}' column. Cannot finish twice.",
                card_id, card_pre.column
            ));
        }
        let thread_worktree = ccx.lock().await.execution_scope_worktree();
        let ab_variant_key = if let Some(variants) = card_pre.ab_variants.as_ref() {
            let variant_key = variants
                .variant_key_for_chat_id(&finish_chat_id)
                .ok_or_else(|| {
                    format!(
                        "Card {} has A/B variants {:?} and {:?}, not {}",
                        card_id, variants.a.chat_id, variants.b.chat_id, finish_chat_id
                    )
                })?;
            let variant = variants
                .variant(variant_key)
                .ok_or_else(|| format!("A/B variant {} not found", variant_key))?;
            if variant.finish.is_some() {
                return Err(format!(
                    "A/B variant {} for card {} has already finished. Cannot finish twice.",
                    finish_chat_id, card_id
                ));
            }
            if let Some(expected_agent_id) = finish_agent_id.as_deref() {
                if variant.agent_id != expected_agent_id {
                    return Err(format!(
                        "A/B variant {} for card {} belongs to agent {}, not {}",
                        finish_chat_id, card_id, variant.agent_id, expected_agent_id
                    ));
                }
            }
            Some(variant_key.to_string())
        } else {
            None
        };
        let resolved_worktree = if let Some(variant_key) = ab_variant_key.as_deref() {
            let variants = card_pre.ab_variants.as_ref().expect("variant key requires A/B");
            let variant = variants
                .variant(variant_key)
                .ok_or_else(|| format!("A/B variant {} not found", variant_key))?;
            Some(resolve_ab_variant_worktree(thread_worktree, variant))
        } else {
            resolve_agent_worktree(thread_worktree, card_pre)
        };
        let card_title_for_commit = card_pre.title.clone();

        let commit_result = if success {
            if let Some(ref worktree) = resolved_worktree {
                match auto_commit_worktree(&worktree.root, &card_id, &card_title_for_commit).await
                {
                    Ok(hash) => hash,
                    Err(e) => {
                        return Err(format!(
                            "Auto-commit failed in worktree '{}': {}. Please ensure your changes are committed before calling agent_finish(success=true). \
                            You can run `git add -A && git commit -m 'your message'` in the worktree, or investigate the error.",
                            worktree.root.display(),
                            e
                        ));
                    }
                }
            } else {
                None
            }
        } else {
            None
        };

        let card_id_owned = card_id.clone();
        let report_clone = report.clone();
        let success_clone = success;
        let commit_hash = commit_result.clone();

        let finish_agent_id_for_update = finish_agent_id.clone();
        let ab_variant_key_for_update = ab_variant_key.clone();
        let (board, (card_title, all_finished, verifier_expected_state, finished_ab_variant)) =
            storage::update_board_atomic(gcx.clone(), &task_id, move |board| {
                let next_board_rev = board.rev + 1;
                let card = board
                    .get_card_mut(&card_id_owned)
                    .ok_or(format!("Card {} not found in task", card_id_owned))?;

                if let Some(variant_key) = ab_variant_key_for_update.as_deref() {
                    let card_title = card.title.clone();
                    let all_finished = mark_ab_variant_finished(
                        card,
                        variant_key,
                        &finish_chat_id,
                        finish_agent_id_for_update.as_deref(),
                        success_clone,
                        &report_clone,
                        commit_hash.as_deref(),
                    )?;
                    return Ok((
                        card_title,
                        all_finished,
                        None,
                        Some(variant_key.to_string()),
                    ));
                }

                if card.column == "done" || card.column == "failed" {
                    return Err(format!(
                        "Card {} is already in '{}' column. Cannot finish twice.",
                        card_id_owned, card.column
                    ));
                }

                if card.agent_chat_id.as_deref() != Some(finish_chat_id.as_str()) {
                    return Err(format!(
                        "Card {} is now owned by agent_chat_id={:?}, not {}",
                        card_id_owned, card.agent_chat_id, finish_chat_id
                    ));
                }
                if let Some(expected_agent_id) = finish_agent_id_for_update.as_deref() {
                    if card.assignee.as_deref() != Some(expected_agent_id) {
                        return Err(format!(
                            "Card {} is now assigned to {:?}, not {}",
                            card_id_owned, card.assignee, expected_agent_id
                        ));
                    }
                }

                let card_title = card.title.clone();

                mark_finished_card(card, success_clone, &report_clone, commit_hash.as_deref());
                let verifier_expected_state = ExpectedCardState::from_card(next_board_rev, card);

                let agents_active = agents_active(&board.cards);
                let all_finished = agents_active == 0;

                Ok((card_title, all_finished, Some(verifier_expected_state), None))
            })
            .await?;

        storage::update_task_stats(gcx.clone(), &task_id).await?;

        let result_message = if let Some(variant_key) = finished_ab_variant.as_deref() {
            let variant_label = variant_key.to_ascii_uppercase();
            if success {
                if all_finished {
                    format!(
                        "✅ **A/B Variant {} Completed: {}**\n\n**Report:**\n{}\n\nAll A/B variants have finished. Planner notified; use `pick_ab_winner` to promote the selected variant.",
                        variant_label, card_title, report.markdown
                    )
                } else {
                    format!(
                        "✅ **A/B Variant {} Completed: {}**\n\n**Report:**\n{}\n\nPlanner notified. The other A/B variant is still running.",
                        variant_label, card_title, report.markdown
                    )
                }
            } else if all_finished {
                format!(
                    "❌ **A/B Variant {} Failed: {}**\n\n**Reason:**\n{}\n\nAll A/B variants have finished. Planner notified; use `pick_ab_winner` to promote the selected variant outcome.",
                    variant_label, card_title, report.markdown
                )
            } else {
                format!(
                    "❌ **A/B Variant {} Failed: {}**\n\n**Reason:**\n{}\n\nPlanner notified. The other A/B variant is still running.",
                    variant_label, card_title, report.markdown
                )
            }
        } else if success {
            if all_finished {
                format!(
                    "✅ **Card Completed: {}**\n\n**Report:**\n{}\n\nAll agents have completed. Planner notified.",
                    card_title, report.markdown
                )
            } else {
                format!(
                    "✅ **Card Completed: {}**\n\n**Report:**\n{}\n\nPlanner notified. Other agents are still running.",
                    card_title, report.markdown
                )
            }
        } else {
            if all_finished {
                format!(
                    "❌ **Card Failed: {}**\n\n**Reason:**\n{}\n\nAll agents have completed. Planner notified.",
                    card_title, report.markdown
                )
            } else {
                format!(
                    "❌ **Card Failed: {}**\n\n**Reason:**\n{}\n\nPlanner notified. Other agents are still running.",
                    card_title, report.markdown
                )
            }
        };

        tracing::info!(
            "Agent finished card {} ({}): {}",
            card_id,
            if success { "success" } else { "failed" },
            report.markdown.chars().take(100).collect::<String>()
        );

        let notify_error = crate::chat::task_agent_monitor::notify_planner_agents_finished(
            crate::app_state::AppState::from_gcx(gcx.clone()).await,
            &task_id,
            &board,
            all_finished,
            planner_chat_id.as_deref(),
        )
        .await
        .err();
        if let Some(ref error) = notify_error {
            tracing::warn!(
                "Agent finished card {}, but planner notification failed: {}",
                card_id,
                error
            );
        }

        if success {
            if let Some(verifier_expected_state) = verifier_expected_state {
                schedule_card_verifier_after_finish(
                    gcx.clone(),
                    task_id.clone(),
                    card_id.clone(),
                    verifier_expected_state,
                )
                .await;
            }
        } else if let Some(expected_state) = verifier_expected_state {
            let card_id_clear = card_id.clone();
            let _ = storage::update_board_atomic(gcx.clone(), &task_id, move |board| {
                let board_rev = board.rev;
                if let Some(c) = board.get_card_mut(&card_id_clear) {
                    clear_finished_agent_session_if_current(c, &expected_state, board_rev)?;
                }
                Ok(())
            })
            .await;
        }

        {
            let ccx_lock = ccx.lock().await;
            ccx_lock.abort_flag.store(true, Ordering::SeqCst);
        }

        let result_message = if let Some(error) = notify_error {
            format!(
                "{}\n\n⚠️ Planner notification failed: {}",
                result_message, error
            )
        } else {
            result_message
        };

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(result_message),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            })],
        ))
        })
        .await
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::types::{
        AbVariantInfo, AbVariants, BoardCard, TaskBoard, TaskMeta as StoredTaskMeta, TaskStatus,
    };

    fn run_git(cwd: &Path, args: &[&str]) -> String {
        let output = std::process::Command::new("git")
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

    fn init_repo(root: &Path) {
        run_git(root, &["init"]);
        run_git(root, &["checkout", "-B", "main"]);
        run_git(root, &["config", "user.email", "test@example.com"]);
        run_git(root, &["config", "user.name", "Test User"]);
        std::fs::write(root.join("file.txt"), "hello\n").unwrap();
        run_git(root, &["add", "file.txt"]);
        run_git(root, &["commit", "-m", "initial"]);
    }

    fn test_card(worktree: Option<String>) -> BoardCard {
        BoardCard {
            id: "T-1".to_string(),
            title: "Card T-1".to_string(),
            column: "doing".to_string(),
            priority: "P1".to_string(),
            depends_on: vec![],
            instructions: String::new(),
            assignee: Some("agent-1".to_string()),
            agent_chat_id: Some("agent-chat-1".to_string()),
            status_updates: vec![],
            comments: vec![],
            final_report: None,
            final_report_structured: None,
            verifier_report: None,
            created_at: Utc::now().to_rfc3339(),
            started_at: Some(Utc::now().to_rfc3339()),
            last_heartbeat_at: None,
            completed_at: None,
            agent_branch: Some("legacy-branch".to_string()),
            agent_worktree: worktree,
            agent_worktree_name: Some("legacy-id".to_string()),
            ab_variants: None,
            team_members: vec![],
            target_files: vec![],
            scope_guard_mode: Default::default(),
        }
    }

    fn ab_variant(key: &str) -> AbVariantInfo {
        AbVariantInfo {
            agent_id: format!("agent-{}", key),
            chat_id: format!("agent-chat-{}", key),
            worktree: format!("/tmp/agent-{}", key),
            worktree_name: Some(format!("worktree-{}", key)),
            branch: Some(format!("branch-{}", key)),
            model: Some(format!("model-{}", key)),
            finish: None,
        }
    }

    fn ab_card() -> BoardCard {
        let mut card = test_card(None);
        card.assignee = Some("ab".to_string());
        card.agent_chat_id = None;
        card.agent_branch = None;
        card.agent_worktree = None;
        card.agent_worktree_name = None;
        card.ab_variants = Some(AbVariants {
            a: ab_variant("a"),
            b: ab_variant("b"),
            winner: None,
        });
        card
    }

    fn parsed_report(summary: &str, success: bool) -> ParsedFinishReport {
        let structured = FinalReport {
            summary: summary.to_string(),
            success,
            ..Default::default()
        };
        ParsedFinishReport {
            markdown: structured.to_markdown(),
            structured: Some(structured),
        }
    }

    fn sample_worktree_meta(temp: &Path) -> WorktreeMeta {
        let root = temp.join("thread-worktree");
        let source = temp.join("source");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&source).unwrap();
        WorktreeMeta {
            id: "thread-id".to_string(),
            kind: "task_agent".to_string(),
            root,
            source_workspace_root: source.clone(),
            repo_root: source,
            branch: Some("thread-branch".to_string()),
            base_branch: Some("main".to_string()),
            base_commit: Some("base".to_string()),
            task_id: Some("task-1".to_string()),
            card_id: Some("T-1".to_string()),
            agent_id: Some("agent-1".to_string()),
            enforce: true,
        }
    }

    async fn write_task(
        root: &Path,
        gcx: Arc<crate::global_context::GlobalContext>,
        card: BoardCard,
    ) {
        let task_dir = root.join(".refact").join("tasks").join("task-1");
        tokio::fs::create_dir_all(&task_dir).await.unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.to_path_buf()];
        let now = Utc::now().to_rfc3339();
        let meta = StoredTaskMeta {
            schema_version: 1,
            id: "task-1".to_string(),
            name: "Task".to_string(),
            status: TaskStatus::Active,
            created_at: now.clone(),
            updated_at: now,
            cards_total: 1,
            cards_done: 0,
            cards_failed: 0,
            agents_active: 1,
            base_branch: Some("main".to_string()),
            base_commit: None,
            default_agent_model: None,
            is_name_generated: false,
            last_agents_summary_at: None,
            planner_session_state: None,
        };
        storage::save_task_meta(gcx.clone(), "task-1", &meta)
            .await
            .unwrap();
        storage::save_board(
            gcx,
            "task-1",
            &TaskBoard {
                cards: vec![card],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn finish_locks_pruned_after_completion() {
        let key = "task-1:T-1".to_string();

        ensure_lock_for("task-1", "T-1", async {
            assert!(get_finish_locks().lock().await.contains_key(&key));
        })
        .await;

        assert!(!get_finish_locks().lock().await.contains_key(&key));
    }

    #[tokio::test]
    async fn finish_lock_not_pruned_while_external_holder_exists() {
        let task_id = "race-task";
        let card_id = "T-Race";
        let key = format!("{}:{}", task_id, card_id);

        let external = get_finish_lock(task_id, card_id).await;

        ensure_lock_for(task_id, card_id, async {}).await;

        assert!(
            get_finish_locks().lock().await.contains_key(&key),
            "lock entry was pruned while another caller still held an Arc; \
             a concurrent waiter would have proceeded against a stale lock \
             while a new caller created a fresh one and ran in parallel"
        );

        drop(external);
        ensure_lock_for(task_id, card_id, async {}).await;

        assert!(
            !get_finish_locks().lock().await.contains_key(&key),
            "lock entry must be pruned once no external holder remains"
        );
    }

    #[test]
    fn agents_active_uses_chat_id_not_assignee() {
        let mut card = test_card(None);
        card.agent_chat_id = None;

        assert_eq!(agents_active(&[card]), 0);
    }

    #[tokio::test]
    async fn finish_heartbeat_rejects_restarted_card_owner() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut card = test_card(None);
        card.assignee = Some("agent-new".to_string());
        card.agent_chat_id = Some("agent-chat-new".to_string());
        write_task(temp.path(), gcx.clone(), card).await;

        let error = refresh_finish_heartbeat_if_current(
            gcx.clone(),
            "task-1",
            "T-1",
            "agent-chat-1",
            Some("agent-1"),
        )
        .await
        .unwrap_err();

        assert!(error.contains("agent-chat-new"), "{error}");
        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let card = board.get_card("T-1").unwrap();
        assert!(card.last_heartbeat_at.is_none());
        assert_eq!(card.agent_chat_id.as_deref(), Some("agent-chat-new"));
    }

    #[tokio::test]
    async fn finish_heartbeat_accepts_current_owner() {
        let temp = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        write_task(temp.path(), gcx.clone(), test_card(None)).await;

        refresh_finish_heartbeat_if_current(
            gcx.clone(),
            "task-1",
            "T-1",
            "agent-chat-1",
            Some("agent-1"),
        )
        .await
        .unwrap();

        let board = storage::load_board(gcx, "task-1").await.unwrap();
        let card = board.get_card("T-1").unwrap();
        assert!(card.last_heartbeat_at.is_some());
    }

    #[test]
    fn spawn_agent_finish_prefers_thread_worktree_over_board_mirror() {
        let temp = tempfile::tempdir().unwrap();
        let meta = sample_worktree_meta(temp.path());
        let legacy_root = temp.path().join("legacy-root");
        let card = test_card(Some(legacy_root.to_string_lossy().to_string()));

        let resolved = resolve_agent_worktree(Some(meta.clone()), &card).unwrap();
        assert_eq!(resolved.root, meta.root);
        assert_eq!(resolved.branch.as_deref(), Some("thread-branch"));
        assert_eq!(resolved.name.as_deref(), Some("thread-id"));

        let legacy = resolve_agent_worktree(None, &card).unwrap();
        assert_eq!(legacy.root, legacy_root);
        assert_eq!(legacy.branch.as_deref(), Some("legacy-branch"));
        assert_eq!(legacy.name.as_deref(), Some("legacy-id"));
    }

    #[test]
    fn spawn_agent_finish_failure_retains_worktree_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let worktree = temp
            .path()
            .join("retained-worktree")
            .to_string_lossy()
            .to_string();
        let mut card = test_card(Some(worktree.clone()));
        let branch = card.agent_branch.clone();
        let name = card.agent_worktree_name.clone();
        let report = ParsedFinishReport {
            markdown: "agent failed".to_string(),
            structured: None,
        };

        mark_finished_card(&mut card, false, &report, None);
        clear_finished_agent_session(&mut card);

        assert_eq!(card.column, "failed");
        assert!(card.assignee.is_none());
        assert!(card.agent_chat_id.is_none());
        assert_eq!(card.agent_worktree.as_deref(), Some(worktree.as_str()));
        assert_eq!(card.agent_branch, branch);
        assert_eq!(card.agent_worktree_name, name);
    }

    #[test]
    fn finish_failure_cleanup_preserves_restarted_owner() {
        let mut card = test_card(None);
        let report = ParsedFinishReport {
            markdown: "agent failed".to_string(),
            structured: None,
        };
        mark_finished_card(&mut card, false, &report, None);
        let expected = ExpectedCardState::from_card(1, &card);
        card.column = "doing".to_string();
        card.assignee = Some("agent-2".to_string());
        card.agent_chat_id = Some("agent-chat-2".to_string());

        let error = clear_finished_agent_session_if_current(&mut card, &expected, 2).unwrap_err();

        assert_eq!(error, "stale finish cleanup");
        assert_eq!(card.assignee.as_deref(), Some("agent-2"));
        assert_eq!(card.agent_chat_id.as_deref(), Some("agent-chat-2"));
    }

    #[test]
    fn spawn_ab_first_variant_finish_does_not_terminalize_shared_card() {
        let mut card = ab_card();
        let report = parsed_report("variant A done", true);

        let all_finished = mark_ab_variant_finished(
            &mut card,
            "a",
            "agent-chat-a",
            Some("agent-a"),
            true,
            &report,
            Some("abc123"),
        )
        .unwrap();

        assert!(!all_finished);
        assert_eq!(card.column, "doing");
        assert!(card.final_report.is_none());
        assert!(card.final_report_structured.is_none());
        assert!(card.completed_at.is_none());
        let variants = card.ab_variants.as_ref().unwrap();
        let finish = variants.a.finish.as_ref().unwrap();
        assert!(finish.success);
        assert_eq!(finish.final_report, report.markdown);
        assert_eq!(finish.commit_hash.as_deref(), Some("abc123"));
        assert!(variants.b.finish.is_none());
    }

    #[test]
    fn spawn_ab_both_variant_finishes_persist_separately() {
        let mut card = ab_card();
        let report_a = parsed_report("variant A done", true);
        let report_b = parsed_report("variant B failed", false);

        assert!(!mark_ab_variant_finished(
            &mut card,
            "a",
            "agent-chat-a",
            Some("agent-a"),
            true,
            &report_a,
            None,
        )
        .unwrap());
        assert!(mark_ab_variant_finished(
            &mut card,
            "b",
            "agent-chat-b",
            Some("agent-b"),
            false,
            &report_b,
            None,
        )
        .unwrap());

        assert_eq!(card.column, "doing");
        assert!(card.final_report.is_none());
        let variants = card.ab_variants.as_ref().unwrap();
        assert_eq!(
            variants.a.finish.as_ref().unwrap().final_report,
            report_a.markdown
        );
        assert_eq!(
            variants.b.finish.as_ref().unwrap().final_report,
            report_b.markdown
        );
        assert!(variants.a.finish.as_ref().unwrap().success);
        assert!(!variants.b.finish.as_ref().unwrap().success);
    }

    #[test]
    fn spawn_ab_variant_finish_rejects_second_finish() {
        let mut card = ab_card();
        let report = parsed_report("variant A done", true);

        mark_ab_variant_finished(
            &mut card,
            "a",
            "agent-chat-a",
            Some("agent-a"),
            true,
            &report,
            None,
        )
        .unwrap();
        let error = mark_ab_variant_finished(
            &mut card,
            "a",
            "agent-chat-a",
            Some("agent-a"),
            true,
            &report,
            None,
        )
        .unwrap_err();

        assert!(error.contains("already finished"), "{error}");
    }

    #[test]
    fn deterministic_agent_commit_message_is_bounded_single_line() {
        let long_title = format!("  Fix   frog\n{}  ", "x".repeat(300));

        let message = deterministic_agent_commit_message("T-1", &long_title);

        assert!(message.starts_with("Card T-1: Fix frog "));
        assert!(!message.contains('\n'));
        assert!(message.chars().count() <= "Card T-1: ".chars().count() + 160);
    }

    #[test]
    fn deterministic_agent_commit_message_sanitizes_control_chars_in_id_and_title() {
        let message = deterministic_agent_commit_message("T-1\n\0\x1b[31m", "Fix\x07 title");

        assert_eq!(message, "Card T-1 [31m: Fix title");
        assert!(!message.chars().any(|ch| ch.is_control()));
    }

    #[test]
    fn sanitize_commit_component_bounds_override_messages() {
        let message = sanitize_commit_component(&format!("Fix\n{}", "x".repeat(300)), 12);

        assert_eq!(message, "Fix xxxxxxxx");
        assert!(!message.chars().any(|ch| ch.is_control()));
    }

    #[test]
    fn tool_agent_finish_structured_object_populates_both_report_shapes() {
        let args = HashMap::from_iter([
            ("success".to_string(), json!(true)),
            (
                "report".to_string(),
                json!({
                    "summary": "Implemented structured finish reports.",
                    "success": false,
                    "files_changed": ["refact-agent/engine/src/tools/tool_agent_finish.rs"],
                    "tests_added_or_updated": ["tool_agent_finish_structured_object_populates_both_report_shapes"],
                    "verification": [{
                        "command": "cargo test --lib -p refact-lsp -- tool_agent_finish",
                        "exit_code": 0,
                        "passed": true,
                        "output_tail": "ok"
                    }],
                    "followup_cards": [{
                        "title": "GUI structured report rendering",
                        "instructions": "Render final_report_structured when present.",
                        "priority": "P2",
                        "target_files": ["refact-agent/gui/src/features/Tasks"]
                    }],
                    "risks": ["Planner still reads legacy markdown."],
                    "assumptions": ["Markdown fallback remains populated."]
                }),
            ),
        ]);
        let parsed = parse_finish_report(&args, true).unwrap();
        let mut card = test_card(None);

        mark_finished_card(&mut card, true, &parsed, None);

        let structured = card.final_report_structured.unwrap();
        assert!(structured.success);
        assert_eq!(structured.summary, "Implemented structured finish reports.");
        assert_eq!(
            structured.files_changed,
            vec!["refact-agent/engine/src/tools/tool_agent_finish.rs"]
        );
        let markdown = card.final_report.unwrap();
        assert!(markdown.contains("## Summary\nImplemented structured finish reports."));
        assert!(markdown.contains("## Files Changed"));
        assert!(markdown.contains("## Tests Added or Updated"));
        assert!(markdown.contains("## Verification"));
        assert!(markdown.contains("## Follow-up Cards"));
        assert!(markdown.contains("## Risks"));
        assert!(markdown.contains("## Assumptions"));
    }

    #[test]
    fn tool_agent_finish_string_with_optional_fields_builds_structured_report() {
        let args = HashMap::from_iter([
            ("success".to_string(), json!(true)),
            ("report".to_string(), json!("Summary from legacy field")),
            ("files_changed".to_string(), json!(["src/lib.rs"])),
            ("tests_added".to_string(), json!(["unit test"])),
            (
                "verification".to_string(),
                json!([{
                    "command": "cargo test",
                    "exit_code": 0,
                    "passed": true,
                    "output_tail": "ok"
                }]),
            ),
        ]);

        let parsed = parse_finish_report(&args, true).unwrap();
        let structured = parsed.structured.unwrap();

        assert_eq!(structured.summary, "Summary from legacy field");
        assert_eq!(structured.files_changed, vec!["src/lib.rs"]);
        assert_eq!(structured.tests_added_or_updated, vec!["unit test"]);
        assert!(parsed.markdown.contains("## Verification"));
    }

    #[test]
    fn agent_finish_rejects_invalid_success_string() {
        let tru_args = HashMap::from_iter([
            ("success".to_string(), json!("tru")),
            ("report".to_string(), json!("invalid success")),
        ]);
        let yes_args = HashMap::from_iter([
            ("success".to_string(), json!("yes")),
            ("report".to_string(), json!("invalid success")),
        ]);

        let tru_error = parse_success_arg(&tru_args).unwrap_err();
        let yes_error = parse_success_arg(&yes_args).unwrap_err();

        assert_eq!(
            tru_error,
            "Invalid 'success' string; expected true or false"
        );
        assert_eq!(
            yes_error,
            "Invalid 'success' string; expected true or false"
        );
    }

    #[test]
    fn agent_finish_accepts_trimmed_boolean_strings() {
        let true_args = HashMap::from_iter([
            ("success".to_string(), json!(" true ")),
            ("report".to_string(), json!("success")),
        ]);
        let false_args = HashMap::from_iter([
            ("success".to_string(), json!(" false ")),
            ("report".to_string(), json!("failure")),
        ]);
        let bool_args = HashMap::from_iter([
            ("success".to_string(), json!(true)),
            ("report".to_string(), json!("boolean success")),
        ]);

        assert!(parse_success_arg(&true_args).unwrap());
        assert!(!parse_success_arg(&false_args).unwrap());
        assert!(parse_success_arg(&bool_args).unwrap());
    }

    #[tokio::test]
    async fn spawn_agent_finish_missing_worktree_returns_error() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("missing-worktree");
        let result = auto_commit_worktree_with_message(
            &missing,
            "T-1",
            "Card T-1",
            Some("test commit".to_string()),
        )
        .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("does not exist"));
    }

    #[tokio::test]
    async fn spawn_agent_finish_non_git_worktree_returns_error() {
        let temp = tempfile::tempdir().unwrap();
        let non_git = temp.path().join("non-git");
        std::fs::create_dir_all(&non_git).unwrap();
        let result = auto_commit_worktree_with_message(
            &non_git,
            "T-1",
            "Card T-1",
            Some("test commit".to_string()),
        )
        .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not a git worktree/repo"));
    }

    #[tokio::test]
    async fn spawn_agent_finish_clean_worktree_returns_no_commit() {
        let temp = tempfile::tempdir().unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        init_repo(&repo);

        let commit = auto_commit_worktree_with_message(
            &repo,
            "T-1",
            "Card T-1",
            Some("test commit".to_string()),
        )
        .await
        .unwrap();

        assert!(commit.is_none());
        assert!(run_git(&repo, &["status", "--porcelain"]).trim().is_empty());
    }

    #[tokio::test]
    async fn spawn_agent_finish_auto_commits_from_worktree_root() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let worktree = temp.path().join("agent-worktree");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        run_git(
            &source,
            &[
                "worktree",
                "add",
                "-b",
                "refact/task/task-1/card/T-1/agent",
                worktree.to_str().unwrap(),
            ],
        );
        std::fs::write(worktree.join("file.txt"), "changed in worktree\n").unwrap();
        let commit = auto_commit_worktree_with_message(
            &worktree,
            "T-1",
            "Card T-1",
            Some("test commit".to_string()),
        )
        .await
        .unwrap();

        assert!(commit.is_some());
        assert!(run_git(&worktree, &["status", "--porcelain"])
            .trim()
            .is_empty());
        assert_eq!(
            std::fs::read_to_string(source.join("file.txt")).unwrap(),
            "hello\n"
        );
        assert_eq!(
            std::fs::read_to_string(worktree.join("file.txt")).unwrap(),
            "changed in worktree\n"
        );
    }

    #[tokio::test]
    async fn spawn_agent_finish_default_commit_message_does_not_need_subchat() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        let worktree = temp.path().join("agent-worktree-default-message");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        run_git(
            &source,
            &[
                "worktree",
                "add",
                "-b",
                "refact/task/task-1/card/T-2/agent",
                worktree.to_str().unwrap(),
            ],
        );
        std::fs::write(
            worktree.join("file.txt"),
            "changed without generated message\n",
        )
        .unwrap();

        let commit = auto_commit_worktree_with_message(
            &worktree,
            "T-2",
            "  Implement    stable finish\nwithout subchat  ",
            None,
        )
        .await
        .unwrap();

        assert!(commit.is_some());
        let subject = run_git(&worktree, &["log", "-1", "--pretty=%s"]);
        assert_eq!(
            subject.trim(),
            "Card T-2: Implement stable finish without subchat"
        );
    }
}
