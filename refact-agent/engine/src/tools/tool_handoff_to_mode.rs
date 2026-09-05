use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;
use uuid::Uuid;

use crate::agentic::mode_transition::{
    ParsedDecisions, insert_goal_messages_before_plan, transfer_goal_ownership,
};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::chat::trajectories::{
    chat_id_is_planner_for_task, resolve_task_planner_controller_chat_id,
    verified_planner_linked_root_chat_id,
};
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::tasks::storage;
use crate::tools::tool_task_documents::{
    create_document_at, documents_dir_for_task, next_available_slug_at,
};
use refact_chat_history::trajectory_ops::sanitize_messages_for_new_thread;
use refact_chat_history::trajectory_snapshot::TrajectorySnapshot;
use refact_runtime_api::SessionState;
use crate::tools::tools_description::{
    MatchConfirmDeny, MatchConfirmDenyResult, Tool, ToolDesc, ToolSource, ToolSourceType,
};
use crate::yaml_configs::customization_registry::{get_mode_config, map_legacy_mode_to_id};

fn parse_string_list(args: &HashMap<String, Value>, key: &str) -> Vec<String> {
    match args.get(key) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        Some(Value::String(text)) => {
            let trimmed = text.trim();
            if trimmed.starts_with('[') {
                serde_json::from_str::<Vec<String>>(trimmed).unwrap_or_default()
            } else {
                trimmed
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            }
        }
        _ => vec![],
    }
}

fn parse_optional_string(args: &HashMap<String, Value>, key: &str) -> Option<String> {
    match args.get(key) {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        _ => None,
    }
}

fn epoch_ms_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn apply_overrides(decisions: &mut ParsedDecisions, args: &HashMap<String, Value>) {
    if let Some(summary) = parse_optional_string(args, "summary") {
        decisions.summary = summary;
    }
    if let Some(summary) = parse_optional_string(args, "context_summary") {
        decisions.summary = summary;
    }
    let files_to_open = parse_string_list(args, "files_to_open");
    if !files_to_open.is_empty() {
        decisions.files_to_open = files_to_open;
    }
    let key_files = parse_string_list(args, "key_files");
    if !key_files.is_empty() {
        decisions.files_to_open = key_files;
    }
    let messages_to_preserve = parse_string_list(args, "messages_to_preserve");
    if !messages_to_preserve.is_empty() {
        decisions.messages_to_preserve = messages_to_preserve;
    }
    let memories_to_include = parse_string_list(args, "memories_to_include");
    if !memories_to_include.is_empty() {
        decisions.memories_to_include = memories_to_include;
    }
    let tool_outputs_to_include = parse_string_list(args, "tool_outputs_to_include");
    if !tool_outputs_to_include.is_empty() {
        decisions.tool_outputs_to_include = tool_outputs_to_include;
    }
    let pending_tasks = parse_string_list(args, "pending_tasks");
    if !pending_tasks.is_empty() {
        decisions.pending_tasks = pending_tasks;
    }
    if let Some(handoff_message) = parse_optional_string(args, "handoff_message") {
        decisions.handoff_message = handoff_message;
    }
    if let Some(initial_plan) = parse_optional_string(args, "initial_plan") {
        decisions.initial_plan = Some(initial_plan);
    }
}

async fn ensure_task_for_planner_handoff(
    gcx: Arc<crate::global_context::GlobalContext>,
    canonical_mode: &str,
    existing_task_meta: Option<refact_chat_api::TaskMeta>,
    current_chat_id: &str,
    current_root_chat_id: Option<&str>,
) -> Result<Option<refact_chat_api::TaskMeta>, String> {
    if canonical_mode != "task_planner" {
        return Ok(existing_task_meta);
    }
    if let Some(task_meta) = existing_task_meta {
        if task_meta.role == "planner" {
            let planner_chat_id = resolve_task_planner_controller_chat_id(
                gcx.clone(),
                current_chat_id,
                current_root_chat_id,
                Some(&task_meta),
            )
            .await;
            return Ok(Some(refact_chat_api::TaskMeta {
                task_id: task_meta.task_id,
                role: "planner".to_string(),
                agent_id: None,
                card_id: None,
                planner_chat_id: Some(planner_chat_id),
            }));
        }
        if let Some(planner_chat_id) = verified_planner_linked_root_chat_id(
            gcx.clone(),
            current_chat_id,
            current_root_chat_id,
            &task_meta,
        )
        .await
        {
            return Ok(Some(refact_chat_api::TaskMeta {
                task_id: task_meta.task_id,
                role: "planner".to_string(),
                agent_id: None,
                card_id: None,
                planner_chat_id: Some(planner_chat_id),
            }));
        }
        if let Some(planner_chat_id) = task_meta
            .planner_chat_id
            .clone()
            .filter(|id| !id.is_empty())
        {
            if chat_id_is_planner_for_task(gcx.clone(), &planner_chat_id, &task_meta.task_id).await
            {
                return Ok(Some(refact_chat_api::TaskMeta {
                    task_id: task_meta.task_id,
                    role: "planner".to_string(),
                    agent_id: None,
                    card_id: None,
                    planner_chat_id: Some(planner_chat_id),
                }));
            }
        }
        let chat_id = storage::next_planner_chat_id(gcx, &task_meta.task_id).await?;
        return Ok(Some(refact_chat_api::TaskMeta {
            task_id: task_meta.task_id,
            role: "planner".to_string(),
            agent_id: None,
            card_id: None,
            planner_chat_id: Some(chat_id),
        }));
    }
    let task = storage::create_task(gcx.clone(), "New Task").await?;
    let chat_id = storage::next_planner_chat_id(gcx, &task.id).await?;
    Ok(Some(refact_chat_api::TaskMeta {
        task_id: task.id,
        role: "planner".to_string(),
        agent_id: None,
        card_id: None,
        planner_chat_id: Some(chat_id),
    }))
}

