use std::collections::HashMap;
use std::process::Command;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;
use uuid::Uuid;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::chat::internal_roles::{event, EventSubkind};
use crate::global_context::GlobalContext;
use crate::tasks::storage;
use crate::tasks::types::{AbVariantInfo, AbVariants, BoardCard, StatusUpdate};
use crate::tools::tool_task_spawn_agent::{
    build_agent_prompt, build_agent_thread_params, find_abandoned_worktrees,
    prepare_agent_worktree_with_suffix, resolve_agent_model, PreparedWorktree,
};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};
use crate::worktrees::service::WorktreeService;
use crate::worktrees::types::WorktreeMeta;
use refact_chat_api::ChatCommand;
use refact_runtime_api::CreateSessionRequest;

#[derive(Clone, Debug, PartialEq, Eq)]
struct AbVariantSpec {
    model: Option<String>,
    extra_instructions: Option<String>,
    suggested_steps: usize,
}

#[derive(Debug)]
struct PreparedAbVariant {
    agent_id: String,
    chat_id: String,
    model: String,
    prepared: PreparedWorktree,
}

#[derive(Debug)]
struct PreparedAbVariants {
    a: PreparedAbVariant,
    b: PreparedAbVariant,
}

pub struct ToolSpawnAb;
pub struct ToolPickAbWinner;

impl ToolSpawnAb {
    pub fn new() -> Self {
        Self
    }
}

impl ToolPickAbWinner {
    pub fn new() -> Self {
        Self
    }
}

fn make_source() -> ToolSource {
    ToolSource {
        source_type: ToolSourceType::Builtin,
        config_path: String::new(),
    }
}

fn validate_id(id: &str, name: &str) -> Result<(), String> {
    if id
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        Ok(())
    } else {
        Err(format!(
            "{} '{}' contains invalid characters (only alphanumeric, '-', '_' allowed)",
            name, id
        ))
    }
}

fn parse_suggested_steps(value: Option<&Value>) -> Result<usize, String> {
    let steps = match value {
        Some(Value::String(s)) => s
            .parse::<usize>()
            .map_err(|_| "suggested_steps must be a non-negative integer".to_string())?,
        Some(Value::Number(n)) => n
            .as_u64()
            .ok_or_else(|| "suggested_steps must be a non-negative integer".to_string())?
            as usize,
        Some(Value::Null) | None => 30,
        Some(_) => return Err("suggested_steps must be a non-negative integer".to_string()),
    };
    Ok(steps.min(50).max(1))
}

fn parse_optional_string(value: Option<&Value>, key: &str) -> Result<Option<String>, String> {
    match value {
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(Some(s.clone())),
        Some(Value::String(_)) | Some(Value::Null) | None => Ok(None),
        Some(_) => Err(format!("{} must be a string", key)),
    }
}

fn parse_variant(args: &HashMap<String, Value>, key: &str) -> Result<AbVariantSpec, String> {
    let value = args.get(key).ok_or_else(|| format!("Missing '{}'", key))?;
    let object = value
        .as_object()
        .ok_or_else(|| format!("{} must be an object", key))?;
    Ok(AbVariantSpec {
        model: parse_optional_string(object.get("model"), "model")?,
        extra_instructions: parse_optional_string(
            object.get("extra_instructions"),
            "extra_instructions",
        )?,
        suggested_steps: parse_suggested_steps(object.get("suggested_steps"))?,
    })
}