async fn create_initial_plan_document(
    gcx: Arc<crate::global_context::GlobalContext>,
    task_id: &str,
    plan_text: &str,
) -> Result<String, String> {
    let documents_dir = documents_dir_for_task(gcx, task_id).await?;
    let slug = next_available_slug_at(&documents_dir, "initial-plan").await?;
    create_document_at(
        &documents_dir,
        &slug,
        "Initial Plan",
        "plan",
        plan_text,
        true,
        Vec::new(),
        "planner",
    )
    .await?;
    Ok(slug)
}

/// The destination always starts with the same authoritative report artifact.
pub(crate) fn transition_report(
    messages: &[ChatMessage],
    source_version: u64,
    model: String,
    from_mode: String,
    to_mode: String,
) -> Result<Vec<ChatMessage>, String> {
    let mut payload = sanitize_messages_for_new_thread(messages);
    for message in &mut payload {
        if message.message_id.is_empty() {
            message.message_id = Uuid::new_v4().to_string();
        }
    }
    Ok(vec![
        refact_core::active_context::make_reconstruction_report(
            payload,
            refact_core::active_context::ReconstructionMetadata {
                source_version: Some(source_version),
                model: Some(model),
                trigger: Some("mode_transition".into()),
                from_mode: Some(from_mode),
                to_mode: Some(to_mode),
                ..Default::default()
            },
        )
        .map_err(|e| e.to_string())?,
    ])
}

pub(crate) fn transition_fingerprint(session: &crate::chat::types::ChatSession) -> Value {
    json!({"thread": session.thread, "goal": session.goal, "ledger": session.goal_ledger,
        "deliveries": session.pending_deliveries_for_snapshot()})
}

fn handoff_completion_messages(
    raw: &[ChatMessage],
    tool_call_id: &str,
    result: &Value,
) -> Result<(Vec<ChatMessage>, ChatMessage), String> {
    let mut updated = raw.to_vec();
    let start = refact_core::active_context::active_context(raw)
        .ok()
        .and_then(|active| active.report_index)
        .map_or(0, |i| i + 1);
    let message = updated[start..]
        .iter_mut()
        .rev()
        .find(|m| m.role == "tool" && m.tool_call_id == tool_call_id)
        .ok_or("Handoff acknowledgement disappeared")?;
    message.content = ChatContent::SimpleText(result.to_string());
    message.preserve = Some(true);
    let message = message.clone();
    Ok((updated, message))
}

fn publish_handoff_completion(
    source: &mut crate::chat::types::ChatSession,
    updated: Vec<ChatMessage>,
    message: ChatMessage,
) {
    source.replace_messages(updated);
    source.emit(crate::chat::types::ChatEvent::MessageUpdated {
        message_id: message.message_id.clone(),
        message,
    });
}

pub(crate) fn transition_source_goal(
    session: &crate::chat::types::ChatSession,
) -> Option<refact_chat_api::GoalSnapshot> {
    let mut goal = session.goal.clone();
    if let Some(state) = refact_chat_api::reduce_goal_ledger(&session.goal_ledger) {
        let goal = goal.get_or_insert_with(Default::default);
        state.apply_to_snapshot(goal);
    }
    goal
}