async fn planner_context(
    ccx: &Arc<AMutex<AtCommandsContext>>,
    args: &HashMap<String, Value>,
    tool_name: &str,
) -> Result<(Arc<GlobalContext>, String, String, String), String> {
    let ccx_lock = ccx.lock().await;
    let is_planner = ccx_lock
        .task_meta
        .as_ref()
        .map(|m| m.role == "planner")
        .unwrap_or(false);
    if !is_planner {
        return Err(format!(
            "{} can only be called by the task planner.",
            tool_name
        ));
    }
    let task_id = args
        .get("task_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| ccx_lock.task_meta.as_ref().map(|m| m.task_id.clone()))
        .or_else(|| storage::infer_task_id_from_chat_id(&ccx_lock.chat_id))
        .ok_or_else(|| "Missing 'task_id' (and chat is not bound to a task)".to_string())?;
    let planner_chat_id = ccx_lock
        .task_meta
        .as_ref()
        .and_then(|m| m.planner_chat_id.clone())
        .unwrap_or_else(|| ccx_lock.chat_id.clone());
    Ok((
        ccx_lock.app.gcx.clone(),
        task_id,
        planner_chat_id,
        ccx_lock.current_model.clone(),
    ))
}

fn ab_prompt_instructions(card_instructions: &str, variant: &AbVariantSpec) -> String {
    match variant.extra_instructions.as_deref() {
        Some(extra) => format!(
            "{}\n\n## Variant Instructions\n{}",
            card_instructions, extra
        ),
        None => card_instructions.to_string(),
    }
}

fn variant_info(variant: &PreparedAbVariant) -> AbVariantInfo {
    AbVariantInfo {
        agent_id: variant.agent_id.clone(),
        chat_id: variant.chat_id.clone(),
        worktree: variant
            .prepared
            .worktree_path()
            .to_string_lossy()
            .to_string(),
        worktree_name: Some(variant.prepared.worktree_name()),
        branch: variant.prepared.branch_name(),
        model: Some(variant.model.clone()),
    }
}

async fn prepare_ab_worktrees(
    gcx: Arc<GlobalContext>,
    task_meta: &crate::tasks::types::TaskMeta,
    task_id: &str,
    card_id: &str,
    model_a: String,
    model_b: String,
) -> Result<PreparedAbVariants, String> {
    let agent_id_a = Uuid::new_v4().to_string();
    let agent_id_b = Uuid::new_v4().to_string();
    let chat_id_a = format!("agent-{}-{}-a", card_id, &agent_id_a[..8]);
    let chat_id_b = format!("agent-{}-{}-b", card_id, &agent_id_b[..8]);

    let prepared_a = prepare_agent_worktree_with_suffix(
        gcx.clone(),
        task_meta,
        task_id,
        &agent_id_a,
        card_id,
        &chat_id_a,
        Some("-a"),
    )
    .await?;

    let prepared_b = match prepare_agent_worktree_with_suffix(
        gcx.clone(),
        task_meta,
        task_id,
        &agent_id_b,
        card_id,
        &chat_id_b,
        Some("-b"),
    )
    .await
    {
        Ok(prepared) => prepared,
        Err(error) => {
            prepared_a.cleanup_unlinked(gcx).await;
            return Err(error);
        }
    };

    Ok(PreparedAbVariants {
        a: PreparedAbVariant {
            agent_id: agent_id_a,
            chat_id: chat_id_a,
            model: model_a,
            prepared: prepared_a,
        },
        b: PreparedAbVariant {
            agent_id: agent_id_b,
            chat_id: chat_id_b,
            model: model_b,
            prepared: prepared_b,
        },
    })
}

async fn create_variant_session(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    planner_chat_id: &str,
    card_id: &str,
    card_title: &str,
    card_instructions: &str,
    dependency_context: &str,
    spec: &AbVariantSpec,
    variant: &PreparedAbVariant,
    worktree: WorktreeMeta,
) -> Result<(), String> {
    let app = crate::app_state::AppState::from_gcx(gcx).await;
    let thread = build_agent_thread_params(
        &variant.chat_id,
        card_title,
        &variant.model,
        task_id,
        &variant.agent_id,
        card_id,
        planner_chat_id,
        worktree,
    );
    let prompt_instructions = ab_prompt_instructions(card_instructions, spec);
    let user_prompt = build_agent_prompt(
        card_title,
        &prompt_instructions,
        dependency_context,
        spec.suggested_steps,
    );
    let initial_message = event(
        EventSubkind::SystemNotice,
        "tool.spawn_ab",
        json!({
            "task_id": task_id,
            "planner_chat_id": planner_chat_id,
            "card_id": card_id,
            "card_title": card_title,
            "agent_id": &variant.agent_id,
            "agent_chat_id": &variant.chat_id,
            "model": &variant.model,
            "suggested_steps": spec.suggested_steps,
            "has_extra_instructions": spec.extra_instructions.is_some(),
            "worktree": variant.prepared.worktree_path().to_string_lossy().to_string(),
            "worktree_name": variant.prepared.worktree_name(),
            "branch": variant.prepared.branch_name(),
        }),
        user_prompt,
    );
    app.chat
        .facade
        .create_session(CreateSessionRequest {
            chat_id: variant.chat_id.clone(),
            thread,
            messages: vec![initial_message],
        })
        .await?;
    app.chat.facade.maybe_save_session(&variant.chat_id).await
}

async fn start_variant_session(gcx: Arc<GlobalContext>, chat_id: &str) -> Result<(), String> {
    let app = crate::app_state::AppState::from_gcx(gcx).await;
    app.chat
        .facade
        .push_command(chat_id, ChatCommand::Regenerate {})
        .await
}

async fn restore_card_after_spawn_ab_failure(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    original_card: BoardCard,
    chat_id_a: String,
    chat_id_b: String,
) {
    let stats_gcx = gcx.clone();
    let _ = storage::update_board_atomic(gcx, task_id, move |board| {
        if let Some(card) = board.get_card_mut(&original_card.id) {
            let matches_ab = card
                .ab_variants
                .as_ref()
                .map(|ab| ab.a.chat_id == chat_id_a && ab.b.chat_id == chat_id_b)
                .unwrap_or(false);
            if matches_ab {
                *card = original_card.clone();
            }
        }
        Ok(())
    })
    .await;
    let _ = storage::update_task_stats(stats_gcx, task_id).await;
}

fn winner_parts<'a>(
    variants: &'a AbVariants,
    winner: &str,
) -> Result<(&'a AbVariantInfo, &'a AbVariantInfo), String> {
    match winner {
        "a" => Ok((&variants.a, &variants.b)),
        "b" => Ok((&variants.b, &variants.a)),
        _ => Err(format!("Invalid winner '{}', must be 'a' or 'b'", winner)),
    }
}