/// Transfer only the active view, writing it back without touching archive bytes.
/// Legacy input is explicitly rebuilt but its source history is never migrated.
pub(crate) fn transition_goal(
    raw: &[ChatMessage],
    goal: Option<&refact_chat_api::GoalSnapshot>,
    source_id: &str,
    target_id: &str,
    mode: &str,
) -> Result<crate::agentic::mode_transition::GoalTransferResult, String> {
    let input =
        refact_core::active_context::legacy_rebuild_input(raw).map_err(|e| e.to_string())?;
    let mut transfer = transfer_goal_ownership(
        &input,
        goal,
        &[],
        source_id,
        target_id,
        mode,
        epoch_ms_now(),
    );
    if transfer.transferred() {
        // Control messages inside the report are immutable reconstruction input.
        // Ledger/snapshot ownership wins over their historical active metadata.
        if refact_core::active_context::active_context(raw)
            .map(|active| active.report_index.is_some())
            .unwrap_or(true)
        {
            transfer.source_messages = raw.to_vec();
        }
    }
    Ok(transfer)
}

/// Stage a goal-free destination, durably relinquish source ownership, then
/// activate the target. Failures can leave a recoverable inactive target, never
/// two durable owners. The caller holds the source lock throughout this commit.
pub(crate) async fn persist_transition(
    gcx: Arc<crate::global_context::GlobalContext>,
    source: &mut crate::chat::types::ChatSession,
    snapshot: &TrajectorySnapshot,
    transfer: &crate::agentic::mode_transition::GoalTransferResult,
) -> Result<(), String> {
    use crate::chat::trajectories::{save_trajectory_snapshot, trajectory_snapshot_from_session};
    if !transfer.transferred() {
        return save_trajectory_snapshot(gcx, snapshot.clone()).await;
    }
    let mut staged = snapshot.clone();
    staged.goal = None;
    staged.goal_ledger.clear();
    let mut active = refact_core::active_context::active_context(&staged.messages)
        .map_err(|e| e.to_string())?
        .messages;
    active.retain(|m| m.role != "goal" && m.role != "goal_delta");
    staged.messages = transition_report(
        &active,
        source.trajectory_version,
        snapshot.model.clone(),
        source.thread.mode.clone(),
        snapshot.mode.clone(),
    )?;
    save_trajectory_snapshot(gcx.clone(), staged).await?;
    let mut source_snapshot = trajectory_snapshot_from_session(source);
    source_snapshot.messages = transfer.source_messages.clone();
    source_snapshot.goal = transfer.source_goal.clone();
    source_snapshot
        .goal_ledger
        .push(refact_chat_api::GoalLedgerEntry {
            seq: source.goal_ledger_last_seq() + 1,
            at_ms: epoch_ms_now(),
            op: refact_chat_api::GoalLedgerOp::TransferredOut {
                target_chat_id: snapshot.chat_id.clone(),
            },
        });
    source_snapshot.version += 1;
    save_trajectory_snapshot(gcx.clone(), source_snapshot.clone())
        .await
        .map_err(|e| {
            format!(
                "Source transfer save failed; inactive destination {} retained: {e}",
                snapshot.chat_id
            )
        })?;
    source.replace_messages(source_snapshot.messages);
    source.goal_ledger = source_snapshot.goal_ledger;
    source.set_goal_projection(transfer.source_goal.clone());
    source.emit_goal_status();
    let mut committed = snapshot.clone();
    committed.version += 1;
    save_trajectory_snapshot(gcx, committed).await.map_err(|e| {
        format!(
            "Source transferred; destination {} activation requires retry: {e}",
            snapshot.chat_id
        )
    })
}

pub struct ToolHandoffToMode {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolHandoffToMode {
    fn tool_description(&self) -> ToolDesc {
        let input_schema = json!({
            "type": "object",
            "properties": {
                "target_mode": {
                    "type": "string",
                    "description": "Target mode ID to hand off to."
                },
                "reason": {
                    "type": "string",
                    "description": "Why the new mode is appropriate"
                },
                "summary": {
                    "type": "string",
                    "description": "Optional summary to include in the handoff context"
                },
                "context_summary": {
                    "type": "string",
                    "description": "Summary of what has been done and what to continue"
                },
                "files_to_open": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "File paths to include in the new chat"
                },
                "key_files": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Key files to carry over (alias of files_to_open)"
                },
                "messages_to_preserve": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "MSG_ID entries to preserve verbatim"
                },
                "memories_to_include": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Memory/knowledge file paths to include"
                },
                "tool_outputs_to_include": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "MSG_ID entries of tool outputs to include"
                },
                "pending_tasks": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Pending tasks to carry forward"
                },
                "handoff_message": {
                    "type": "string",
                    "description": "Short handoff message for the new chat"
                },
                "initial_plan": {
                    "type": "string",
                    "description": "Optional plan text to save as the initial task document when target_mode is task_planner"
                }
            },
            "required": ["target_mode"]
        });

        ToolDesc {
            name: "handoff_to_mode".to_string(),
            display_name: "Handoff To Mode".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description:
                "Create a new chat in another mode using the current conversation context."
                    .to_string(),
            input_schema,
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
        let target = parse_optional_string(args, "target_mode")
            .ok_or("Missing required argument `target_mode`")?;
        let (gcx, chat_id) = {
            let c = ccx.lock().await;
            (c.app.gcx.clone(), c.chat_id.clone())
        };
        let canonical = map_legacy_mode_to_id(&target);
        get_mode_config(gcx.clone(), canonical, None)
            .await
            .ok_or("Target mode not found")?;
        let session = gcx
            .chat_sessions
            .read()
            .await
            .get(&chat_id)
            .cloned()
            .ok_or("Chat session not found")?;
        let mut session = session.lock().await;
        if session.closed
            || crate::chat::context_rebuild::compression_attempt_active(&session)
            || session.pending_mode_handoff.is_some()
            || session.pending_context_rebuild.is_some()
        {
            return Err("A context operation is already pending".into());
        }
        session.pending_mode_handoff = Some(json!({"args":args,"tool_call_id":tool_call_id}));
        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".into(),
                tool_call_id: tool_call_id.clone(),
                preserve: Some(true),
                content: ChatContent::SimpleText(
                    json!({"type":"handoff_to_mode", "queued":true, "target_mode":canonical})
                        .to_string(),
                ),
                output_filter: Some(OutputFilter::no_limits()),
                ..Default::default()
            })],
        ))
    }

    async fn command_to_match_against_confirm_deny(
        &self,
        _ccx: Arc<AMutex<AtCommandsContext>>,
        args: &HashMap<String, Value>,
    ) -> Result<String, String> {
        let target = args
            .get("target_mode")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        Ok(format!("handoff_to_mode {}", target))
    }

    async fn match_against_confirm_deny(
        &self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        args: &HashMap<String, Value>,
    ) -> Result<MatchConfirmDeny, String> {
        let command_to_match = self
            .command_to_match_against_confirm_deny(ccx.clone(), args)
            .await
            .map_err(|e| format!("Error getting tool command to match: {}", e))?;
        Ok(MatchConfirmDeny {
            result: MatchConfirmDenyResult::PASS,
            command: command_to_match,
            rule: "default".to_string(),
        })
    }
}