async fn cleanup_ab_variant(
    gcx: Arc<GlobalContext>,
    variant: &AbVariantInfo,
) -> Result<(), String> {
    let project_dirs = crate::files_correction::get_project_dirs(gcx.clone()).await;
    let workspace_root = project_dirs.first().ok_or("No workspace folder found")?;

    if let Some(worktree_name) = variant.worktree_name.as_deref() {
        if let Ok(service) = WorktreeService::new(gcx.cache_dir.clone(), workspace_root.clone()) {
            if service
                .delete_worktree(worktree_name, true, true)
                .await
                .is_ok()
            {
                return Ok(());
            }
        }
    }

    let worktree_removed = Command::new("git")
        .args(["worktree", "remove", &variant.worktree, "--force"])
        .current_dir(workspace_root)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    if !worktree_removed && std::path::Path::new(&variant.worktree).exists() {
        std::fs::remove_dir_all(&variant.worktree).map_err(|e| {
            format!(
                "Failed to remove loser worktree '{}': {}",
                variant.worktree, e
            )
        })?;
    }
    if let Some(branch) = variant.branch.as_deref() {
        let _ = Command::new("git")
            .args(["branch", "-D", branch])
            .current_dir(workspace_root)
            .output();
    }
    Ok(())
}

async fn pick_ab_winner_impl(
    gcx: Arc<GlobalContext>,
    task_id: &str,
    card_id: &str,
    winner: &str,
) -> Result<(String, String), String> {
    if winner != "a" && winner != "b" {
        return Err(format!("Invalid winner '{}', must be 'a' or 'b'", winner));
    }

    let board = storage::load_board(gcx.clone(), task_id).await?;
    let card = board
        .get_card(card_id)
        .ok_or_else(|| format!("Card {} not found", card_id))?;
    let variants = card
        .ab_variants
        .clone()
        .ok_or_else(|| format!("Card {} has no A/B variants", card_id))?;
    let (winner_variant, loser_variant) = winner_parts(&variants, winner)?;
    let winner_variant = winner_variant.clone();
    let loser_variant = loser_variant.clone();

    cleanup_ab_variant(gcx.clone(), &loser_variant).await?;

    let card_id_owned = card_id.to_string();
    let winner_owned = winner.to_string();
    let loser_key = if winner == "a" { "b" } else { "a" }.to_string();
    let winner_for_board = winner_variant.clone();
    storage::update_board_atomic(gcx.clone(), task_id, move |board| {
        let card = board
            .get_card_mut(&card_id_owned)
            .ok_or(format!("Card {} not found", card_id_owned))?;
        card.agent_worktree = Some(winner_for_board.worktree.clone());
        card.agent_worktree_name = winner_for_board.worktree_name.clone();
        card.agent_branch = winner_for_board.branch.clone();
        card.agent_chat_id = Some(winner_for_board.chat_id.clone());
        card.assignee = Some(winner_for_board.agent_id.clone());
        card.ab_variants = None;
        card.status_updates.push(StatusUpdate {
            timestamp: Utc::now().to_rfc3339(),
            message: format!("A/B winner: {}, loser cleaned up", winner_owned),
        });
        Ok(())
    })
    .await?;
    storage::update_task_stats(gcx, task_id).await?;
    Ok((winner_variant.chat_id, loser_key))
}

#[async_trait]
impl Tool for ToolSpawnAb {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "spawn_ab".to_string(),
            display_name: "Spawn A/B Agents".to_string(),
            source: make_source(),
            experimental: false,
            allow_parallel: false,
            description: "Spawn two task agents for the same planned card in separate worktrees so the planner can pick a winner.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string", "description": "Task UUID (optional in planner context)" },
                    "card_id": { "type": "string", "description": "Planned card ID to race" },
                    "variant_a": {
                        "type": "object",
                        "properties": {
                            "model": { "type": "string" },
                            "extra_instructions": { "type": "string" },
                            "suggested_steps": { "type": "integer" }
                        }
                    },
                    "variant_b": {
                        "type": "object",
                        "properties": {
                            "model": { "type": "string" },
                            "extra_instructions": { "type": "string" },
                            "suggested_steps": { "type": "integer" }
                        }
                    }
                },
                "required": ["card_id", "variant_a", "variant_b"]
            }),
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
        let (gcx, task_id, planner_chat_id, current_model) =
            planner_context(&ccx, args, "spawn_ab").await?;
        let card_id = args
            .get("card_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'card_id'")?;
        validate_id(&task_id, "task_id")?;
        validate_id(card_id, "card_id")?;
        let variant_a = parse_variant(args, "variant_a")?;
        let variant_b = parse_variant(args, "variant_b")?;
        let task_meta = storage::load_task_meta(gcx.clone(), &task_id).await?;
        let model_a = match variant_a.model.as_deref() {
            Some(model) => model.to_string(),
            None => {
                resolve_agent_model(
                    gcx.clone(),
                    task_meta.default_agent_model.as_deref(),
                    &current_model,
                )
                .await?
            }
        };
        let model_b = match variant_b.model.as_deref() {
            Some(model) => model.to_string(),
            None => {
                resolve_agent_model(
                    gcx.clone(),
                    task_meta.default_agent_model.as_deref(),
                    &current_model,
                )
                .await?
            }
        };

        let board = storage::load_board(gcx.clone(), &task_id).await?;
        let abandoned_worktrees = find_abandoned_worktrees(&board);
        if !abandoned_worktrees.is_empty() {
            return Err(format!(
                "Cannot spawn A/B agents while abandoned task worktrees exist. Clean them first.\n\n{}",
                abandoned_worktrees.join("\n")
            ));
        }
        let card = board
            .get_card(card_id)
            .ok_or_else(|| format!("Card {} not found", card_id))?;
        if card.column != "planned" {
            return Err(format!(
                "Card {} is in column '{}', expected 'planned'",
                card_id, card.column
            ));
        }
        if card.ab_variants.is_some() {
            return Err(format!("Card {} already has A/B variants", card_id));
        }
        let original_card = card.clone();
        let card_title = card.title.clone();
        let card_instructions = card.instructions.clone();
        let dependency_context = board
            .get_dependency_reports(card_id)
            .into_iter()
            .map(|(title, report)| format!("### {}\n{}", title, report))
            .collect::<Vec<_>>()
            .join("\n\n");

        let prepared =
            prepare_ab_worktrees(gcx.clone(), &task_meta, &task_id, card_id, model_a, model_b)
                .await?;
        let variants = AbVariants {
            a: variant_info(&prepared.a),
            b: variant_info(&prepared.b),
        };

        let card_id_owned = card_id.to_string();
        let variants_for_board = variants.clone();
        let board_update = storage::update_board_atomic(gcx.clone(), &task_id, move |board| {
            let card = board
                .get_card_mut(&card_id_owned)
                .ok_or(format!("Card {} not found", card_id_owned))?;
            if card.column != "planned" {
                return Err(format!(
                    "Card {} is in column '{}', expected 'planned'",
                    card_id_owned, card.column
                ));
            }
            if card.ab_variants.is_some() {
                return Err(format!("Card {} already has A/B variants", card_id_owned));
            }
            card.column = "doing".to_string();
            card.assignee = Some("ab".to_string());
            card.agent_chat_id = None;
            card.agent_branch = None;
            card.agent_worktree = None;
            card.agent_worktree_name = None;
            card.started_at = Some(Utc::now().to_rfc3339());
            card.ab_variants = Some(variants_for_board.clone());
            card.status_updates.push(StatusUpdate {
                timestamp: Utc::now().to_rfc3339(),
                message: format!(
                    "A/B spawned: A={}, B={}",
                    variants_for_board.a.chat_id, variants_for_board.b.chat_id
                ),
            });
            Ok(())
        })
        .await;
        if let Err(error) = board_update {
            prepared.a.prepared.cleanup_unlinked(gcx.clone()).await;
            prepared.b.prepared.cleanup_unlinked(gcx.clone()).await;
            return Err(error);
        }
        storage::update_task_stats(gcx.clone(), &task_id).await?;

        let session_result = async {
            create_variant_session(
                gcx.clone(),
                &task_id,
                &planner_chat_id,
                card_id,
                &card_title,
                &card_instructions,
                &dependency_context,
                &variant_a,
                &prepared.a,
                prepared.a.prepared.worktree_meta(),
            )
            .await?;
            create_variant_session(
                gcx.clone(),
                &task_id,
                &planner_chat_id,
                card_id,
                &card_title,
                &card_instructions,
                &dependency_context,
                &variant_b,
                &prepared.b,
                prepared.b.prepared.worktree_meta(),
            )
            .await?;
            start_variant_session(gcx.clone(), &prepared.a.chat_id).await?;
            start_variant_session(gcx.clone(), &prepared.b.chat_id).await
        }
        .await;
        if let Err(error) = session_result {
            restore_card_after_spawn_ab_failure(
                gcx.clone(),
                &task_id,
                original_card,
                prepared.a.chat_id.clone(),
                prepared.b.chat_id.clone(),
            )
            .await;
            prepared.a.prepared.cleanup_unlinked(gcx.clone()).await;
            prepared.b.prepared.cleanup_unlinked(gcx.clone()).await;
            return Err(error);
        }

        let result_message = format!(
            "# A/B Agents Spawned\n\n**Card:** {}\n**A:** {} ({})\n**B:** {} ({})\n\nUse `pick_ab_winner(card_id=\"{}\", winner=\"a\"|\"b\")` after comparing the variants.",
            card_id,
            variants.a.chat_id,
            prepared.a.model,
            variants.b.chat_id,
            prepared.b.model,
            card_id
        );
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
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[async_trait]
impl Tool for ToolPickAbWinner {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "pick_ab_winner".to_string(),
            display_name: "Pick A/B Winner".to_string(),
            source: make_source(),
            experimental: false,
            allow_parallel: false,
            description: "Pick the winning A/B task-agent variant, promote its worktree to the primary card worktree, and delete the loser.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "task_id": { "type": "string", "description": "Task UUID (optional in planner context)" },
                    "card_id": { "type": "string", "description": "A/B card ID" },
                    "winner": { "type": "string", "enum": ["a", "b"], "description": "Winning variant" }
                },
                "required": ["card_id", "winner"]
            }),
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
        let (gcx, task_id, _, _) = planner_context(&ccx, args, "pick_ab_winner").await?;
        let card_id = args
            .get("card_id")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'card_id'")?;
        let winner = args
            .get("winner")
            .and_then(|v| v.as_str())
            .ok_or("Missing 'winner'")?;
        validate_id(&task_id, "task_id")?;
        validate_id(card_id, "card_id")?;
        let (winner_chat_id, loser_key) =
            pick_ab_winner_impl(gcx, &task_id, card_id, winner).await?;
        let result_message = format!(
            "# A/B Winner Picked\n\n**Card:** {}\n**Winner:** {} ({})\n**Loser:** {} cleaned up",
            card_id, winner, winner_chat_id, loser_key
        );
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
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::types::{TaskBoard, TaskMeta, TaskStatus};
    use std::path::Path;

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

    fn init_repo(root: &Path) {
        run_git(root, &["init"]);
        run_git(root, &["checkout", "-b", "main"]);
        run_git(root, &["config", "user.email", "test@example.com"]);
        run_git(root, &["config", "user.name", "Test User"]);
        std::fs::write(root.join("file.txt"), "hello\n").unwrap();
        run_git(root, &["add", "file.txt"]);
        run_git(root, &["commit", "-m", "initial"]);
    }

    async fn set_workspace(gcx: Arc<GlobalContext>, root: &Path) {
        let root = root.canonicalize().unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root];
    }

    fn task_meta(task_id: &str) -> TaskMeta {
        let now = Utc::now().to_rfc3339();
        TaskMeta {
            schema_version: 1,
            id: task_id.to_string(),
            name: "Task".to_string(),
            status: TaskStatus::Active,
            created_at: now.clone(),
            updated_at: now,
            cards_total: 1,
            cards_done: 0,
            cards_failed: 0,
            agents_active: 0,
            base_branch: None,
            base_commit: None,
            default_agent_model: None,
            is_name_generated: false,
            last_agents_summary_at: None,
            planner_session_state: None,
        }
    }

    fn test_card() -> BoardCard {
        BoardCard {
            id: "T-1".to_string(),
            title: "Card T-1".to_string(),
            column: "planned".to_string(),
            priority: "P1".to_string(),
            depends_on: vec![],
            instructions: "Do the thing".to_string(),
            assignee: None,
            agent_chat_id: None,
            status_updates: vec![],
            comments: vec![],
            final_report: None,
            final_report_structured: None,
            verifier_report: None,
            created_at: Utc::now().to_rfc3339(),
            started_at: None,
            last_heartbeat_at: None,
            completed_at: None,
            agent_branch: None,
            agent_worktree: None,
            agent_worktree_name: None,
            ab_variants: None,
            team_members: vec![],
            target_files: vec![],
            scope_guard_mode: Default::default(),
        }
    }

    async fn setup_repo_task() -> (Arc<GlobalContext>, tempfile::TempDir, std::path::PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("repo");
        std::fs::create_dir_all(&source).unwrap();
        init_repo(&source);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        set_workspace(gcx.clone(), &source).await;
        let task_dir = source.join(".refact").join("tasks").join("task-1");
        tokio::fs::create_dir_all(&task_dir).await.unwrap();
        tokio::fs::write(
            task_dir.join("meta.yaml"),
            serde_yaml::to_string(&task_meta("task-1")).unwrap(),
        )
        .await
        .unwrap();
        let mut board = TaskBoard::default();
        board.cards.push(test_card());
        tokio::fs::write(
            task_dir.join("board.yaml"),
            serde_yaml::to_string(&board).unwrap(),
        )
        .await
        .unwrap();
        (gcx, temp, source)
    }

    #[test]
    fn spawn_ab_ab_variants_field_defaults_to_none() {
        let card: BoardCard = serde_json::from_str(
            r#"{
                "id":"T-1",
                "title":"legacy",
                "column":"planned",
                "assignee":null,
                "agent_chat_id":null,
                "created_at":"2026-05-16T00:00:00Z",
                "started_at":null,
                "completed_at":null
            }"#,
        )
        .unwrap();

        assert!(card.ab_variants.is_none());
    }

    #[tokio::test]
    async fn spawn_ab_creates_2_worktrees() {
        let (gcx, _temp, _source) = setup_repo_task().await;
        let meta = storage::load_task_meta(gcx.clone(), "task-1")
            .await
            .unwrap();

        let prepared = prepare_ab_worktrees(
            gcx.clone(),
            &meta,
            "task-1",
            "T-1",
            "model-a".to_string(),
            "model-b".to_string(),
        )
        .await
        .unwrap();

        assert!(prepared.a.prepared.worktree_path().is_dir());
        assert!(prepared.b.prepared.worktree_path().is_dir());
        assert_ne!(
            prepared.a.prepared.worktree_path(),
            prepared.b.prepared.worktree_path()
        );
        assert!(prepared
            .a
            .prepared
            .branch_name()
            .as_deref()
            .unwrap()
            .ends_with("-a"));
        assert!(prepared
            .b
            .prepared
            .branch_name()
            .as_deref()
            .unwrap()
            .ends_with("-b"));

        prepared.a.prepared.cleanup_unlinked(gcx.clone()).await;
        prepared.b.prepared.cleanup_unlinked(gcx).await;
    }

    #[tokio::test]
    async fn pick_ab_winner_cleans_up_loser() {
        let (gcx, _temp, _source) = setup_repo_task().await;
        let meta = storage::load_task_meta(gcx.clone(), "task-1")
            .await
            .unwrap();
        let prepared = prepare_ab_worktrees(
            gcx.clone(),
            &meta,
            "task-1",
            "T-1",
            "model-a".to_string(),
            "model-b".to_string(),
        )
        .await
        .unwrap();
        let winner_root = prepared.a.prepared.worktree_path();
        let loser_root = prepared.b.prepared.worktree_path();
        let loser_branch = prepared.b.prepared.branch_name().unwrap();
        let variants = AbVariants {
            a: variant_info(&prepared.a),
            b: variant_info(&prepared.b),
        };
        storage::update_board_atomic(gcx.clone(), "task-1", move |board| {
            let card = board.get_card_mut("T-1").unwrap();
            card.column = "doing".to_string();
            card.assignee = Some("ab".to_string());
            card.ab_variants = Some(variants.clone());
            Ok(())
        })
        .await
        .unwrap();

        let (winner_chat_id, loser_key) = pick_ab_winner_impl(gcx.clone(), "task-1", "T-1", "a")
            .await
            .unwrap();
        let board = storage::load_board(gcx.clone(), "task-1").await.unwrap();
        let card = board.get_card("T-1").unwrap();

        assert_eq!(loser_key, "b");
        assert_eq!(winner_chat_id, prepared.a.chat_id);
        assert_eq!(
            card.agent_worktree.as_deref(),
            Some(winner_root.to_str().unwrap())
        );
        assert!(card.ab_variants.is_none());
        assert!(winner_root.is_dir());
        assert!(!loser_root.exists());
        let branches = run_git(
            &prepared.a.prepared.source_workspace_root(),
            &["branch", "--list", &loser_branch],
        );
        assert!(branches.trim().is_empty());

        cleanup_ab_variant(gcx, &variant_info(&prepared.a))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn pick_ab_winner_with_invalid_winner_returns_error() {
        let (gcx, _temp, _source) = setup_repo_task().await;
        let err = pick_ab_winner_impl(gcx, "task-1", "T-1", "c")
            .await
            .unwrap_err();

        assert!(err.contains("Invalid winner"));
    }
}