async fn finalize_handoff_inner(
    gcx: Arc<crate::global_context::GlobalContext>,
    session_arc: &Arc<AMutex<crate::chat::types::ChatSession>>,
    payload: Value,
) -> Result<(), String> {
    let args: HashMap<String, Value> =
        serde_json::from_value(payload.get("args").cloned().ok_or("Missing handoff args")?)
            .map_err(|e| e.to_string())?;
    let args = &args;
    let tool_call_id = payload
        .get("tool_call_id")
        .and_then(Value::as_str)
        .ok_or("Missing handoff tool call")?
        .to_string();
    let tool_call_id = &tool_call_id;
    let target_mode = match args.get("target_mode") {
        Some(Value::String(s)) if !s.trim().is_empty() => s.trim().to_string(),
        _ => return Err("Missing required argument `target_mode`".to_string()),
    };
    let reason = parse_optional_string(args, "reason").unwrap_or_default();

    let chat_facade = crate::app_state::AppState::from_gcx(gcx.clone())
        .await
        .chat
        .facade;
    let (chat_id, source_version, source_thread, source_fingerprint, abort) = {
        let session = session_arc.lock().await;
        if session.runtime.state != SessionState::Idle
            || session.closed
            || crate::chat::context_rebuild::compression_attempt_active(&session)
        {
            return Err("Handoff requires an idle source session".into());
        }
        (
            session.thread.id.clone(),
            session.trajectory_version,
            session.thread.clone(),
            transition_fingerprint(&session),
            session.abort_flag.clone(),
        )
    };

    let session_snapshot = chat_facade.session_snapshot(&chat_id).await?;
    let raw_messages = session_snapshot.messages;
    let messages = refact_core::active_context::legacy_rebuild_input(&raw_messages)
        .map_err(|e| e.to_string())?;
    let source_goal = transition_source_goal(&*session_arc.lock().await);
    let thread = session_snapshot.thread;
    let existing_task_meta = thread.task_meta.clone();
    let session_state = session_snapshot.session_state;
    let pause_reasons = session_snapshot.pause_reasons;

    match session_state {
        SessionState::Generating => {
            return Err("Cannot handoff while model is generating. Wait for the current response to complete.".to_string());
        }
        SessionState::Paused => {
            return Err(
                "Cannot handoff while session is paused. Resume or abort first.".to_string(),
            );
        }
        SessionState::WaitingIde => {
            return Err("Cannot handoff while waiting for IDE response. Cancel the IDE wait or wait for it to complete.".to_string());
        }
        SessionState::Error => {
            return Err(
                "Cannot handoff from an error state. Acknowledge the error first.".to_string(),
            );
        }
        SessionState::WaitingUserInput => {
            if !pause_reasons.is_empty() {
                return Err(
                    "Cannot handoff while pending tool approvals exist. Resolve them first."
                        .to_string(),
                );
            }
        }
        SessionState::ExecutingTools | SessionState::Idle | SessionState::Completed => {}
    }
    if messages.is_empty() {
        return Err("Cannot handoff an empty chat".to_string());
    }
    let last_asst_idx = messages
        .iter()
        .rposition(|m| m.role == "assistant" && m.tool_calls.is_some());
    if let Some(asst_idx) = last_asst_idx {
        let asst = &messages[asst_idx];
        let call_ids: std::collections::HashSet<&str> = asst
            .tool_calls
            .as_ref()
            .unwrap()
            .iter()
            .map(|c| c.id.as_str())
            .collect();
        let result_ids: std::collections::HashSet<&str> = messages[asst_idx + 1..]
            .iter()
            .filter(|m| {
                (m.role == "tool" || m.role == "diff" || m.role == "context_file")
                    && !m.tool_call_id.is_empty()
            })
            .map(|m| m.tool_call_id.as_str())
            .collect();
        let mut missing_ids: Vec<&str> = call_ids
            .difference(&result_ids)
            .copied()
            .filter(|id| *id != tool_call_id.as_str())
            .collect();
        if !missing_ids.is_empty() {
            missing_ids.sort();
            return Err(format!(
                "Cannot handoff: the latest assistant message has {} tool calls without results: {:?}",
                missing_ids.len(),
                missing_ids
            ));
        }
    }

    let canonical_mode = map_legacy_mode_to_id(&target_mode).to_string();
    let mode_config = get_mode_config(gcx.clone(), &canonical_mode, None)
        .await
        .ok_or_else(|| format!("Mode '{}' not found", canonical_mode))?;

    let mode_title = if mode_config.title.is_empty() {
        mode_config.id.clone()
    } else {
        mode_config.title.clone()
    };
    let mode_description = if mode_config.description.is_empty() {
        mode_title.clone()
    } else {
        format!("{} — {}", mode_title, mode_config.description)
    };

    let mut decisions = ParsedDecisions {
        summary: if reason.is_empty() {
            format!("Continue the conversation in {}.", mode_description)
        } else {
            reason.clone()
        },
        handoff_message: if reason.is_empty() {
            format!("Continue in {}.", mode_description)
        } else {
            reason.clone()
        },
        ..Default::default()
    };

    apply_overrides(&mut decisions, args);
    let plan_body = parse_optional_string(args, "initial_plan")
            .or_else(|| parse_optional_string(args, "summary"))
            .or_else(|| parse_optional_string(args, "context_summary"))
            .or_else(|| parse_optional_string(args, "handoff_message"))
            .unwrap_or_else(|| {
                "# Initial Plan\n\nNo plan content provided at handoff. Edit this document or use doc_create.".to_string()
            });

    let outcome = Box::pin(crate::agentic::mode_transition::reconstruct_context(
        gcx.clone(),
        crate::agentic::mode_transition::ReconstructionRequest {
            messages: &raw_messages,
            target_mode: &canonical_mode,
            target_mode_description: &mode_description,
            parent_chat_id: Some(&chat_id),
            model_override: None,
            abort_flag: Some(abort.clone()),
            hints: Some(decisions),
            target_budget_symbols: None,
            preserve_goal_messages: false,
        },
    ))
    .await?;
    let mut new_messages = outcome.messages;
    let mut source = session_arc.lock().await;
    if source.pending_mode_handoff.as_ref() != Some(&payload)
        || source.trajectory_version != source_version
        || source.runtime.state != SessionState::Idle
        || source.closed
        || source.pending_context_rebuild.is_some()
        || abort.load(std::sync::atomic::Ordering::SeqCst)
        || serde_json::to_value(&source.thread).ok() != serde_json::to_value(&source_thread).ok()
        || transition_fingerprint(&source) != source_fingerprint
        || crate::chat::context_rebuild::compression_attempt_active(&source)
    {
        return Err("Source changed during handoff reconstruction".into());
    }
    if !source
        .messages
        .iter()
        .any(|m| m.role == "tool" && m.tool_call_id == *tool_call_id)
    {
        return Err("Handoff acknowledgement disappeared".into());
    }
    let new_chat_id = Uuid::new_v4().to_string();
    let mut task_meta = existing_task_meta;
    if canonical_mode == "task_planner" {
        if let Some(meta) = task_meta.as_mut() {
            meta.role = "planner".into();
            meta.agent_id = None;
            meta.card_id = None;
            meta.planner_chat_id = Some(new_chat_id.clone());
        }
    }
    let root_chat_id = if canonical_mode == "task_planner" {
        Some(new_chat_id.clone())
    } else {
        thread
            .root_chat_id
            .clone()
            .or_else(|| Some(chat_id.clone()))
    };
    let transferred_goal = transition_goal(
        &raw_messages,
        source_goal.as_ref(),
        &chat_id,
        &new_chat_id,
        &canonical_mode,
    )?;
    if transferred_goal.transferred() {
        insert_goal_messages_before_plan(
            &mut new_messages,
            transferred_goal.target_messages.clone(),
        );
    }

    let new_messages = transition_report(
        &new_messages,
        source_version,
        outcome.model,
        thread.mode.clone(),
        canonical_mode.clone(),
    )?;
    let now = chrono::Utc::now().to_rfc3339();
    let snapshot_messages = new_messages.clone();

    let snapshot_task_meta = task_meta.clone();
    let snapshot = TrajectorySnapshot {
        goal_verification_blocked_until_ms: None,
        goal: transferred_goal.target_goal.clone(),
        goal_ledger: transferred_goal
            .target_goal
            .as_ref()
            .map(|target| {
                refact_chat_api::seed_transferred_goal_ledger(target, &chat_id, epoch_ms_now())
            })
            .unwrap_or_default(),
        chat_id: new_chat_id.clone(),
        title: String::new(),
        model: thread.model.clone(),
        mode: canonical_mode.clone(),
        tool_use: thread.tool_use.clone(),
        messages: snapshot_messages.clone(),
        created_at: now,
        boost_reasoning: thread.boost_reasoning.unwrap_or(false),
        checkpoints_enabled: thread.checkpoints_enabled,
        context_tokens_cap: thread.context_tokens_cap,
        auto_compression_cap: thread.auto_compression_cap,
        auto_compression_cap_pending: thread.auto_compression_cap.is_none(),
        include_project_info: thread.include_project_info,
        is_title_generated: false,
        auto_approve_editing_tools: thread.auto_approve_editing_tools,
        auto_approve_dangerous_commands: thread.auto_approve_dangerous_commands,
        autonomous_no_confirm: thread.autonomous_no_confirm,
        version: 1,
        task_meta: snapshot_task_meta,
        worktree: thread.worktree.clone(),
        parent_id: Some(chat_id.clone()),
        link_type: Some("handoff".to_string()),
        root_chat_id: root_chat_id.clone(),
        reasoning_effort: thread.reasoning_effort.clone(),
        thinking_budget: thread.thinking_budget,
        temperature: thread.temperature,
        frequency_penalty: thread.frequency_penalty,
        max_tokens: thread.max_tokens,
        parallel_tool_calls: thread.parallel_tool_calls,
        previous_response_id: None,
        active_skill: None,
        auto_enrichment_enabled: thread.auto_enrichment_enabled,
        buddy_meta: None,
        auto_compact_enabled: thread.auto_compact_enabled,
        frozen_request_prefix: None,
        claude_code_identity: None,
        reactive_compact_attempts: None,
        wake_up_at: None,
        waiting_for_card_ids: Vec::new(),
        pending_deliveries: Vec::new(),
    };

    persist_transition(gcx.clone(), &mut source, &snapshot, &transferred_goal).await?;
    drop(source);
    // Task/document creation happens only after a durable destination exists.
    let task_setup = ensure_task_for_planner_handoff(
        gcx.clone(),
        &canonical_mode,
        task_meta.clone(),
        &new_chat_id,
        root_chat_id.as_deref(),
    )
    .await;
    let (mut task_meta, mut task_error) = match task_setup {
        Ok(meta) => (meta, None),
        Err(error) => (snapshot.task_meta.clone(), Some(error)),
    };
    if canonical_mode == "task_planner" {
        if let Some(meta) = task_meta.as_mut() {
            meta.planner_chat_id = Some(new_chat_id.clone());
        }
    }
    let (initial_plan_doc_slug, initial_plan_doc_error) = if canonical_mode == "task_planner" {
        if let Some(meta) = &task_meta {
            match create_initial_plan_document(gcx.clone(), &meta.task_id, &plan_body).await {
                Ok(slug) => (Some(slug), None),
                Err(error) => (None, Some(error)),
            }
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };
    if let Some(meta) = &task_meta {
        let mut destination = snapshot.clone();
        destination.version += 2;
        destination.task_meta = Some(meta.clone());
        if let Err(error) = chat_facade.save_trajectory_snapshot(destination).await {
            task_error = Some(error);
            task_meta = snapshot.task_meta.clone();
        }
    }
    let result = json!({
        "type": "handoff_to_mode",
        "status": "completed",
        "new_chat_id": new_chat_id,
        "target_mode": canonical_mode,
        "reason": reason,
        "messages_count": new_messages.len(),
        "task_meta": task_meta,
        "root_chat_id": root_chat_id,
        "parent_id": chat_id,
        "link_type": "handoff",
        "initial_plan_document": initial_plan_doc_slug,
        "initial_plan_error": initial_plan_doc_error,
        "task_setup_error": task_error,
    });

    let mut source = session_arc.lock().await;
    let (updated, message) = handoff_completion_messages(&source.messages, tool_call_id, &result)?;
    let mut completion = crate::chat::trajectories::trajectory_snapshot_from_session(&source);
    completion.messages = updated.clone();
    completion.version += 1;
    crate::chat::trajectories::save_trajectory_snapshot(gcx, completion)
        .await
        .map_err(|e| {
            format!("Destination {new_chat_id} saved, but acknowledgement persistence failed: {e}")
        })?;
    publish_handoff_completion(&mut source, updated, message);

    Ok(())
}

pub async fn finalize_pending_handoff(
    gcx: Arc<crate::global_context::GlobalContext>,
    session_arc: &Arc<AMutex<crate::chat::types::ChatSession>>,
    payload: Value,
) -> Result<(), String> {
    {
        let mut session = session_arc.lock().await;
        if session.pending_mode_handoff.is_some()
            || session.pending_context_rebuild.is_some()
            || crate::chat::context_rebuild::compression_attempt_active(&session)
        {
            return Err("Another context operation is pending".into());
        }
        session.pending_mode_handoff = Some(payload.clone());
    }
    let result = finalize_handoff_inner(gcx, session_arc, payload.clone()).await;
    let mut session = session_arc.lock().await;
    if session.pending_mode_handoff.as_ref() == Some(&payload) {
        session.pending_mode_handoff = None;
    }
    result
}

#[cfg(test)]
mod deferred_tests {
    use super::*;

    #[test]
    fn report_transfer_preserves_archive_and_ledger_stop_wins() {
        let mut session = crate::chat::types::ChatSession::new("source".into());
        session.install_goal("agent", "Ship safely", true, Default::default());
        let active = session.messages.clone();
        let mut raw = vec![ChatMessage::new("user".into(), "immutable archive".into())];
        raw.extend(
            transition_report(&active, 1, "model".into(), "agent".into(), "agent".into()).unwrap(),
        );
        let archive = serde_json::to_vec(&raw).unwrap();
        let transfer =
            transition_goal(&raw, session.goal.as_ref(), "source", "target", "agent").unwrap();
        assert!(transfer.transferred());
        assert_eq!(
            serde_json::to_vec(&transfer.source_messages).unwrap(),
            archive
        );
        let report = transition_report(
            &transfer.target_messages,
            1,
            "model".into(),
            "agent".into(),
            "agent".into(),
        )
        .unwrap();
        assert_eq!(report.len(), 1);
        assert_eq!(report[0].role, "compression_report");
        assert!(refact_core::active_context::active_context(&report)
            .unwrap()
            .messages
            .iter()
            .any(|m| m.role == "goal"));
        session.goal_ledger_append(refact_chat_api::GoalLedgerOp::StatusChanged {
            from: refact_chat_api::GoalStatus::Active,
            to: refact_chat_api::GoalStatus::Stopped,
            reason: "user stop".into(),
        });
        let goal = transition_source_goal(&session);
        assert!(
            !transition_goal(&raw, goal.as_ref(), "source", "target", "agent")
                .unwrap()
                .transferred()
        );
    }

    #[test]
    fn completed_handoff_updates_actual_ack_and_emits_navigation_event() {
        let mut session = crate::chat::types::ChatSession::new("source".into());
        session.add_message(ChatMessage {
            role: "tool".into(),
            tool_call_id: "handoff".into(),
            content: ChatContent::SimpleText("queued".into()),
            ..Default::default()
        });
        let id = session.messages[0].message_id.clone();
        let mut events = session.subscribe();
        let (updated, message) = handoff_completion_messages(
            &session.messages,
            "handoff",
            &json!({
                "type": "handoff_to_mode", "status": "completed", "new_chat_id": "target"
            }),
        )
        .unwrap();
        publish_handoff_completion(&mut session, updated, message);
        assert_eq!(session.messages[0].message_id, id);
        let mut found = false;
        while let Ok(event) = events.try_recv() {
            let event: Value = serde_json::from_str(&event).unwrap();
            if event["type"] == "message_updated" {
                assert_eq!(event["message_id"], id);
                assert!(event["message"]["content"]
                    .as_str()
                    .unwrap()
                    .contains("completed"));
                found = true;
            }
        }
        assert!(
            found,
            "completion must emit MessageUpdated for navigation middleware"
        );
    }

    #[test]
    fn completed_handoff_emits_message_updated_for_existing_ack() {
        let mut session = crate::chat::types::ChatSession::new("source".into());
        session.add_message(ChatMessage {
            role: "tool".into(),
            tool_call_id: "handoff".into(),
            content: ChatContent::SimpleText("queued".into()),
            ..Default::default()
        });
        let id = session.messages[0].message_id.clone();
        let mut events = session.subscribe();
        let (messages, message) = handoff_completion_messages(
            &session.messages,
            "handoff",
            &json!({
                "type": "handoff_to_mode", "status": "completed", "new_chat_id": "target"
            }),
        )
        .unwrap();
        publish_handoff_completion(&mut session, messages, message);
        assert_eq!(session.messages[0].message_id, id);
        let mut found = false;
        while let Ok(event) = events.try_recv() {
            let event: Value = serde_json::from_str(&event).unwrap();
            if event["type"] == "message_updated" {
                assert_eq!(event["message_id"], id);
                assert!(event["message"]["content"]
                    .as_str()
                    .unwrap()
                    .contains("completed"));
                found = true;
            }
        }
        assert!(found, "completion must notify navigation middleware");
    }

    #[tokio::test]
    async fn handoff_queue_keeps_sibling_calls_and_allows_same_mode_restart() {
        use crate::call_validation::{ChatToolCall, ChatToolFunction};
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let app = crate::app_state::AppState::from_gcx(gcx.clone()).await;
        let mut session = crate::chat::types::ChatSession::new("queue-source".into());
        session.thread.mode = "agent".into();
        session.add_message(ChatMessage::new("user".into(), "Restart".into()));
        session.add_message(ChatMessage {
            role: "assistant".into(),
            tool_calls: Some(
                [("handoff", "handoff_to_mode"), ("sibling", "cat")]
                    .into_iter()
                    .map(|(id, name)| ChatToolCall {
                        id: id.into(),
                        index: None,
                        function: ChatToolFunction {
                            name: name.into(),
                            arguments: "{}".into(),
                        },
                        tool_type: "function".into(),
                        extra_content: None,
                        started_at_ms: None,
                        completed_at_ms: None,
                    })
                    .collect(),
            ),
            ..Default::default()
        });
        session.set_runtime_state(SessionState::ExecutingTools, None);
        let before = serde_json::to_value(&session.messages).unwrap();
        let session = Arc::new(AMutex::new(session));
        gcx.chat_sessions
            .write()
            .await
            .insert("queue-source".into(), session.clone());
        let ccx = Arc::new(AMutex::new(
            AtCommandsContext::new_from_app(
                app,
                4096,
                20,
                false,
                vec![],
                "queue-source".into(),
                None,
                "model".into(),
                None,
                None,
            )
            .await,
        ));
        let mut tool = ToolHandoffToMode {
            config_path: String::new(),
        };
        let args = HashMap::from([("target_mode".into(), json!("agent"))]);
        let (_, messages) = tool
            .tool_execute(ccx.clone(), &"handoff".into(), &args)
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(&session.lock().await.messages).unwrap(),
            before
        );
        let payload = session.lock().await.pending_mode_handoff.clone().unwrap();
        assert_eq!(payload["tool_call_id"], "handoff");
        let ContextEnum::ChatMessage(result) = &messages[0] else {
            panic!("expected acknowledgement")
        };
        let result: Value = serde_json::from_str(&result.content.content_text_only()).unwrap();
        assert_eq!(result["queued"], true);
        assert!(result.get("new_chat_id").is_none());
        assert!(tool
            .tool_execute(ccx, &"handoff".into(), &args)
            .await
            .is_err());
    }
}
