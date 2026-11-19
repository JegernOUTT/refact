use futures::future::BoxFuture;
use serde_json::{json, Value};

use super::chat::{carried_records, message_rows, messages_schema, range, ROLES};
use super::common::{failed, to_value};
use super::{EngineClass, EngineCtx, EngineError, EngineFn, EngineOutput};

pub fn functions() -> Vec<EngineFn> {
    vec![
        EngineFn {
            name: "chats.live",
            summary: "Chats loaded in the engine now with state, mode, kind and project; a chat busy right now is reported as busy. Child chats see only their own tree.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{"include_internal":{"type":"boolean","description":"Include internal and buddy chats; false by default."}}}),
            output: || Some(json!({"type":"object","required":["chats","total","truncated"],"properties":{"chats":{"type":"array","items":{"type":"object","properties":{"id":{"type":"string"},"title":{"type":"string"},"mode":{"type":"string"},"state":{"type":"string"},"paused":{"type":"boolean"},"kind":{"enum":["chat","subagent","buddy","internal"]},"project_root":{"type":"string"},"idle_ms":{"type":"integer"},"this_chat":{"type":"boolean"},"own_tree":{"type":"boolean"}}}},"total":{"type":"integer"},"truncated":{"type":"boolean"}}})),
            handler: live,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "chats.list",
            summary: "Saved chats, newest first, with title, mode, model, size, tokens, cost and task links; page with cursor.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{
                "limit":{"type":"integer","minimum":1,"maximum":200,"description":"Chats per page; 50 by default."},
                "cursor":{"type":"string","minLength":1,"description":"next_cursor of the previous page."},
                "displayable_only":{"type":"boolean","description":"Only chats the chat list shows; true by default."}
            }}),
            output: || Some(json!({"type":"object","required":["items","has_more"],"properties":{"items":{"type":"array","items":{"type":"object"}},"next_cursor":{"type":"string"},"has_more":{"type":"boolean"},"total_count":{"type":"integer"}}})),
            handler: list,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "chats.activity",
            summary: "Chats in this project that need attention: running, retrying, failed or with schedules.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{}}),
            output: || Some(json!({"type":"object","required":["chats"],"properties":{"chats":{"type":"array","items":{"type":"object","properties":{"id":{"type":"string"},"title":{"type":"string"},"updated_at_ms":{"type":"integer"},"session_state":{"type":"string"},"recovery_state":{"type":"string"},"schedule_count":{"type":"integer"},"paused_schedule_count":{"type":"integer"}}}}}})),
            handler: activity,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "chats.read",
            summary: "Another chat's messages by index range, with `digest`, an opaque pin of its whole history for chats.patch(expected_digest=); unlike get_trajectory_context, the messages' privacy records and the chat's derived zones travel into this run. Chats of this project only. Use chat.messages for this chat.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"required":["chat_id"],"properties":{
                "chat_id":{"type":"string","minLength":1},
                "start":{"type":"integer","minimum":0,"description":"First index; the last `limit` messages by default."},
                "limit":{"type":"integer","minimum":1,"maximum":200,"description":"Messages to return; 50 by default."},
                "roles":{"type":"array","minItems":1,"uniqueItems":true,"items":{"enum":ROLES}},
                "max_chars":{"type":"integer","minimum":100,"maximum":20000,"description":"Characters of text per message; 4000 by default."}
            }}),
            output: || Some(messages_schema(true)),
            handler: read,
            preview: None,
            timeout: None,
        },
    ]
    .into_iter()
    .chain(change_functions())
    .chain(trajectory_functions())
    .collect()
}

const LIVE_MAX: usize = 500;

async fn child_scope(ctx: &EngineCtx) -> (bool, String) {
    let context = ctx.ccx.lock().await;
    let child = context.background_agent_id.is_some() || context.subchat_depth > 0;
    let root = if context.root_chat_id.is_empty() {
        ctx.chat_id.clone()
    } else {
        context.root_chat_id.clone()
    };
    (child, root)
}

fn live<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let include_internal = args.get("include_internal").and_then(Value::as_bool) == Some(true);
        let (child, root) = child_scope(ctx).await;
        let loaded: Vec<(String, _)> = {
            let sessions = ctx.app.chat.sessions.read().await;
            sessions
                .iter()
                .map(|(id, s)| (id.clone(), s.clone()))
                .collect()
        };
        let now = crate::daemon::state::now_ms();
        let mut rows = Vec::new();
        let mut generating = 0;
        for (id, session) in loaded {
            let Ok(session) = session.try_lock() else {
                if !child {
                    rows.push(json!({"id":id,"state":"busy","this_chat":id == ctx.chat_id}));
                }
                continue;
            };
            let Some(chat) = crate::http::routers::v1::engine_observation::project(&session, now)
            else {
                continue;
            };
            let own_tree = session
                .thread
                .root_chat_id
                .as_deref()
                .unwrap_or(&session.chat_id)
                == root
                || session.chat_id == root;
            drop(session);
            if (!include_internal && matches!(chat.kind.as_str(), "internal" | "buddy"))
                || (child && !own_tree)
            {
                continue;
            }
            generating += usize::from(chat.session_state == "generating");
            let mut row = json!({"id":chat.id,"title":chat.title.chars().take(200).collect::<String>(),"mode":chat.mode,"state":chat.session_state,"paused":chat.paused,"kind":chat.kind,"idle_ms":now.saturating_sub(chat.updated_at_ms),"this_chat":chat.id == ctx.chat_id,"own_tree":own_tree});
            if let Some(project_root) = chat.project_root {
                row["project_root"] = json!(project_root);
            }
            rows.push(row);
        }
        rows.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
        let total = rows.len();
        rows.truncate(LIVE_MAX);
        Ok(EngineOutput::new(
            json!({"chats":rows,"total":total,"truncated":total > LIVE_MAX}),
            format!("{total} loaded chats ({generating} generating)"),
        ))
    })
}

fn list<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize;
        let cursor = args.get("cursor").and_then(Value::as_str).map(String::from);
        let displayable = args.get("displayable_only").and_then(Value::as_bool) != Some(false);
        let page = crate::chat::trajectories::list_trajectories_page(
            ctx.app.clone(),
            limit,
            cursor,
            displayable,
        )
        .await
        .map_err(|e| {
            if e.contains("cursor") {
                EngineError::Type(e)
            } else {
                EngineError::Failed(e)
            }
        })?;
        let (child, root) = child_scope(ctx).await;
        let items: Vec<Value> = page
            .items
            .iter()
            .filter(|m| !child || m.root_chat_id.as_deref().unwrap_or(&m.id) == root || m.id == root)
            .map(|m| {
                let mut row = json!({"id":m.id,"title":m.title.chars().take(200).collect::<String>(),"created_at":m.created_at,"updated_at":m.updated_at,"model":m.model,"mode":m.mode,"message_count":m.message_count,"total_tokens":m.total_tokens,"lines_added":m.total_lines_added,"lines_removed":m.total_lines_removed});
                for (key, value) in [
                    ("parent_id", &m.parent_id),
                    ("link_type", &m.link_type),
                    ("root_chat_id", &m.root_chat_id),
                    ("task_id", &m.task_id),
                    ("task_role", &m.task_role),
                    ("card_id", &m.card_id),
                    ("session_state", &m.session_state),
                ] {
                    if let Some(value) = value {
                        row[key] = json!(value);
                    }
                }
                if let Some(cost) = m.total_cost_usd {
                    row["total_cost_usd"] = json!(cost);
                }
                if let Some(worktree) = &m.worktree {
                    row["worktree_kind"] = json!(worktree.kind);
                }
                row
            })
            .collect();
        let mut data = json!({"items":items,"has_more":page.has_more});
        if !child {
            data["total_count"] = json!(page.total_count);
        }
        if let Some(cursor) = page.next_cursor {
            data["next_cursor"] = json!(cursor);
        }
        let text = if child {
            format!("{} chats of this chat's tree on this page", items.len())
        } else {
            format!("{} of {} chats", items.len(), page.total_count)
        };
        Ok(EngineOutput::new(data, text))
    })
}

fn activity<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let response = crate::http::routers::v1::chat_activity::activity(ctx.app.clone())
            .await
            .map_err(failed)?;
        let (child, root) = child_scope(ctx).await;
        let mut data = to_value(&response)?;
        if child {
            let loaded: Vec<_> = ctx
                .app
                .chat
                .sessions
                .read()
                .await
                .values()
                .cloned()
                .collect();
            let mut own: std::collections::HashSet<String> =
                [root.clone(), ctx.chat_id.clone()].into();
            for session in loaded {
                if let Ok(s) = session.try_lock() {
                    if s.thread.root_chat_id.as_deref() == Some(root.as_str()) {
                        own.insert(s.chat_id.clone());
                    }
                }
            }
            if let Some(chats) = data["chats"].as_array_mut() {
                chats.retain(|c| c["id"].as_str().is_some_and(|id| own.contains(id)));
            }
        }
        let count = data["chats"].as_array().map_or(0, Vec::len);
        Ok(EngineOutput::new(
            data,
            format!("{count} chats need attention"),
        ))
    })
}

fn read<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let chat_id = args["chat_id"].as_str().unwrap_or_default().to_string();
        if chat_id == ctx.chat_id {
            return Err(EngineError::Type(
                "this is this chat; use chat.messages for this chat".into(),
            ));
        }
        crate::chat::trajectories::validate_trajectory_id(&chat_id)
            .map_err(|e| EngineError::Type(format!("invalid chat_id: {}", e.message)))?;
        let (child, root) = child_scope(ctx).await;
        saved_file_refusal(ctx, &chat_id).await?;
        let file =
            crate::chat::trajectories::find_trajectory_path(ctx.app.gcx.clone(), &chat_id).await;
        let session = ctx.app.chat.sessions.read().await.get(&chat_id).cloned();
        let (title, messages, derived, chat_root, loaded, buddy, session_root) = match session {
            Some(session) => {
                let s = tokio::time::timeout(std::time::Duration::from_secs(2), session.lock())
                    .await
                    .map_err(|_| {
                        EngineError::Failed(format!("chat {chat_id} is busy; try again"))
                    })?;
                let derived: Vec<(std::path::PathBuf, crate::privacy::records::DerivedZone)> = s
                    .derived_privacy_zones
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .iter()
                    .map(|(p, z)| (p.clone(), z.clone()))
                    .collect();
                (
                    s.thread.title.clone(),
                    s.messages.clone(),
                    derived,
                    s.thread.root_chat_id.clone(),
                    true,
                    s.thread.buddy_meta.is_some() || s.thread.mode == "buddy",
                    crate::coordination::scope::session_source_root(&s),
                )
            }
            None => {
                let gcx = ctx.app.gcx.clone();
                if file.is_none() {
                    return Err(EngineError::Failed(format!("chat {chat_id} was not found")));
                }
                let loaded = crate::chat::trajectories::load_trajectory_for_chat(gcx, &chat_id)
                    .await
                    .ok_or_else(|| {
                        EngineError::Failed(format!("chat {chat_id} could not be loaded"))
                    })?;
                let buddy = loaded.thread.buddy_meta.is_some() || loaded.thread.mode == "buddy";
                (
                    loaded.thread.title,
                    loaded.messages,
                    Vec::new(),
                    loaded.thread.root_chat_id,
                    false,
                    buddy,
                    None,
                )
            }
        };
        if buddy {
            return Err(EngineError::Permission(format!(
                "chat {chat_id} is a buddy conversation; chats.read does not read those"
            )));
        }
        if child && chat_root.as_deref().unwrap_or(&chat_id) != root && chat_id != root {
            return Err(EngineError::Permission(format!(
                "chat {chat_id} is outside this chat's tree; a child chat reads only its own tree"
            )));
        }
        if same_project(ctx, file.as_deref(), session_root).await != Some(true) {
            return Err(EngineError::Permission(format!(
                "chat {chat_id} is not a chat of this project; chats.read reads chats of this project only"
            )));
        }
        let digest = history_gates::history_pin(&chat_id, file.as_deref(), &messages);
        let total = messages.len();
        let span = range(args, total)?;
        let slice = &messages[span.start..span.end];
        let (rows, selected) = message_rows(slice, span.start, args, false);
        let mut output = EngineOutput::new(Value::Null, "");
        output.privacy = carried_records(&selected, &format!("chat {chat_id}'s"))?;
        if !derived.is_empty() {
            let files: Vec<Value> = derived
                .iter()
                .map(|(path, zone)| json!({"path":path,"zone":zone.zone,"attribution":"observed"}))
                .collect();
            output.privacy.push(json!({"files":files}));
        }
        output.text = format!(
            "messages {}..{} of {total} from {}",
            span.start,
            span.end,
            title.chars().take(200).collect::<String>()
        );
        output.data = json!({"chat_id":chat_id,"title":title.chars().take(200).collect::<String>(),"total":total,"start":span.start,"loaded":loaded,"digest":digest,"messages":rows});
        Ok(output)
    })
}

// ---------------------------------------------------------------------------
// Changes of another chat. Each goes through that chat's own command queue (or delivery path),
// and refuses this chat, unknown chats, companions, background-agent children and, unless said
// otherwise, busy chats.

use std::sync::Arc;

use super::chat::{
    object_schema, plan_params, queue_command, thread_state, FLAG_ARGS, REASONING_ARGS,
    SAMPLING_ARGS, CONTEXT_ARGS,
};
use super::{Plan, Preview};
use crate::chat::types::ChatSession;

type SessionArc = Arc<tokio::sync::Mutex<ChatSession>>;

pub(super) fn change_functions() -> Vec<EngineFn> {
    vec![
        EngineFn {
            name: "chats.set_params",
            summary: "Change another idle chat of this project: model, mode, reasoning, sampling, context, flags, retry policy (same arguments as the chat.set_* functions) and the approval flags editing_tools, dangerous_commands and all_confirmations. Risky when it turns an approval flag on or widens the mode; for a chat outside this chat's tree the card names that chat, its tree, project and worktree and shows its mode and each approval flag before and after. The settings are compared again when the change is accepted. Refused for busy chats and for agent, runner, supervised, task (planner included), buddy and companion chats. Approval flags never affect engine functions.",
            class: EngineClass::Change,
            input: || {
                let mut schema = object_schema(&["model", "mode", "reasoning", "sampling", "context", "flags", "recovery"], &["chat_id"]);
                schema["properties"]["chat_id"] = json!({"type":"string","minLength":1});
                let flags = auto_approve_schema(false);
                for (key, value) in flags["properties"].as_object().into_iter().flatten() {
                    schema["properties"][key] = value.clone();
                }
                schema
            },
            output: || Some(json!({"type":"object","required":["chat_id","request_id","changed","opened"],"properties":{"chat_id":{"type":"string"},"request_id":{"type":"string"},"changed":{"type":"object"},"opened":{"type":"boolean","description":"True when this call opened the chat to change it."}}})),
            handler: set_params,
            preview: Some(set_params_preview),
            timeout: None,
        },
        EngineFn {
            name: "chats.set_title",
            summary: "Rename another chat (allowed while it is busy).",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"required":["chat_id","title"],"properties":{"chat_id":{"type":"string","minLength":1},"title":{"type":"string","minLength":1,"maxLength":200}}}),
            output: || Some(json!({"type":"object","required":["chat_id","old_title","new_title"],"properties":{"chat_id":{"type":"string"},"old_title":{"type":"string"},"new_title":{"type":"string"}}})),
            handler: set_title,
            preview: Some(set_title_preview),
            timeout: None,
        },
        EngineFn {
            name: "chats.send",
            summary: "Send a user message to another chat of this project: push=\"when_idle\" (default) lands after its turn and starts a new one, \"append\" at its next safe boundary inside a running turn, \"preempt\" now (interrupting a running turn). The text stays literal (no @ commands). Carries this run's privacy records and passes the egress check for that chat's model first. Reports whether it was delivered or queued; a queued message lands under that chat's settings at that time. Risky when that chat runs tools without asking or pursues an active goal.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"required":["chat_id","text"],"properties":{"chat_id":{"type":"string","minLength":1},"text":{"type":"string","minLength":1,"maxLength":20000},"push":{"enum":["when_idle","append","preempt"]}}}),
            output: || Some(json!({"type":"object","required":["chat_id","request_id","outcome","queued","opened"],"properties":{"chat_id":{"type":"string"},"request_id":{"type":"string"},"outcome":{"enum":["delivered","queued","duplicate","expired","superseded","sealed"]},"queued":{"type":"boolean"},"opened":{"type":"boolean"}}})),
            handler: send,
            preview: Some(send_preview),
            timeout: None,
        },
        EngineFn {
            name: "chats.abort",
            summary: "Stop the current generation of another chat; an idle chat has nothing to stop.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"required":["chat_id"],"properties":{"chat_id":{"type":"string","minLength":1}}}),
            output: || Some(json!({"type":"object","required":["chat_id","was_state","aborted"],"properties":{"chat_id":{"type":"string"},"was_state":{"type":"string"},"aborted":{"type":"boolean"}}})),
            handler: abort,
            preview: Some(abort_preview),
            timeout: None,
        },
        EngineFn {
            name: "chats.fork",
            summary: "Create a new chat that copies another chat's history up to a message (message_id from chats.read); the copied messages keep their privacy records.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"required":["chat_id","up_to_message_id"],"properties":{"chat_id":{"type":"string","minLength":1},"up_to_message_id":{"type":"string","minLength":1},"title":{"type":"string","minLength":1,"maxLength":200}}}),
            output: || Some(json!({"type":"object","required":["new_chat_id","source_chat_id","messages_copied"],"properties":{"new_chat_id":{"type":"string"},"source_chat_id":{"type":"string"},"messages_copied":{"type":"integer"}}})),
            handler: fork,
            preview: Some(fork_preview),
            timeout: Some(std::time::Duration::from_secs(60)),
        },
        EngineFn {
            name: "chats.compact_preview",
            summary: "Whether another chat's history can be compressed now, with message and token counts and the model chats.compact would use; changes nothing.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"required":["chat_id"],"properties":{"chat_id":{"type":"string","minLength":1},"model":{"type":"string","minLength":1}}}),
            output: || Some(json!({"type":"object","required":["eligible","source_messages","approximate_source_tokens"],"properties":{"eligible":{"type":"boolean"},"trajectory_version":{"type":"integer"},"resolved_model":{"type":"string"},"context_window":{"type":"integer"},"source_messages":{"type":"integer"},"approximate_source_tokens":{"type":"integer"},"reason":{"type":"string"}}})),
            handler: compact_check,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "chats.compact",
            summary: "Compress another idle chat's history with one model call (originals are kept in the report); chats.compact_preview shows what it would do.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"required":["chat_id"],"properties":{"chat_id":{"type":"string","minLength":1},"model":{"type":"string","minLength":1,"description":"Compression model."}}}),
            output: || Some(json!({"type":"object","properties":{"eligible":{"type":"boolean"},"applied":{"type":"boolean"},"trajectory_version":{"type":"integer"},"resolved_model":{"type":"string"},"before_message_count":{"type":"integer"},"after_message_count":{"type":"integer"},"before_approx_tokens":{"type":"integer"},"after_approx_tokens":{"type":"integer"},"reason":{"type":"string"}}})),
            handler: compact,
            preview: Some(compact_preview),
            timeout: Some(std::time::Duration::from_secs(300)),
        },
    ]
    .into_iter()
    .chain(risky_functions())
    .collect()
}

/// Opens another chat for a change, refusing the cases every `chats.*` change refuses.
async fn other_chat(
    ctx: &EngineCtx,
    chat_id: &str,
    require_idle: bool,
) -> Result<SessionArc, EngineError> {
    if chat_id == ctx.chat_id {
        return Err(EngineError::Type(
            "this is this chat; use chat.* for this chat".into(),
        ));
    }
    {
        let context = ctx.ccx.lock().await;
        if context.background_agent_id.is_some() || context.subchat_depth > 0 {
            return Err(EngineError::Permission(
                "chats.* changes run only in a root chat".into(),
            ));
        }
    }
    crate::chat::trajectories::validate_trajectory_id(chat_id)
        .map_err(|e| EngineError::Type(format!("invalid chat_id: {}", e.message)))?;
    if ctx
        .app
        .agents
        .find_agent_id_by_child_chat_id(chat_id)
        .await
        .is_some()
    {
        return Err(EngineError::Permission(format!(
            "chat {chat_id} is a background agent's chat; use the agents area"
        )));
    }
    let sessions = ctx.app.chat.sessions.clone();
    match crate::chat::session_runtime::restore_session_if_trajectory_exists(
        ctx.app.clone(),
        &sessions,
        chat_id,
    )
    .await
    {
        crate::chat::session_runtime::SessionRestore::Open => {}
        crate::chat::session_runtime::SessionRestore::NoTrajectory => {
            return Err(EngineError::Type(format!("chat {chat_id} does not exist")))
        }
        crate::chat::session_runtime::SessionRestore::Refused(error) => {
            return Err(EngineError::Failed(format!(
                "chat {chat_id} cannot be opened now: {error:?}"
            )))
        }
    }
    let session = sessions
        .read()
        .await
        .get(chat_id)
        .cloned()
        .ok_or_else(|| EngineError::Failed(format!("chat {chat_id} could not be opened")))?;
    {
        let s = session.lock().await;
        if s.closed {
            return Err(EngineError::Failed(format!("chat {chat_id} is closed")));
        }
        if crate::buddy::companion::is_companion(&s.thread) {
            return Err(EngineError::Permission(format!(
                "chat {chat_id} is a companion conversation"
            )));
        }
        if s.runner_lease.is_some() {
            return Err(EngineError::Permission(format!(
                "chat {chat_id} is owned by a background runner"
            )));
        }
        if require_idle && !s.turn_finished() {
            return Err(EngineError::Permission(format!(
                "chat {chat_id} is busy ({}); try again when it is idle",
                s.runtime.state
            )));
        }
    }
    Ok(session)
}

fn chat_scope(title: &str, id: &str) -> String {
    format!(
        "chat {} ({id})",
        super::mask::scrub_text(&title.chars().take(80).collect::<String>())
    )
}

fn chat_id_arg(args: &Value) -> String {
    args["chat_id"].as_str().unwrap_or_default().to_string()
}

#[derive(Clone)]
struct OtherParamsPlan {
    patch: Value,
    changed: serde_json::Map<String, Value>,
}

const SETTING_ARGS: &[&str] = &[
    "model",
    "mode",
    "recovery_mode",
    "max_retries",
    "delay_ms",
    "total_timeout_ms",
    "reset",
];

/// Whether any setting other than an approval flag is given.
fn settings_given(args: &Value) -> bool {
    SETTING_ARGS
        .iter()
        .chain(REASONING_ARGS)
        .chain(SAMPLING_ARGS)
        .chain(CONTEXT_ARGS)
        .chain(FLAG_ARGS)
        .any(|k| args.get(*k).is_some())
}

fn flags_given(args: &Value) -> bool {
    APPROVAL_ARGS
        .iter()
        .any(|(arg, _, _)| args.get(*arg).is_some())
}

fn set_params_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        if !settings_given(args) && !flags_given(args) {
            return Err(EngineError::Type("no settings given".into()));
        }
        other_params_preview(ctx, args, "change another chat's settings").await
    })
}

fn set_params<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(other_params_apply(ctx, args, "chats.set_params"))
}

/// What a parameter change of another chat is compared against at the answer and again under the
/// lock that accepts its command (A8): every setting of that chat, its model, mode and approval
/// flags among them, with the model as resolved and the provider its requests go to.
fn params_state(thread: &crate::chat::types::ThreadParams, resolver: &Resolver) -> Value {
    json!({
        "thread": thread_state(thread),
        "resolved_model": resolver.model(&thread.model),
        "provider": resolver.provider(&thread.model),
    })
}

/// The plan state of a parameter change: its settings and the file the chat is saved in.
fn params_plan_state(view: &TargetView, resolver: &Resolver) -> Value {
    json!({
        "params": params_state(&view.thread, resolver),
        "disk": view.disk.fingerprint(),
    })
}

fn turns_on_approvals(plan: &ParamsPlan) -> bool {
    APPROVAL_ARGS.iter().any(|(_, key, _)| {
        plan.changed
            .get(*key)
            .is_some_and(|change| change["new"] == json!(true))
    })
}

/// The card of a parameter change of another chat. A change that turns an approval flag on or
/// widens the mode of a chat outside this chat's tree is risky (D1); its card names that chat, its
/// tree, project and worktree and shows its mode and each approval flag before and after (A9).
async fn params_card(
    ctx: &EngineCtx,
    view: &TargetView,
    plan: &ParamsPlan,
    resolver: &Resolver,
) -> (Vec<refact_chat_api::PythonEngineItem>, Option<String>) {
    let (_, root) = child_scope(ctx).await;
    if !(turns_on_approvals(plan) || plan.mode_widens) || this_root(&root, &view.id, &view.thread) {
        let warning = (!plan.warnings.is_empty())
            .then(|| format!("{}: {}", view.thread.title, plan.warnings.join(" ")));
        return (plan.items.clone(), warning);
    }
    let title = view.thread.title.chars().take(80).collect::<String>();
    let tree = view
        .thread
        .root_chat_id
        .clone()
        .unwrap_or_else(|| view.id.clone());
    let project = this_project(ctx)
        .await
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "unknown".into());
    let worktree = view
        .thread
        .worktree
        .as_ref()
        .map(|worktree| worktree.root.display().to_string())
        .unwrap_or_else(|| "none (the project folder)".into());
    let mut items = vec![
        super::mask::item("chat", None, Some(json!(view.id))),
        super::mask::item("tree", None, Some(json!(tree))),
        super::mask::item("project", None, Some(json!(project))),
        super::mask::item("worktree", None, Some(json!(worktree))),
    ];
    if !plan.changed.contains_key("mode") {
        items.push(super::mask::item(
            "mode",
            Some(json!(view.thread.mode)),
            Some(json!(view.thread.mode)),
        ));
    }
    if !plan.changed.contains_key("model") {
        let model = resolver.model(&view.thread.model);
        items.push(super::mask::item(
            "model",
            Some(json!(model)),
            Some(json!(model)),
        ));
    }
    items.extend(
        plan.items
            .iter()
            .filter(|item| !APPROVAL_ARGS.iter().any(|(arg, _, _)| item.label == *arg))
            .cloned(),
    );
    for ((arg, key, _), old) in APPROVAL_ARGS.iter().zip(approval_flags(&view.thread)) {
        let new = plan.patch.get(*key).cloned().unwrap_or(json!(old));
        items.push(super::mask::item(arg, Some(json!(old)), Some(new)));
    }
    let detail = if plan.warnings.is_empty() {
        "the change gives it more tools or fewer confirmations.".to_string()
    } else {
        plan.warnings.join(" ")
    };
    let warning = format!(
        "{title} ({}) is outside this chat's tree (tree {tree}, project {project}): {detail}",
        view.id
    );
    (items, Some(warning))
}

/// The plan and card of a parameter change of another chat (`chats.set_params` and its
/// `chats.set_auto_approve` form), from a view that never opens the chat.
async fn other_params_preview(
    ctx: &EngineCtx,
    args: &Value,
    title: &str,
) -> Result<Preview, EngineError> {
    let id = chat_id_arg(args);
    let view = target_view(ctx, &id).await?;
    require_idle(&id, &view.idle)?;
    let mut plan = if settings_given(args) {
        let tokens = crate::chat::trajectory_ops::approx_token_count(&view.messages);
        plan_params(ctx, &view.thread, view.pinned_model, tokens, args).await?
    } else {
        ParamsPlan::default()
    };
    plan_flags(&view.thread, args, &mut plan);
    let scope = chat_scope(&view.thread.title, &id);
    if plan.changed.is_empty() {
        return Ok(Preview {
            title: title.into(),
            items: Vec::new(),
            scope: Some(scope),
            class: EngineClass::Change,
            warning: None,
            plan: Plan::nothing(),
        });
    }
    let resolver = Resolver::load(ctx).await;
    let (items, warning) = params_card(ctx, &view, &plan, &resolver).await;
    let warning = with_opening_note(&view, warning);
    Ok(Preview {
        title: title.into(),
        items,
        scope: Some(scope),
        class: if warning.is_some() {
            EngineClass::Risky
        } else {
            EngineClass::Change
        },
        warning,
        plan: Plan::new(
            params_plan_state(&view, &resolver),
            OtherParamsPlan {
                patch: Value::Object(plan.patch.clone()),
                changed: plan.changed.clone(),
            },
        ),
    })
}

/// Test hooks run on a chat under its lock at a named point: `<chat id>` right before a parameter
/// change compares the settings under the lock that accepts it, `regenerate:<chat id>` right
/// before a regenerate checks the chat under the lock that queues it, `create:<title>` right
/// before a new chat is saved, `delete:<chat id>` right before a loaded member of a delete is
/// checked again.
#[cfg(test)]
pub(super) static TEST_HOOKS: std::sync::Mutex<Vec<(String, fn(&mut ChatSession))>> =
    std::sync::Mutex::new(Vec::new());

#[cfg(test)]
fn test_hook(id: &str, s: &mut ChatSession) {
    let hook = TEST_HOOKS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .find(|(target, _)| target == id)
        .map(|(_, hook)| *hook);
    if let Some(hook) = hook {
        hook(s);
    }
}

/// Applies an approved parameter change of another chat: the plan is taken against a fresh view
/// before the chat is opened, and the settings are compared again under the lock that accepts the
/// `SetParams` command (A8).
async fn other_params_apply(
    ctx: &EngineCtx,
    args: &Value,
    function: &'static str,
) -> Result<EngineOutput, EngineError> {
    let id = chat_id_arg(args);
    let view = target_view(ctx, &id).await?;
    require_idle(&id, &view.idle)?;
    let resolver = Resolver::load(ctx).await;
    let plan: OtherParamsPlan = ctx.take_plan(function, &params_plan_state(&view, &resolver))?;
    let shown = params_state(&view.thread, &resolver);
    let (session, opened) = live_target(ctx, &view).await?;
    let patch = plan.patch.clone();
    let request_id = queue_built(ctx, &session, |s| {
        #[cfg(test)]
        test_hook(&id, s);
        live_gates(&view, s, opened, 1)?;
        let late = Resolver::current(&ctx.app.gcx).ok_or_else(|| {
            EngineError::Failed(format!(
                "{function}: model capabilities are being reloaded; nothing was applied to chat {id}, try again"
            ))
        })?;
        if params_state(&s.thread, &late) != shown {
            return Err(EngineError::Failed(format!(
                "{function}: chat {id} changed since the preview{} (its model, provider, mode, approval flags or other settings); nothing was applied, call it again to see a fresh preview",
                if opened { " (opening it changed it)" } else { "" }
            )));
        }
        Ok(refact_chat_api::ChatCommand::SetParams { patch })
    })
    .await?;
    let flags = APPROVAL_ARGS
        .iter()
        .any(|(_, key, _)| plan.changed.contains_key(*key));
    Ok(EngineOutput::new(
        json!({"chat_id":id,"request_id":request_id,"changed":plan.changed,"opened":opened}),
        format!(
            "{} settings of {} changed{}{}",
            plan.changed.len(),
            view.thread.title,
            if opened { " (the chat was opened)" } else { "" },
            if flags {
                format!("; {ENGINE_UNAFFECTED}")
            } else {
                String::new()
            }
        ),
    ))
}

#[derive(Clone)]
struct TitlePlan {
    title: String,
}

fn valid_title(args: &Value) -> Result<String, EngineError> {
    let title = args["title"]
        .as_str()
        .unwrap_or_default()
        .trim()
        .to_string();
    if title.is_empty() || title.contains(['\n', '\r']) {
        return Err(EngineError::Type("title must be one non-empty line".into()));
    }
    Ok(title)
}

fn set_title_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let title = valid_title(args)?;
        let id = chat_id_arg(args);
        let old = target_view(ctx, &id).await?.thread.title;
        let plan = if old == title {
            Plan::nothing()
        } else {
            Plan::new(
                json!({"title":old}),
                TitlePlan {
                    title: title.clone(),
                },
            )
        };
        Ok(Preview {
            title: "rename another chat".into(),
            items: vec![super::mask::item(
                "title",
                Some(json!(old)),
                Some(json!(title)),
            )],
            scope: Some(format!("chat {id}")),
            class: EngineClass::Change,
            warning: None,
            plan,
        })
    })
}

fn set_title<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let id = chat_id_arg(args);
        let session = other_chat(ctx, &id, false).await?;
        let old = session.lock().await.thread.title.clone();
        let plan: TitlePlan = ctx.take_plan("chats.set_title", &json!({"title":old}))?;
        queue_command(
            ctx,
            &session,
            refact_chat_api::ChatCommand::SetParams {
                patch: json!({"title":plan.title,"is_title_generated":false}),
            },
        )
        .await?;
        Ok(EngineOutput::new(
            json!({"chat_id":id,"old_title":old,"new_title":plan.title}),
            format!("chat {id} renamed (applies when that chat is idle)"),
        ))
    })
}

async fn send_clearance(ctx: &EngineCtx, model: &str) -> Result<(), EngineError> {
    let destination = crate::privacy::records::provider_destination(model);
    ctx.clear_for(&destination).await.map_err(|refusal| {
        EngineError::Permission(format!(
            "the message cannot go to that chat's model: {refusal}"
        ))
    })
}

fn push_arg(args: &Value) -> Result<refact_chat_api::PushMode, EngineError> {
    refact_chat_api::PushMode::parse(
        args.get("push")
            .and_then(Value::as_str)
            .unwrap_or("when_idle"),
    )
    .map_err(EngineError::Type)
}

fn approval_flags(thread: &crate::chat::types::ThreadParams) -> [bool; 3] {
    [
        thread.auto_approve_editing_tools,
        thread.auto_approve_dangerous_commands,
        thread.autonomous_no_confirm,
    ]
}

/// What a send is compared against when the answer comes and again under the lock that accepts
/// it (A8): the model as resolved and the provider its requests go to, the mode, each approval
/// flag, whether it pursues its goal and, for a preempt, the turn it interrupts.
fn send_state(view: &TargetView, resolver: &Resolver, push: refact_chat_api::PushMode) -> Value {
    let busy = push == refact_chat_api::PushMode::Preempt && turn_running(&view.idle);
    json!({
        "provider": resolver.provider(&view.thread.model),
        "resolved_model": resolver.model(&view.thread.model),
        "model": view.thread.model,
        "mode": view.thread.mode,
        "flags": approval_flags(&view.thread),
        "goal_active": view.goal_active,
        "push": push,
        "busy": busy,
        "turn": if busy { view.turn } else { None },
    })
}

fn turn_running(idle: &IdleFacts) -> bool {
    !idle.turn_finished || idle.streaming
}

fn send_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let id = chat_id_arg(args);
        let text = args["text"].as_str().unwrap_or_default();
        crate::chat::content::validate_content_with_attachments(&json!(text), &[])
            .map_err(EngineError::Type)?;
        let push = push_arg(args)?;
        let view = target_view(ctx, &id).await?;
        let resolver = Resolver::load(ctx).await;
        send_clearance(ctx, &resolver.model(&view.thread.model)).await?;
        let (mut items, warning) = turn_card(ctx, &view).await;
        let warning = with_opening_note(&view, warning);
        items.insert(0, super::mask::item("message", None, Some(json!(text))));
        items.push(super::mask::item(
            "delivery",
            None,
            Some(json!(match push {
                refact_chat_api::PushMode::WhenIdle => "when that chat is idle; it starts a turn",
                refact_chat_api::PushMode::Append =>
                    "at that chat's next safe boundary, inside a running turn",
                refact_chat_api::PushMode::Preempt => "now",
            })),
        ));
        if push == refact_chat_api::PushMode::Preempt && turn_running(&view.idle) {
            items.push(super::mask::item(
                "running turn",
                Some(json!(view.idle.state)),
                Some(json!("interrupted: its unfinished answer is discarded and its open tool calls are closed")),
            ));
        }
        Ok(Preview {
            title: "send a message to another chat".into(),
            items,
            scope: Some(chat_scope(&view.thread.title, &id)),
            class: if warning.is_some() {
                EngineClass::Risky
            } else {
                EngineClass::Change
            },
            warning,
            plan: Plan::new(send_state(&view, &resolver, push), ()),
        })
    })
}

fn send<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let id = chat_id_arg(args);
        let text = args["text"].as_str().unwrap_or_default().to_string();
        let push = push_arg(args)?;
        let view = target_view(ctx, &id).await?;
        let resolver = Resolver::load(ctx).await;
        let shown = send_state(&view, &resolver, push);
        ctx.take_plan::<()>("chats.send", &shown)?;
        send_clearance(ctx, &resolver.model(&view.thread.model)).await?;
        let mut message = crate::call_validation::ChatMessage::new("user".into(), text);
        crate::privacy::records::merge_records(&mut message, run_records(ctx).await?);
        let delivery =
            refact_chat_api::PendingDelivery::new(vec![message], push, "engine.chats.send", true);
        let request_id = delivery.id.clone();
        let (_, opened) = live_target(ctx, &view).await?;
        let (target, nominal, mode, flags) = (
            id.clone(),
            view.thread.model.clone(),
            view.thread.mode.clone(),
            approval_flags(&view.thread),
        );
        let busy = shown["busy"] == json!(true);
        let goal_active = view.goal_active;
        let (resolved, provider, turn) = (
            shown["resolved_model"].clone(),
            shown["provider"].clone(),
            shown["turn"].clone(),
        );
        let gcx = ctx.app.gcx.clone();
        let admit = move |s: &ChatSession| -> Result<(), String> {
            if let Some(refusal) = live_owner_refusal(&target, s) {
                return Err(refusal_text(refusal));
            }
            let late = Resolver::current(&gcx).ok_or_else(|| {
                format!("chats.send: model capabilities are being reloaded; nothing was sent to chat {target}, try again")
            })?;
            let running =
                !s.turn_finished() || !crate::chat::context_rebuild::explicit_context_ready(s);
            let now_turn = json!(s.turn_origin.as_ref().map(|origin| origin.at_ms));
            if s.thread.model != nominal
                || json!(late.model(&s.thread.model)) != resolved
                || json!(late.provider(&s.thread.model)) != provider
                || s.thread.mode != mode
                || approval_flags(&s.thread) != flags
                || goal_pursued(s.goal.as_ref()) != goal_active
                || (push == refact_chat_api::PushMode::Preempt
                    && (running != busy || (busy && now_turn != turn)))
            {
                return Err(format!(
                    "chats.send: chat {target} changed since the preview (model, provider, mode, approval flags, goal or its running turn); nothing was sent, call it again to see a fresh preview"
                ));
            }
            Ok(())
        };
        let opened_note = if opened { " (the chat was opened)" } else { "" };
        let receipt =
            crate::chat::delivery::deliver_to_chat_admitted(ctx.app.clone(), &id, delivery, &admit)
                .await
                .map_err(|e| EngineError::Failed(format!("{e}{opened_note}")))?;
        let outcome = receipt.outcome;
        let text = match outcome {
            refact_chat_api::DeliveryOutcome::Delivered => {
                format!("message delivered to {}{opened_note}", view.thread.title)
            }
            refact_chat_api::DeliveryOutcome::Queued => format!(
                "message queued in {}; it lands {}{opened_note}",
                view.thread.title,
                if push == refact_chat_api::PushMode::WhenIdle {
                    "when that chat is idle, under its settings at that time"
                } else {
                    "at that chat's next safe boundary"
                }
            ),
            other => format!(
                "message not delivered to {}: {}{opened_note}",
                view.thread.title,
                serde_json::to_value(other)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_default()
            ),
        };
        Ok(EngineOutput::new(
            json!({"chat_id":id,"request_id":request_id,"outcome":outcome,"queued":outcome == refact_chat_api::DeliveryOutcome::Queued,"opened":opened}),
            text,
        ))
    })
}

/// The privacy records of this chat and this run, which travel with text the script sends elsewhere.
pub(super) async fn run_records(
    ctx: &EngineCtx,
) -> Result<Vec<refact_privacy::FileRecord>, EngineError> {
    let mut carrier = crate::call_validation::ChatMessage::default();
    let history = ctx.ccx.lock().await.messages.clone();
    crate::privacy::records::carry_records_into(&mut carrier, &history)
        .map_err(|e| EngineError::Failed(e.to_string()))?;
    let (taint, reads) = {
        let run = ctx.run_privacy.lock().unwrap_or_else(|e| e.into_inner());
        (run.taint.clone(), run.reads.clone())
    };
    crate::python_tool::run_output::merge_taint(&mut carrier, &taint)
        .map_err(EngineError::Failed)?;
    let derived = ctx.ccx.lock().await.derived_privacy_zones.clone();
    let direct = crate::privacy::records::observed_file_records_with_derived(
        &ctx.app.gcx,
        reads.iter().map(std::path::PathBuf::from),
        &derived,
    )
    .map_err(EngineError::Failed)?;
    crate::privacy::records::merge_records(&mut carrier, direct);
    match carrier.extra.get("privacy") {
        Some(privacy) => {
            let records: refact_privacy::PrivacyRecord =
                serde_json::from_value(privacy.clone()).map_err(failed)?;
            Ok(records.files)
        }
        None => Ok(Vec::new()),
    }
}

fn abort_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let id = chat_id_arg(args);
        let view = target_view(ctx, &id).await?;
        let (state, busy, title) = (
            view.idle.state.clone(),
            !view.idle.turn_finished,
            view.thread.title.clone(),
        );
        Ok(Preview {
            title: "stop another chat".into(),
            items: vec![super::mask::item(
                "state",
                Some(json!(state)),
                Some(json!("stopped")),
            )],
            scope: Some(chat_scope(&title, &id)),
            class: EngineClass::Change,
            warning: None,
            plan: if busy {
                Plan::new(json!({"state":state}), ())
            } else {
                Plan::nothing()
            },
        })
    })
}

fn abort<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let id = chat_id_arg(args);
        let session = other_chat(ctx, &id, false).await?;
        let request_id = format!("engine-{}", uuid::Uuid::new_v4());
        let (was, save) = {
            let mut s = session.lock().await;
            let was = s.runtime.state.to_string();
            ctx.take_plan::<()>("chats.abort", &json!({"state":was}))?;
            let _guard = crate::chat::NativeMutationGuard::enter_locked(&session, &mut s)
                .map_err(EngineError::Permission)?;
            (was, crate::chat::abort_chat_locked(&mut s, &request_id))
        };
        if save {
            crate::chat::trajectories::maybe_save_trajectory_with_intent(
                ctx.app.clone(),
                session.clone(),
                crate::chat::types::TrajectoryCommitIntent::Required,
            )
            .await;
        }
        Ok(EngineOutput::new(
            json!({"chat_id":id,"was_state":was,"aborted":true}),
            format!("chat {id} stopped (was {was})"),
        ))
    })
}

const FORK_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// The error of a refused `Ack` for `request_id` in a serialized chat event.
fn rejected_ack(event: &str, request_id: &str) -> Option<String> {
    let event: Value = serde_json::from_str(event).ok()?;
    (event["type"] == "ack"
        && event["client_request_id"] == request_id
        && event["accepted"] == false)
        .then(|| {
            event["result"]["error"]
                .as_str()
                .unwrap_or("the chat refused the command")
                .to_string()
        })
}

/// Removes a chat this call created and that stayed empty.
async fn discard_new_chat(ctx: &EngineCtx, new_id: &str, target: &SessionArc) {
    let removed = {
        let mut sessions = ctx.app.chat.sessions.write().await;
        match sessions.get(new_id) {
            Some(current) if Arc::ptr_eq(current, target) => sessions.remove(new_id),
            _ => None,
        }
    };
    if let Some(session) = removed {
        let mut s = session.lock().await;
        s.close_event_channel();
        s.queue_notify.notify_waiters();
    }
    if crate::chat::trajectories::find_trajectory_path(ctx.app.gcx.clone(), new_id)
        .await
        .is_some()
    {
        if let Err(e) = crate::chat::trajectories::delete_trajectory(&ctx.app, new_id).await {
            tracing::warn!(
                "engine chats.fork: the empty chat {new_id} was not deleted: {}",
                e.message
            );
        }
    }
}

/// Messages of the source up to and including `message_id`, or an error naming the source.
fn fork_source(
    id: &str,
    messages: &[crate::call_validation::ChatMessage],
    message_id: &str,
) -> Result<Vec<crate::call_validation::ChatMessage>, EngineError> {
    let end = messages
        .iter()
        .position(|m| m.message_id == message_id)
        .ok_or_else(|| {
            EngineError::Type(format!(
                "message {message_id} is not in chat {id}; see chats.read"
            ))
        })?;
    Ok(messages[..=end].to_vec())
}

/// What a fork or a compaction of another chat pins: its history and, while it is loaded, its
/// version.
fn history_state(view: &TargetView) -> Value {
    json!({"digest":view.digest(),"version":view.version})
}

fn fork_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let id = chat_id_arg(args);
        let message_id = args["up_to_message_id"].as_str().unwrap_or_default();
        let view = target_view(ctx, &id).await?;
        let messages = fork_source(&id, &view.messages, message_id)?;
        crate::privacy::records::records_to_carry(&messages).map_err(|e| {
            EngineError::Failed(format!(
                "the source's privacy records could not be read ({}); nothing was forked",
                e.message
            ))
        })?;
        Ok(Preview {
            title: "fork another chat".into(),
            items: vec![
                super::mask::item(
                    "source",
                    None,
                    Some(json!(super::mask::scrub_text(&view.thread.title))),
                ),
                super::mask::item(
                    "copied messages",
                    None,
                    Some(json!(messages
                        .iter()
                        .filter(|m| crate::chat::queue::is_allowed_for_branch(m))
                        .count())),
                ),
            ],
            scope: Some("new chat".into()),
            class: EngineClass::Change,
            warning: None,
            plan: Plan::new(
                json!({"source":history_state(&view),"message":message_id}),
                (),
            ),
        })
    })
}

fn fork<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let id = chat_id_arg(args);
        let message_id = args["up_to_message_id"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let view = target_view(ctx, &id).await?;
        ctx.take_plan::<()>(
            "chats.fork",
            &json!({"source":history_state(&view),"message":message_id}),
        )?;
        let source = other_chat(ctx, &id, false).await?;
        let messages = {
            let s = source.lock().await;
            if history_gates::history_pin(&id, view.file.as_deref(), &s.messages) != view.digest() {
                return Err(EngineError::Failed(format!(
                    "chats.fork: chat {id} changed since the preview; nothing was forked, call it again to see a fresh preview"
                )));
            }
            fork_source(&id, &s.messages, &message_id)?
        };
        let records = crate::privacy::records::records_to_carry(&messages).map_err(|e| {
            EngineError::Failed(format!(
                "the source's privacy records could not be read ({}); nothing was forked",
                e.message
            ))
        })?;
        let new_id = uuid::Uuid::new_v4().to_string();
        let target = crate::chat::get_or_create_session_with_trajectory(
            ctx.app.clone(),
            &ctx.app.chat.sessions,
            &new_id,
        )
        .await
        .map_err(|e| EngineError::Failed(format!("the new chat could not be created: {e:?}")))?;
        let mut events = target.lock().await.subscribe();
        let branch_request = queue_command(
            ctx,
            &target,
            refact_chat_api::ChatCommand::BranchFromChat {
                source_chat_id: id.clone(),
                up_to_message_id: message_id,
            },
        )
        .await;
        let branch_request = match branch_request {
            Ok(request_id) => request_id,
            Err(error) => {
                discard_new_chat(ctx, &new_id, &target).await;
                return Err(error);
            }
        };
        let outcome = tokio::time::timeout(FORK_WAIT, async {
            loop {
                {
                    let s = target.lock().await;
                    if s.thread.parent_id.as_deref() == Some(id.as_str()) && !s.messages.is_empty()
                    {
                        return Ok(s.messages.len());
                    }
                }
                match tokio::time::timeout(std::time::Duration::from_millis(50), events.recv())
                    .await
                {
                    Ok(Ok(event)) => {
                        if let Some(reason) = rejected_ack(&event, &branch_request) {
                            return Err(reason);
                        }
                    }
                    Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) | Err(_) => {}
                    Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                        return Err("the new chat was closed".to_string());
                    }
                }
            }
        })
        .await;
        let copied = match outcome {
            Ok(Ok(copied)) => copied,
            Ok(Err(reason)) => {
                discard_new_chat(ctx, &new_id, &target).await;
                return Err(EngineError::Failed(format!(
                    "the fork of {id} was refused: {reason}; nothing was created"
                )));
            }
            Err(_) => {
                return Err(EngineError::Failed(format!(
                    "the fork of {id} did not finish within {} s; chat {new_id} was created and may still receive the copy, check it with chats.read",
                    FORK_WAIT.as_secs()
                )))
            }
        };
        if !records.is_empty() {
            {
                let mut s = target.lock().await;
                if let Some(first) = s.messages.first_mut() {
                    crate::privacy::records::merge_records(first, records);
                }
                s.increment_version();
            }
            crate::chat::trajectories::maybe_save_trajectory_with_intent(
                ctx.app.clone(),
                target.clone(),
                crate::chat::types::TrajectoryCommitIntent::Required,
            )
            .await;
        }
        if let Some(title) = args.get("title").and_then(Value::as_str) {
            queue_command(
                ctx,
                &target,
                refact_chat_api::ChatCommand::SetParams {
                    patch: json!({"title":title,"is_title_generated":false}),
                },
            )
            .await?;
        }
        Ok(EngineOutput::new(
            json!({"new_chat_id":new_id,"source_chat_id":id,"messages_copied":copied}),
            format!("forked {id} into {new_id} ({copied} messages)"),
        ))
    })
}

async fn compaction_preview(
    ctx: &EngineCtx,
    session: &SessionArc,
    model: Option<&str>,
) -> crate::chat::context_rebuild::ManualCompressionPreview {
    let (state, active, messages, live_ids, thread, version) = {
        let s = session.lock().await;
        (
            if crate::chat::context_rebuild::explicit_context_ready(&s) {
                s.runtime.state
            } else {
                refact_chat_api::SessionState::Generating
            },
            crate::chat::context_rebuild::compression_attempt_active(&s)
                || s.pending_mode_handoff.is_some()
                || s.pending_context_rebuild.is_some(),
            s.messages.clone(),
            crate::chat::context_rebuild::live_tool_call_ids(&s),
            s.thread.clone(),
            s.trajectory_version,
        )
    };
    let mut preview = crate::chat::context_rebuild::preview_manual_context_rebuild(
        ctx.app.gcx.clone(),
        state,
        active,
        &messages,
        &live_ids,
        &thread,
        model,
    )
    .await;
    if preview.eligible {
        preview.trajectory_version = Some(version);
    }
    preview
}

/// The compaction preview of a saved chat that is not loaded, read from its file without opening it.
async fn persisted_compaction_preview(
    ctx: &EngineCtx,
    id: &str,
    model: Option<&str>,
) -> Result<crate::chat::context_rebuild::ManualCompressionPreview, EngineError> {
    if id == ctx.chat_id {
        return Err(EngineError::Type(
            "this is this chat; use chat.* for this chat".into(),
        ));
    }
    if child_scope(ctx).await.0 {
        return Err(EngineError::Permission(
            "chats.* changes run only in a root chat".into(),
        ));
    }
    crate::chat::trajectories::validate_trajectory_id(id)
        .map_err(|e| EngineError::Type(format!("invalid chat_id: {}", e.message)))?;
    if ctx
        .app
        .agents
        .find_agent_id_by_child_chat_id(id)
        .await
        .is_some()
    {
        return Err(EngineError::Permission(format!(
            "chat {id} is a background agent's chat; use the agents area"
        )));
    }
    let loaded = crate::chat::trajectories::load_trajectory_for_chat(ctx.app.gcx.clone(), id)
        .await
        .ok_or_else(|| EngineError::Type(format!("chat {id} does not exist")))?;
    if crate::buddy::companion::is_companion(&loaded.thread) {
        return Err(EngineError::Permission(format!(
            "chat {id} is a companion conversation"
        )));
    }
    Ok(
        crate::chat::context_rebuild::preview_manual_context_rebuild(
            ctx.app.gcx.clone(),
            refact_chat_api::SessionState::Idle,
            false,
            &loaded.messages,
            &std::collections::HashSet::new(),
            &loaded.thread,
            model,
        )
        .await,
    )
}

fn compact_check<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let id = chat_id_arg(args);
        let model = args.get("model").and_then(Value::as_str);
        let loaded = ctx.app.chat.sessions.read().await.get(&id).cloned();
        let preview = match loaded {
            Some(_) => {
                let session = other_chat(ctx, &id, false).await?;
                compaction_preview(ctx, &session, model).await
            }
            None => persisted_compaction_preview(ctx, &id, model).await?,
        };
        let text = format!(
            "{} messages, about {} tokens; eligible: {}",
            preview.source_messages, preview.approximate_source_tokens, preview.eligible
        );
        Ok(EngineOutput::new(to_value(&preview)?, text))
    })
}

fn compact_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let id = chat_id_arg(args);
        let view = target_view(ctx, &id).await?;
        if !view.idle.turn_finished {
            return Err(EngineError::Permission(format!(
                "chat {id} is busy ({}); try again when it is idle",
                view.idle.state
            )));
        }
        let model = args.get("model").and_then(Value::as_str);
        let session = ctx.app.chat.sessions.read().await.get(&id).cloned();
        let preview = match session {
            Some(session) if view.loaded => compaction_preview(ctx, &session, model).await,
            _ => {
                crate::chat::context_rebuild::preview_manual_context_rebuild(
                    ctx.app.gcx.clone(),
                    refact_chat_api::SessionState::Idle,
                    false,
                    &view.messages,
                    &std::collections::HashSet::new(),
                    &view.thread,
                    model,
                )
                .await
            }
        };
        if !preview.eligible {
            return Err(EngineError::Failed(format!(
                "chat {id} cannot be compressed now: {}",
                preview
                    .reason
                    .clone()
                    .unwrap_or_else(|| "not eligible".into())
            )));
        }
        let resolved = preview.resolved_model.clone().unwrap_or_default();
        Ok(Preview {
            title: "compress another chat's history".into(),
            items: vec![
                super::mask::item(
                    "messages",
                    Some(json!(preview.source_messages)),
                    Some(json!("one summary plus the recent messages")),
                ),
                super::mask::item(
                    "tokens",
                    Some(json!(preview.approximate_source_tokens)),
                    Some(json!("fewer")),
                ),
                super::mask::item(
                    "cost",
                    None,
                    Some(json!(format!("one model call on {resolved}"))),
                ),
            ],
            scope: Some(format!("chat {id}")),
            class: EngineClass::Change,
            warning: None,
            plan: Plan::new(history_state(&view), ()),
        })
    })
}

fn compact<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let id = chat_id_arg(args);
        let view = target_view(ctx, &id).await?;
        ctx.take_plan::<()>("chats.compact", &history_state(&view))?;
        let session = other_chat(ctx, &id, true).await?;
        let model = args.get("model").and_then(Value::as_str);
        let version = {
            let s = session.lock().await;
            if history_gates::history_pin(&id, view.file.as_deref(), &s.messages) != view.digest() {
                return Err(EngineError::Failed(format!(
                    "chats.compact: chat {id} changed since the preview; nothing was compressed, call it again to see a fresh preview"
                )));
            }
            s.trajectory_version
        };
        let result = crate::chat::context_rebuild::apply_manual_context_rebuild(
            ctx.app.gcx.clone(),
            &session,
            model,
            Some(version),
        )
        .await;
        if result.applied {
            crate::chat::trajectories::try_save_trajectory_with_intent(
                ctx.app.clone(),
                session.clone(),
                crate::chat::types::TrajectoryCommitIntent::Required,
            )
            .await
            .map_err(EngineError::Failed)?;
        } else {
            return Err(EngineError::Failed(format!(
                "chat {id} was not compressed: {}",
                result
                    .reason
                    .clone()
                    .unwrap_or_else(|| "not applied".into())
            )));
        }
        let text = format!(
            "{} -> {} messages, {} -> {} tokens",
            result.before_message_count,
            result.after_message_count,
            result.before_approx_tokens,
            result.after_approx_tokens
        );
        Ok(EngineOutput::new(to_value(&result)?, text))
    })
}

// ---------------------------------------------------------------------------
// Risky and destructive changes of another chat.

use super::chat::{
    auto_approve_schema, plan_flags, queue_built, ParamsPlan, APPROVAL_ARGS, ENGINE_UNAFFECTED,
};

const MAX_SHOWN_CHARS: usize = 300;

fn risky_functions() -> Vec<EngineFn> {
    vec![
        EngineFn {
            name: "chats.set_auto_approve",
            summary: "The approval-flag part of chats.set_params: turn automatic approvals of another idle chat on or off (editing tools, dangerous commands, all confirmations). Risky when a flag is turned on; for a chat outside this chat's tree the card names that chat, its tree, project and worktree. These flags never affect engine functions.",
            class: EngineClass::Change,
            input: || auto_approve_schema(true),
            output: || Some(json!({"type":"object","required":["chat_id","request_id","changed","opened"],"properties":{"chat_id":{"type":"string"},"request_id":{"type":"string"},"changed":{"type":"object"},"opened":{"type":"boolean","description":"True when this call opened the chat to change it."}}})),
            handler: set_auto_approve,
            preview: Some(set_auto_approve_preview),
            timeout: None,
        },
        EngineFn {
            name: "chats.clear_privacy_records",
            summary: "Forget the privacy records of chosen paths in another idle loaded chat so its content may reach more destinations; records in blocked zones are always kept.",
            class: EngineClass::Risky,
            input: || json!({"type":"object","additionalProperties":false,"required":["chat_id","paths"],"properties":{"chat_id":{"type":"string","minLength":1},"paths":{"type":"array","minItems":1,"maxItems":100,"uniqueItems":true,"items":{"type":"string","minLength":1}}}}),
            output: || Some(json!({"type":"object","required":["cleared","remaining","persisted"],"properties":{"cleared":{"type":"integer"},"remaining":{"type":"integer"},"persisted":{"type":"boolean"}}})),
            handler: clear_records,
            preview: Some(clear_records_preview),
            timeout: None,
        },
        EngineFn {
            name: "chats.retry_from",
            summary: "Cut another idle chat's history at a user message (index from chats.read) and send a new user message from there, which starts a turn that cannot raise engine change cards. The card shows where it cuts (not that message's text) and that chat's mode, model, approval flags and goal. Refused when this run carries privacy records, because the new message cannot carry them; passes the egress check for that chat's model first. User messages only: for other roles use chats.patch. Chats of this project; refused for busy chats and for agent, runner, supervised, task, buddy and companion chats; a saved chat is opened only after approval.",
            class: EngineClass::Destructive,
            input: || json!({"type":"object","additionalProperties":false,"required":["chat_id","index","text"],"properties":{"chat_id":{"type":"string","minLength":1},"index":{"type":"integer","minimum":0},"text":{"type":"string","minLength":1,"maxLength":20000}}}),
            output: || Some(json!({"type":"object","required":["chat_id","request_id","messages_removed","opened"],"properties":{"chat_id":{"type":"string"},"request_id":{"type":"string"},"messages_removed":{"type":"integer"},"opened":{"type":"boolean","description":"True when this call opened the chat to change it."}}})),
            handler: retry_from,
            preview: Some(retry_from_preview),
            timeout: None,
        },
        EngineFn {
            name: "chats.edit_message",
            summary: "Replace the text of one user message in another idle chat (message_id from chats.read), optionally regenerating from it. Carries this run's privacy records and passes the egress check for that chat's model first; the records of the old text shown on the card travel into this run. User messages only: for other roles use chats.patch. Chats of this project; refused for busy chats and for agent, runner, supervised, task, buddy and companion chats; a saved chat is opened only after approval.",
            class: EngineClass::Destructive,
            input: || json!({"type":"object","additionalProperties":false,"required":["chat_id","message_id","text"],"properties":{"chat_id":{"type":"string","minLength":1},"message_id":{"type":"string","minLength":1},"text":{"type":"string","minLength":1,"maxLength":20000},"regenerate":{"type":"boolean","description":"Delete the later messages and generate a new answer; false by default."}}}),
            output: || Some(json!({"type":"object","required":["chat_id","request_id","messages_removed","opened"],"properties":{"chat_id":{"type":"string"},"request_id":{"type":"string"},"messages_removed":{"type":"integer"},"opened":{"type":"boolean","description":"True when this call opened the chat to change it."}}})),
            handler: edit_message,
            preview: Some(edit_message_preview),
            timeout: None,
        },
        EngineFn {
            name: "chats.remove_message",
            summary: "Delete one user message from another idle chat (message_id from chats.read), optionally regenerating from there; the records of the text shown on the card travel into this run. User messages only: for other roles use chats.patch. Chats of this project; refused for busy chats and for agent, runner, supervised, task, buddy and companion chats; a saved chat is opened only after approval.",
            class: EngineClass::Destructive,
            input: || json!({"type":"object","additionalProperties":false,"required":["chat_id","message_id"],"properties":{"chat_id":{"type":"string","minLength":1},"message_id":{"type":"string","minLength":1},"regenerate":{"type":"boolean","description":"Also delete the later messages and generate a new answer; false by default."}}}),
            output: || Some(json!({"type":"object","required":["chat_id","request_id","messages_removed","opened"],"properties":{"chat_id":{"type":"string"},"request_id":{"type":"string"},"messages_removed":{"type":"integer"},"opened":{"type":"boolean","description":"True when this call opened the chat to change it."}}})),
            handler: remove_message,
            preview: Some(remove_message_preview),
            timeout: None,
        },
        EngineFn {
            name: "chats.delete",
            summary: "Delete another chat of this project and its Python workspace; with cascade=True also every chat below it, children first (at most 50). A loaded chat must be idle and not open in a viewer (close it first); it is closed in the engine, then deleted. Each chat is checked again just before it is deleted, and nothing opens it until it is gone; the call stops at the first one that changed or no longer passes, and its result is then ok false with every chat listed as deleted or kept with the reason. Refused for this chat and its ancestors, chats with children without cascade, and agent, runner, supervised, task, buddy, companion and worktree chats and chats saved twice.",
            class: EngineClass::Destructive,
            input: || json!({"type":"object","additionalProperties":false,"required":["chat_id"],"properties":{"chat_id":{"type":"string","minLength":1},"cascade":{"type":"boolean","description":"Also delete every chat below it, children first."}}}),
            output: || Some(json!({"type":"object","required":["chat_id","deleted"],"properties":{"chat_id":{"type":"string"},"deleted":{"type":"boolean"},"chats":{"type":"array","description":"With cascade: the deleted chats, children first. When the delete stopped (ok false): every chat as {id, deleted, reason, workspace_deleted}."},"remaining_copy":{"type":"boolean"}}})),
            handler: delete,
            preview: Some(delete_preview),
            timeout: Some(std::time::Duration::from_secs(60)),
        },
    ]
}

fn shown_text(text: &str) -> String {
    let mut shown: String = text.chars().take(MAX_SHOWN_CHARS).collect();
    if text.chars().count() > MAX_SHOWN_CHARS {
        shown.push('…');
    }
    shown
}

fn this_root(ctx_root: &str, chat_id: &str, thread: &crate::chat::types::ThreadParams) -> bool {
    thread.root_chat_id.as_deref().unwrap_or(chat_id) == ctx_root || chat_id == ctx_root
}

fn set_auto_approve_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        if !flags_given(args) {
            return Err(EngineError::Type(
                "no flags given; give at least one of editing_tools, dangerous_commands, all_confirmations".into(),
            ));
        }
        other_params_preview(ctx, args, "change automatic approvals in another chat").await
    })
}

fn set_auto_approve<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(other_params_apply(ctx, args, "chats.set_auto_approve"))
}

/// A loaded chat other than this one, never opened from disk.
async fn loaded_other(ctx: &EngineCtx, chat_id: &str) -> Result<SessionArc, EngineError> {
    if chat_id == ctx.chat_id {
        return Err(EngineError::Type(
            "this is this chat; its privacy records cannot be cleared while the script runs".into(),
        ));
    }
    crate::chat::trajectories::validate_trajectory_id(chat_id)
        .map_err(|e| EngineError::Type(format!("invalid chat_id: {}", e.message)))?;
    if ctx
        .app
        .agents
        .find_agent_id_by_child_chat_id(chat_id)
        .await
        .is_some()
    {
        return Err(EngineError::Permission(format!(
            "chat {chat_id} is a background agent's chat; use the agents area"
        )));
    }
    let session = ctx.app.chat.sessions.read().await.get(chat_id).cloned();
    session.ok_or_else(|| {
        EngineError::Permission(format!(
            "chat {chat_id} is not loaded (Chat session not found); open it first"
        ))
    })
}

/// Every privacy record of the messages, including those nested in compression reports.
fn walk_records(extra: &serde_json::Map<String, Value>, out: &mut Vec<refact_privacy::FileRecord>) {
    if let Some(nested) = extra
        .get(refact_core::active_context::COMPRESSION_REPORT_KEY)
        .and_then(|report| report.get("payload"))
        .and_then(|payload| payload.get("messages"))
        .and_then(Value::as_array)
    {
        for message in nested.iter().filter_map(Value::as_object) {
            walk_records(message, out);
        }
    }
    if let Some(privacy) = extra.get("privacy") {
        if let Ok(privacy) =
            serde_json::from_value::<refact_privacy::PrivacyRecord>(privacy.clone())
        {
            out.extend(privacy.files);
        }
    }
}

#[derive(Clone)]
struct ClearPlan {
    paths: Vec<String>,
}

/// Per requested path: the zones of its records and derived zone, and whether anything of it
/// would be forgotten; plus the state the plan is computed from.
async fn clear_state(
    ctx: &EngineCtx,
    session: &SessionArc,
    paths: &[String],
) -> Result<(String, Vec<(String, Vec<String>, bool)>, Value), EngineError> {
    crate::privacy::load_privacy_if_needed(ctx.app.gcx.clone()).await;
    let classification = crate::privacy::classification_cache::snapshot(&ctx.app.gcx)
        .map_err(|e| EngineError::Failed(format!("the privacy policy could not be loaded: {e}")))?;
    let s = session.lock().await;
    if !s.turn_finished() {
        return Err(EngineError::Permission(format!(
            "chat {} is busy ({}); try again when it is idle",
            s.chat_id, s.runtime.state
        )));
    }
    let mut records = Vec::new();
    for message in &s.messages {
        walk_records(&message.extra, &mut records);
    }
    let derived = s
        .derived_privacy_zones
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let blocked = |zone: &str| crate::http::routers::v1::privacy::blocked_zone(zone);
    let mut rows = Vec::new();
    for path in paths {
        let static_blocked = blocked(
            &classification
                .classify(&ctx.app.gcx, std::path::Path::new(path))
                .zone
                .name,
        );
        let mut zones: Vec<String> = Vec::new();
        let mut removable = false;
        for record in records.iter().filter(|r| &r.path == path) {
            if !zones.contains(&record.zone) {
                zones.push(record.zone.clone());
            }
            removable |= !static_blocked && !blocked(&record.zone);
        }
        if let Some(entry) = derived.get(std::path::Path::new(path)) {
            if !zones.contains(&entry.zone) {
                zones.push(entry.zone.clone());
            }
            removable |= !static_blocked && !blocked(&entry.zone);
        }
        rows.push((path.clone(), zones, removable));
    }
    let state = json!({
        "version": s.trajectory_version,
        "rows": rows.iter().map(|(p, z, r)| json!([p, z, r])).collect::<Vec<_>>(),
    });
    Ok((s.thread.title.clone(), rows, state))
}

fn paths_arg(args: &Value) -> Vec<String> {
    args["paths"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

fn clear_records_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let id = chat_id_arg(args);
        let paths = paths_arg(args);
        let session = loaded_other(ctx, &id).await?;
        let (title, rows, state) = clear_state(ctx, &session, &paths).await?;
        let items = rows
            .iter()
            .map(|(path, zones, removable)| {
                super::mask::named_item(
                    path,
                    (!zones.is_empty()).then(|| json!(zones.join(", "))),
                    Some(json!(if *removable {
                        "forgotten"
                    } else if zones.is_empty() {
                        "no record"
                    } else {
                        "kept (blocked zone)"
                    })),
                )
            })
            .collect();
        let any = rows.iter().any(|(_, _, removable)| *removable);
        Ok(Preview {
            title: "forget privacy records in another chat".into(),
            items,
            scope: Some(chat_scope(&title, &id)),
            class: EngineClass::Risky,
            warning: Some("Content from these files in that chat may then be sent to destinations that were blocked for it.".into()),
            plan: if any {
                Plan::new(state, ClearPlan { paths })
            } else {
                Plan::nothing()
            },
        })
    })
}

fn clear_records<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let id = chat_id_arg(args);
        let session = loaded_other(ctx, &id).await?;
        let (title, _, state) = clear_state(ctx, &session, &paths_arg(args)).await?;
        let plan: ClearPlan = ctx.take_plan("chats.clear_privacy_records", &state)?;
        let response =
            crate::http::routers::v1::privacy::clear_privacy_records(&ctx.app, &id, &plan.paths)
                .await
                .map_err(|e| EngineError::Failed(e.message))?;
        let persisted = response.persisted != Some(false);
        Ok(EngineOutput::new(
            json!({"cleared":response.cleared,"remaining":response.remaining,"persisted":persisted}),
            format!(
                "{} privacy records forgotten in {title}, {} kept{}",
                response.cleared,
                response.remaining,
                if persisted { "" } else { "; not saved to disk" }
            ),
        ))
    })
}

enum MessageRef<'a> {
    Index(usize),
    Id(&'a str),
}

/// Finds a user message of another chat that the short history forms can change.
fn find_user_message(
    chat_id: &str,
    messages: &[crate::call_validation::ChatMessage],
    at: &MessageRef,
) -> Result<usize, EngineError> {
    let index = match at {
        MessageRef::Index(index) => {
            if *index >= messages.len() {
                return Err(EngineError::Type(format!(
                    "index {index} is out of range: chat {chat_id} has {} messages; see chats.read",
                    messages.len()
                )));
            }
            *index
        }
        MessageRef::Id(id) => messages
            .iter()
            .position(|m| m.message_id == *id)
            .ok_or_else(|| {
                EngineError::Type(format!(
                    "message {id} is not in chat {chat_id}; see chats.read"
                ))
            })?,
    };
    let role = &messages[index].role;
    if role != "user" {
        return Err(EngineError::Permission(format!(
            "message {index} of chat {chat_id} is a {role} message; only user messages can be changed"
        )));
    }
    if let MessageRef::Id(_) = at {
        let start =
            refact_core::active_context::active_context_mutable_start(messages).map_err(|e| {
                EngineError::Failed(format!("the chat's history cannot be edited now: {e:?}"))
            })?;
        if index < start {
            return Err(EngineError::Permission(format!(
                "message {index} of chat {chat_id} is inside compressed history and cannot be changed"
            )));
        }
    }
    Ok(index)
}

/// The detached view of another idle chat and the user message a short history form changes; a
/// saved chat is not opened (A1).
async fn message_view(
    ctx: &EngineCtx,
    args: &Value,
    at: &MessageRef<'_>,
) -> Result<(TargetView, usize), EngineError> {
    let id = chat_id_arg(args);
    let view = target_view(ctx, &id).await?;
    require_idle(&id, &view.idle)?;
    let index = find_user_message(&id, &view.messages, at)?;
    Ok((view, index))
}

fn validate_text(args: &Value) -> Result<String, EngineError> {
    let text = args["text"].as_str().unwrap_or_default().to_string();
    crate::chat::content::validate_content_with_attachments(&json!(text), &[])
        .map_err(EngineError::Type)?;
    Ok(text)
}

#[derive(Clone)]
struct HistoryPlan {
    command: refact_chat_api::ChatCommand,
    removed: usize,
    carry: bool,
    cut: Cut,
}

/// Which messages a short history form removes, from the message it changes.
#[derive(Clone, Copy)]
enum Cut {
    FromIt,
    AfterIt,
    It,
    Nothing,
}

fn cut_positions(cut: Cut, index: usize, len: usize) -> Vec<usize> {
    match cut {
        Cut::FromIt => (index..len).collect(),
        Cut::AfterIt => (index + 1..len).collect(),
        Cut::It => vec![index],
        Cut::Nothing => Vec::new(),
    }
}

/// V16 for the short forms, through the code of chats.patch: the card rows of what the cut
/// discards besides its text (recorded usage and cost, file checkpoints), refused when its
/// delivery ids would not all fit in that chat's duplicate filter.
fn cut_rows(
    messages: &[crate::call_validation::ChatMessage],
    positions: &[usize],
) -> Result<Vec<refact_chat_api::PythonEngineItem>, EngineError> {
    delivery_count_refusal(
        refact_chat_history::history_patch::removed_delivery_ids(messages, positions).len(),
    )?;
    Ok(
        refact_chat_history::history_patch::removal_rows(messages, positions)
            .into_iter()
            .map(|row| super::mask::named_item(&row.label, row.old, row.new))
            .collect(),
    )
}

/// Queues the planned history command: the plan is taken against a fresh view before the chat is
/// opened, and the chat is checked again under the lock that queues the command; the run's
/// privacy records go onto the message that keeps them.
async fn apply_history_plan(
    ctx: &EngineCtx,
    function: &str,
    args: &Value,
    at: MessageRef<'_>,
) -> Result<EngineOutput, EngineError> {
    let (view, _) = message_view(ctx, args, &at).await?;
    let approved = Approved::of(ctx, &view).await;
    let plan: HistoryPlan = ctx.take_plan(function, &pin_state(&view, &approved))?;
    let records = if plan.carry {
        send_clearance(ctx, &effective_model(ctx, &view.thread.model).await).await?;
        run_records(ctx).await?
    } else {
        Vec::new()
    };
    if !records.is_empty()
        && matches!(
            plan.command,
            refact_chat_api::ChatCommand::RetryFromIndex { .. }
        )
    {
        return Err(EngineError::Permission(RETRY_RECORDS_REFUSAL.into()));
    }
    let (session, opened) = live_target(ctx, &view).await?;
    let (id, title) = (view.id.clone(), view.thread.title.clone());
    let command = plan.command.clone();
    let request_id = queue_built(ctx, &session, move |s| {
        live_recheck(ctx, &view, s, opened, 1, &approved)?;
        let index = find_user_message(&view.id, &s.messages, &at)?;
        let positions = cut_positions(plan.cut, index, s.messages.len());
        for id in refact_chat_history::history_patch::removed_delivery_ids(&s.messages, &positions)
        {
            s.record_dropped_delivery(id, refact_core::chat_types::DeliveryStatus::Delivered);
        }
        if !records.is_empty() {
            let mut holder = s.messages[index].clone();
            crate::privacy::records::merge_records(&mut holder, records.clone());
            let kept = refact_privacy::records_from_messages(std::slice::from_ref(&holder))
                .map(|kept| kept.into_iter().map(|(_, r)| r).collect::<Vec<_>>())
                .unwrap_or_default();
            if records
                .iter()
                .filter(|r| crate::privacy::records::record_is_persistable(r))
                .any(|r| !kept.contains(r))
            {
                return Err(EngineError::Permission(
                    "the run's privacy records cannot be attached to that message; nothing was applied".into(),
                ));
            }
            s.messages[index] = holder;
        }
        Ok(command)
    })
    .await?;
    Ok(EngineOutput::new(
        json!({"chat_id":id,"request_id":request_id,"messages_removed":plan.removed,"opened":opened}),
        format!(
            "{} queued in {} ({} messages removed){}",
            function.trim_start_matches("chats."),
            title,
            plan.removed,
            if opened { "; the chat was opened" } else { "" }
        ),
    ))
}

const RETRY_RECORDS_REFUSAL: &str = "this run carries privacy records (it read guarded files or chat content), and the new message of a retry cannot carry them; chats.retry_from runs only from a script that has read nothing guarded, or use chats.edit_message, which keeps the records on the edited message";

/// The card of a short history form. One that starts a turn shows what that chat may do (A5); a
/// saved chat with an active goal is said to keep pursuing it once opened (A1).
async fn history_preview(
    ctx: &EngineCtx,
    title: &str,
    view: &TargetView,
    mut items: Vec<refact_chat_api::PythonEngineItem>,
    mut warning: String,
    plan: HistoryPlan,
    starts_turn: bool,
) -> Preview {
    if starts_turn {
        let (turn_items, turn_warning) = turn_card(ctx, view).await;
        items.extend(turn_items);
        if let Some(turn_warning) = turn_warning {
            warning.push(' ');
            warning.push_str(&turn_warning);
        }
    }
    if !view.loaded && view.goal_active {
        warning.push_str(
            " That chat is not open; opening it to apply this lets its active goal continue.",
        );
    }
    Preview {
        title: title.into(),
        items,
        scope: Some(chat_scope(&view.thread.title, &view.id)),
        class: EngineClass::Destructive,
        warning: Some(warning),
        plan: Plan::new(pin_state(view, &Approved::of(ctx, view).await), plan),
    }
}

fn retry_from_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let text = validate_text(args)?;
        let index = args["index"].as_u64().unwrap_or_default() as usize;
        let (view, index) = message_view(ctx, args, &MessageRef::Index(index)).await?;
        send_clearance(ctx, &effective_model(ctx, &view.thread.model).await).await?;
        if !run_records(ctx).await?.is_empty() {
            return Err(EngineError::Permission(RETRY_RECORDS_REFUSAL.into()));
        }
        let removed = view.messages.len() - index;
        let chars = view.messages[index]
            .content
            .content_text_only()
            .chars()
            .count();
        let mut items = vec![
            super::mask::item(
                "cut at message",
                Some(json!(format!("#{index} user, {chars} characters"))),
                None,
            ),
            super::mask::item("messages removed", Some(json!(removed)), Some(json!(0))),
            super::mask::item("new message", None, Some(json!(text))),
        ];
        items.extend(cut_rows(
            &view.messages,
            &cut_positions(Cut::FromIt, index, view.messages.len()),
        )?);
        Ok(history_preview(
            ctx,
            "rewrite another chat from a message",
            &view,
            items,
            "Messages after this point in that chat will be deleted and cannot be restored.".into(),
            HistoryPlan {
                command: refact_chat_api::ChatCommand::RetryFromIndex {
                    index,
                    content: json!(text),
                    attachments: Vec::new(),
                },
                removed,
                carry: true,
                cut: Cut::FromIt,
            },
            true,
        )
        .await)
    })
}

fn retry_from<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let index = args["index"].as_u64().unwrap_or_default() as usize;
        apply_history_plan(ctx, "chats.retry_from", args, MessageRef::Index(index)).await
    })
}

fn regenerate_arg(args: &Value) -> bool {
    args.get("regenerate").and_then(Value::as_bool) == Some(true)
}

fn edit_message_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let text = validate_text(args)?;
        let message_id = args["message_id"].as_str().unwrap_or_default();
        let (view, index) = message_view(ctx, args, &MessageRef::Id(message_id)).await?;
        let regenerate = regenerate_arg(args) && index + 1 < view.messages.len();
        send_clearance(ctx, &effective_model(ctx, &view.thread.model).await).await?;
        let old = view.messages[index].content.content_text_only();
        if old == text && !regenerate {
            return Ok(Preview {
                title: "edit a message in another chat".into(),
                items: Vec::new(),
                scope: Some(chat_scope(&view.thread.title, &view.id)),
                class: EngineClass::Destructive,
                warning: None,
                plan: Plan::nothing(),
            });
        }
        let removed = if regenerate {
            view.messages.len() - index - 1
        } else {
            0
        };
        let mut items = vec![super::mask::item(
            "message",
            Some(json!(shown_text(&old))),
            Some(json!(text)),
        )];
        let mut warning =
            "The original text will be overwritten and cannot be restored.".to_string();
        let cut = if regenerate {
            Cut::AfterIt
        } else {
            Cut::Nothing
        };
        if regenerate {
            items.push(super::mask::item(
                "messages removed",
                Some(json!(removed)),
                Some(json!(0)),
            ));
            warning.push_str(" Later messages will be deleted.");
        }
        items.extend(cut_rows(
            &view.messages,
            &cut_positions(cut, index, view.messages.len()),
        )?);
        show_records(ctx, &view, &[index])?;
        Ok(history_preview(
            ctx,
            "edit a message in another chat",
            &view,
            items,
            warning,
            HistoryPlan {
                command: refact_chat_api::ChatCommand::UpdateMessage {
                    message_id: message_id.to_string(),
                    content: json!(text),
                    attachments: Vec::new(),
                    regenerate,
                },
                removed,
                carry: true,
                cut,
            },
            regenerate,
        )
        .await)
    })
}

fn edit_message<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let message_id = args["message_id"].as_str().unwrap_or_default();
        apply_history_plan(ctx, "chats.edit_message", args, MessageRef::Id(message_id)).await
    })
}

fn remove_message_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let message_id = args["message_id"].as_str().unwrap_or_default();
        let regenerate = regenerate_arg(args);
        let (view, index) = message_view(ctx, args, &MessageRef::Id(message_id)).await?;
        let removed = if regenerate {
            view.messages.len() - index
        } else {
            1
        };
        let old = view.messages[index].content.content_text_only();
        let mut items = vec![super::mask::item(
            "message",
            Some(json!(shown_text(&old))),
            None,
        )];
        let mut warning = "The message will be deleted and cannot be restored.".to_string();
        let cut = if regenerate { Cut::FromIt } else { Cut::It };
        if regenerate {
            items.push(super::mask::item(
                "messages removed",
                Some(json!(removed)),
                Some(json!(0)),
            ));
            warning.push_str(" Later messages will be deleted too.");
        }
        items.extend(cut_rows(
            &view.messages,
            &cut_positions(cut, index, view.messages.len()),
        )?);
        show_records(ctx, &view, &[index])?;
        Ok(history_preview(
            ctx,
            "delete a message from another chat",
            &view,
            items,
            warning,
            HistoryPlan {
                command: refact_chat_api::ChatCommand::RemoveMessage {
                    message_id: message_id.to_string(),
                    regenerate,
                },
                removed,
                carry: false,
                cut,
            },
            regenerate,
        )
        .await)
    })
}

fn remove_message<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let message_id = args["message_id"].as_str().unwrap_or_default();
        apply_history_plan(
            ctx,
            "chats.remove_message",
            args,
            MessageRef::Id(message_id),
        )
        .await
    })
}

#[derive(Clone)]
struct DeletePlan;

/// One chat of a delete, as its card shows it and as it is pinned until it is deleted.
struct DeleteMember {
    id: String,
    title: String,
    line: String,
    pin: Value,
    disk: crate::chat::trajectories::TrajectoryDiskPin,
}

/// How many holders of a loaded chat's session are not the sessions map, this call and its
/// command processor: a viewer's event stream or a running task (D9).
fn other_holders(session: &SessionArc, processor_running: bool) -> usize {
    Arc::strong_count(session).saturating_sub(2 + usize::from(processor_running))
}

/// Checks one chat of a delete and returns its card line and pin: the target-authority check and
/// this project (A6, A10, through the detached view), the idle gate and no other holder for a
/// loaded chat (K1, D9), and no worktree attached (K4).
async fn delete_member(ctx: &EngineCtx, id: &str) -> Result<DeleteMember, EngineError> {
    let view = target_view(ctx, id).await?;
    if crate::chat::trajectories::trajectory_copy_count(ctx.app.gcx.clone(), id).await > 1 {
        return Err(EngineError::Permission(format!(
            "chat {id} has more than one saved copy; delete it by hand"
        )));
    }
    if view.thread.worktree.is_some() {
        return Err(EngineError::Permission(format!(
            "chat {id} is attached to a worktree; detach it before deleting the chat"
        )));
    }
    if view.loaded {
        require_idle(id, &view.idle)?;
        let session = ctx.app.chat.sessions.read().await.get(id).cloned();
        if let Some(session) = session {
            let running = session
                .lock()
                .await
                .queue_processor_running
                .load(std::sync::atomic::Ordering::SeqCst);
            if other_holders(&session, running) > 0 {
                return Err(EngineError::Permission(format!(
                    "chat {id} is open in a viewer or used by a running task; close it first"
                )));
            }
        }
    }
    let state = if view.loaded {
        "open in the engine".to_string()
    } else {
        format!("updated {}", view.updated_at.clone().unwrap_or_default())
    };
    let title = view.thread.title.chars().take(200).collect::<String>();
    Ok(DeleteMember {
        id: id.to_string(),
        line: format!("{title}, {} messages, {state}", view.messages.len()),
        title,
        pin: json!({"id":id,"digest":view.digest(),"disk":view.disk.fingerprint(),"loaded":view.loaded}),
        disk: view.disk,
    })
}

/// The chats a delete removes, children before their parents (K4): the chat alone, or with
/// `cascade` its whole subtree, every member checked.
async fn delete_members(
    ctx: &EngineCtx,
    root: &str,
    cascade: bool,
) -> Result<Vec<DeleteMember>, EngineError> {
    if root == ctx.chat_id {
        return Err(EngineError::Type(
            "this is this chat; it cannot delete itself".into(),
        ));
    }
    crate::chat::trajectories::validate_trajectory_id(root)
        .map_err(|e| EngineError::Type(format!("invalid chat_id: {}", e.message)))?;
    let links = chat_links(ctx).await?;
    let order = history_gates::cascade_order(root, &links, history_gates::MAX_CASCADE)
        .map_err(EngineError::Permission)?;
    if order.iter().any(|id| *id == ctx.chat_id) {
        return Err(EngineError::Permission(format!(
            "chat {root} is an ancestor of this chat; a chat cannot delete its own tree"
        )));
    }
    if !cascade && order.len() > 1 {
        return Err(EngineError::Permission(format!(
            "chat {root} has child chats; pass cascade=True to delete them with it, or keep them"
        )));
    }
    let mut members = Vec::new();
    for id in &order {
        members.push(delete_member(ctx, id).await?);
    }
    Ok(members)
}

/// The parent link of every known chat: saved chats, and loaded ones with their live parent.
async fn chat_links(ctx: &EngineCtx) -> Result<Vec<(String, Option<String>)>, EngineError> {
    let mut links: Vec<(String, Option<String>)> =
        crate::chat::trajectories::list_all_trajectories_meta(ctx.app.clone(), false)
            .await
            .map_err(|e| EngineError::Failed(format!("the chat list could not be read: {e}")))?
            .into_iter()
            .map(|meta| (meta.id, meta.parent_id))
            .collect();
    let loaded: Vec<(String, SessionArc)> = ctx
        .app
        .chat
        .sessions
        .read()
        .await
        .iter()
        .map(|(id, session)| (id.clone(), session.clone()))
        .collect();
    for (id, session) in loaded {
        let parent = session.lock().await.thread.parent_id.clone();
        let mut known = false;
        for (link, link_parent) in links.iter_mut() {
            if *link == id {
                *link_parent = parent.clone();
                known = true;
            }
        }
        if !known {
            links.push((id, parent));
        }
    }
    Ok(links)
}

fn delete_state(members: &[DeleteMember]) -> Value {
    json!({ "members": members.iter().map(|member| member.pin.clone()).collect::<Vec<_>>() })
}

fn delete_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let id = chat_id_arg(args);
        let cascade = args
            .get("cascade")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let members = delete_members(ctx, &id, cascade).await?;
        let title = members
            .last()
            .map(|member| member.title.clone())
            .unwrap_or_default();
        let several = members.len() > 1;
        Ok(Preview {
            title: if several {
                "delete another chat and its child chats".into()
            } else {
                "delete another chat".into()
            },
            items: members
                .iter()
                .map(|member| {
                    super::mask::named_item(
                        &format!("chat {}", member.id),
                        Some(json!(member.line)),
                        Some(json!("deleted")),
                    )
                })
                .collect(),
            scope: Some(chat_scope(&title, &id)),
            class: EngineClass::Destructive,
            warning: Some(if several {
                format!(
                    "These {} chats and their Python workspaces will be deleted and cannot be restored.",
                    members.len()
                )
            } else {
                "This chat and its Python workspace will be deleted and cannot be restored.".into()
            }),
            plan: Plan::new(delete_state(&members), DeletePlan),
        })
    })
}

/// Barrier inside a delete, after a member was checked again under its reservation and before it
/// is closed.
#[cfg(test)]
pub(super) static DELETE_GATE: TestGates = std::sync::Mutex::new(Vec::new());

/// Why one member of a delete was kept: nothing of it was deleted, or its Python workspace was
/// deleted (at least in part) and its file was kept.
enum Kept {
    Whole(String),
    WorkspaceDeleted(String),
}

/// The idle gate a delete closes a loaded chat with: the engine's own, no viewer subscribed to its
/// events, and the C25 one (K1), which is decided before the chat is closed.
fn idle_to_delete(s: &ChatSession) -> bool {
    s.is_idle_to_close()
        && s.event_tx.receiver_count() == 0
        && (s.closed || history_gates::idle_refusal(&s.chat_id, &idle_of_session(s)).is_none())
}

/// Deletes one member right after checking it again (K4): it is reserved from that check to its
/// unlink, so nothing opens it or branches from it meanwhile; it must be unchanged since the
/// preview; a loaded chat is closed through the idle gate only when nothing else holds it; and the
/// unlink is bound to the file that was checked (T2). `Ok(true)` when another saved copy remains.
async fn delete_one(
    ctx: &EngineCtx,
    member: &DeleteMember,
    deleted: &[String],
) -> Result<bool, Kept> {
    let text = |e: EngineError| match e {
        EngineError::Type(m) | EngineError::Permission(m) | EngineError::Failed(m) => {
            Kept::Whole(m)
        }
    };
    let sessions = &ctx.app.chat.sessions;
    let Some(_reservation) = crate::chat::session_runtime::reserve_for_delete(sessions, &member.id)
    else {
        return Err(Kept::Whole("another delete holds it".into()));
    };
    let again = delete_member(ctx, &member.id).await.map_err(text)?;
    if again.pin != member.pin {
        return Err(Kept::Whole("it changed since the preview".into()));
    }
    let links = chat_links(ctx).await.map_err(text)?;
    if links.iter().any(|(child, parent)| {
        parent.as_deref() == Some(member.id.as_str()) && !deleted.contains(child)
    }) {
        return Err(Kept::Whole(
            "a chat was added below it since the preview".into(),
        ));
    }
    #[cfg(test)]
    pass_test_gate(&DELETE_GATE, &member.id).await;
    let session = sessions.read().await.get(&member.id).cloned();
    let pin = match session {
        Some(session) => {
            if !crate::chat::session_runtime::close_idle_session_now(
                ctx.app.clone(),
                sessions,
                &member.id,
                session,
                idle_to_delete,
            )
            .await
            {
                return Err(Kept::Whole(
                    "it could not be closed in the engine (busy, or open in a viewer or a task); close it first"
                        .into(),
                ));
            }
            again.disk.refreshed().await
        }
        None => again.disk,
    };
    if sessions.read().await.contains_key(&member.id) {
        return Err(Kept::Whole("it was opened again".into()));
    }
    crate::chat::trajectories::delete_trajectory_at(&ctx.app, &member.id, &pin)
        .await
        .map_err(|(error, workspace_deleted)| {
            if workspace_deleted {
                Kept::WorkspaceDeleted(error.message)
            } else {
                Kept::Whole(error.message)
            }
        })
}

fn delete<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let id = chat_id_arg(args);
        let cascade = args
            .get("cascade")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let members = delete_members(ctx, &id, cascade).await?;
        ctx.take_plan::<DeletePlan>("chats.delete", &delete_state(&members))?;
        let mut deleted: Vec<String> = Vec::new();
        let mut applied = Vec::new();
        let mut remaining_copy = false;
        for (position, member) in members.iter().enumerate() {
            #[cfg(test)]
            {
                let session = ctx.app.chat.sessions.read().await.get(&member.id).cloned();
                if let Some(session) = session {
                    test_hook(&format!("delete:{}", member.id), &mut *session.lock().await);
                }
            }
            let line = Some(json!(member.line));
            match delete_one(ctx, member, &deleted).await {
                Ok(remains) => {
                    remaining_copy |= remains;
                    deleted.push(member.id.clone());
                    applied.push(super::mask::named_item(
                        &format!("chat {}", member.id),
                        line,
                        Some(json!("deleted")),
                    ));
                }
                Err(kept) => {
                    let (reason, workspace_deleted) = match kept {
                        Kept::Whole(reason) => (reason, false),
                        Kept::WorkspaceDeleted(reason) => (reason, true),
                    };
                    if workspace_deleted {
                        applied.push(super::mask::named_item(
                            &format!("chat {}", member.id),
                            line,
                            Some(json!("its Python workspace deleted, its file kept")),
                        ));
                    }
                    let mut chats: Vec<Value> = deleted
                        .iter()
                        .map(|deleted| json!({"id":deleted,"deleted":true}))
                        .collect();
                    chats.push(json!({"id":member.id,"deleted":false,"reason":reason,"workspace_deleted":workspace_deleted}));
                    chats.extend(members[position + 1..].iter().map(|rest| {
                        json!({"id":rest.id,"deleted":false,"reason":"not reached: the delete stopped before it","workspace_deleted":false})
                    }));
                    let kept: Vec<&str> = members[position..]
                        .iter()
                        .map(|member| member.id.as_str())
                        .collect();
                    let error = format!(
                        "deleted: {}; stopped at chat {}: {reason}; kept: {}",
                        if deleted.is_empty() {
                            "none".to_string()
                        } else {
                            deleted.join(", ")
                        },
                        member.id,
                        kept.join(", ")
                    );
                    let mut output = EngineOutput::new(
                        json!({"chat_id":id,"deleted":false,"chats":chats}),
                        format!(
                            "{} of {} chats deleted; stopped at chat {}",
                            deleted.len(),
                            members.len(),
                            member.id
                        ),
                    );
                    output.stopped = Some(super::Stopped {
                        error,
                        applied,
                    });
                    return Ok(output);
                }
            }
        }
        let mut data = json!({"chat_id":id,"deleted":true});
        if cascade {
            data["chats"] = json!(deleted);
        }
        if remaining_copy {
            data["remaining_copy"] = json!(true);
        }
        let title = members
            .last()
            .map(|member| member.title.clone())
            .unwrap_or_default();
        Ok(EngineOutput::new(
            data,
            format!(
                "chat {title} deleted{}{}",
                if deleted.len() > 1 {
                    format!(" with {} child chats", deleted.len() - 1)
                } else {
                    String::new()
                },
                if remaining_copy {
                    "; another saved copy remains"
                } else {
                    ""
                }
            ),
        ))
    })
}

// ---------------------------------------------------------------------------
// Shared gates of the trajectory functions: who may be targeted (A6), chats of this project only
// (A10), a detached view for previews that never opens a chat (A1), the idle gate (K1) and the
// pins a plan is compared against when the answer comes (K2).

use refact_chat_history::history_gates::{self, IdleFacts, Refusal, TargetFacts};

const VIEW_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

fn refusal_error(refusal: Refusal) -> EngineError {
    match refusal {
        Refusal::Type(message) => EngineError::Type(message),
        Refusal::Permission(message) => EngineError::Permission(message),
        Refusal::Failed(message) => EngineError::Failed(message),
    }
}

async fn canonical(path: &std::path::Path) -> Option<std::path::PathBuf> {
    tokio::fs::canonicalize(path)
        .await
        .ok()
        .map(|path| dunce::simplified(&path).to_path_buf())
}

/// The project of this chat: the source root of its session, else the project of this run.
async fn this_project(ctx: &EngineCtx) -> Option<std::path::PathBuf> {
    let session = ctx
        .app
        .chat
        .sessions
        .read()
        .await
        .get(&ctx.chat_id)
        .cloned();
    if let Some(session) = session {
        let root = crate::coordination::scope::session_source_root(&*session.lock().await);
        if let Some(root) = root {
            return canonical(&root).await;
        }
    }
    canonical(ctx.project_root.as_deref()?).await
}

/// Whether a chat is stored under this chat's project: the project folder holding its file, or
/// for a chat that was never saved the source root of its live session. `None` when either
/// project cannot be told.
async fn same_project(
    ctx: &EngineCtx,
    file: Option<&std::path::Path>,
    session_root: Option<std::path::PathBuf>,
) -> Option<bool> {
    let target = match file {
        Some(file) => crate::coordination::scope::restored_source_root(&ctx.app, file).await?,
        None => canonical(&session_root?).await?,
    };
    Some(this_project(ctx).await? == target)
}

fn owner_facts(
    facts: &mut TargetFacts,
    thread: &crate::chat::types::ThreadParams,
    runner_owned: bool,
    closed: bool,
) {
    facts.background_agent |= matches!(thread.link_type.as_deref(), Some("subagent" | "delegate"));
    facts.runner_owned = runner_owned;
    facts.companion = crate::buddy::companion::is_companion(thread);
    facts.buddy = thread.buddy_meta.is_some() || thread.mode == "buddy";
    facts.task_role = thread.task_meta.as_ref().map(|meta| meta.role.clone());
    facts.closed = closed;
}

fn open_tool_window(messages: &[crate::call_validation::ChatMessage]) -> bool {
    refact_core::active_context::active_context_mutable_start(messages).is_ok_and(|start| {
        refact_chat_history::history_patch::pending_tool_result_window(messages, start)
    })
}

fn recovery_pending(recovery: Option<&refact_core::recovery::RecoveryRecord>) -> bool {
    recovery.is_some_and(|record| {
        matches!(
            record.state,
            refact_core::recovery::RecoveryState::Waiting
                | refact_core::recovery::RecoveryState::Attempting
                | refact_core::recovery::RecoveryState::NeedsAction
        )
    })
}

fn idle_of_session(s: &ChatSession) -> IdleFacts {
    IdleFacts {
        state: s.runtime.state.to_string(),
        turn_finished: s.turn_finished(),
        streaming: !crate::chat::context_rebuild::explicit_context_ready(s),
        paused: s.runtime.paused || !s.runtime.pause_reasons.is_empty(),
        live_tool_calls: !crate::chat::context_rebuild::live_tool_call_ids(s).is_empty(),
        compressing: crate::chat::context_rebuild::compression_attempt_active(s),
        handoff_or_rebuild: s.pending_mode_handoff.is_some() || s.pending_context_rebuild.is_some(),
        browser_decision: s.pending_browser_message.is_some(),
        interrupted: s.runtime.interruption.is_some(),
        planner_paused: s.planner_paused(),
        queued_commands: s.command_queue.len(),
        taken_commands: s.commands_taken,
        native_in_flight: s
            .native_commands_in_flight
            .load(std::sync::atomic::Ordering::SeqCst),
        pending_deliveries: s.pending_deliveries.len() + s.runner_pending_deliveries.len(),
        wake_pending: !s.delivery_wake_sources.is_empty() || s.is_pending_wake_up(),
        review_or_verifier: s.review_readiness_pending.is_some()
            || s.verifier_readiness_pending.is_some()
            || s.review_readiness_resume_nonce.is_some()
            || s.verifier_readiness_resume_nonce.is_some(),
        recovery_pending: recovery_pending(s.runtime.recovery.as_ref()),
        external_reload: s.external_reload_pending.is_some(),
        open_tool_window: open_tool_window(&s.messages),
    }
}

fn idle_of_saved(saved: &crate::chat::trajectories::LoadedTrajectory) -> IdleFacts {
    IdleFacts {
        state: "saved".into(),
        turn_finished: true,
        live_tool_calls: saved
            .pending_questions
            .iter()
            .any(|set| set.status == refact_chat_api::PendingQuestionStatus::Pending),
        browser_decision: saved.pending_browser_decision.is_some(),
        pending_deliveries: saved.pending_deliveries.len(),
        wake_pending: saved.wake_up_at.is_some_and(|at| at > chrono::Utc::now()),
        recovery_pending: recovery_pending(saved.recovery.as_ref()),
        open_tool_window: open_tool_window(&saved.messages),
        ..Default::default()
    }
}

fn require_idle(id: &str, idle: &IdleFacts) -> Result<(), EngineError> {
    match history_gates::idle_refusal(id, idle) {
        Some(reason) => Err(EngineError::Permission(reason)),
        None => Ok(()),
    }
}

/// A detached read-only view of another chat, taken for a preview without opening it.
struct TargetView {
    id: String,
    loaded: bool,
    file: Option<std::path::PathBuf>,
    disk: crate::chat::trajectories::TrajectoryDiskPin,
    thread: crate::chat::types::ThreadParams,
    messages: Vec<crate::call_validation::ChatMessage>,
    derived: Vec<(std::path::PathBuf, crate::privacy::records::DerivedZone)>,
    goal_active: bool,
    goal: Option<refact_chat_api::GoalSnapshot>,
    goal_stopped_by_abort: bool,
    recovery: Option<refact_core::recovery::RecoveryRecord>,
    pinned_model: bool,
    updated_at: Option<String>,
    idle: IdleFacts,
    /// When the chat's current turn started, as latched by the engine; a preempt pins it.
    turn: Option<u64>,
    /// The trajectory version of a loaded chat.
    version: Option<u64>,
}

impl TargetView {
    /// The history pin of the view, computed only where a change uses it.
    fn digest(&self) -> String {
        history_gates::history_pin(&self.id, self.file.as_deref(), &self.messages)
    }
}

/// Whether a chat pursues its goal now, for loaded and saved chats alike: it owns an active goal
/// that is neither paused nor stopped.
pub(super) fn goal_pursued(goal: Option<&refact_chat_api::GoalSnapshot>) -> bool {
    goal.is_some_and(|goal| goal.active && goal.status == refact_chat_api::GoalStatus::Active)
}

/// A saved chat whose file is larger than this is refused before it is parsed.
const MAX_SAVED_FILE_BYTES: u64 = 96 * 1024 * 1024;

async fn saved_file_refusal(ctx: &EngineCtx, id: &str) -> Result<(), EngineError> {
    let size =
        crate::chat::trajectories::largest_trajectory_candidate(ctx.app.gcx.clone(), id).await;
    if size > MAX_SAVED_FILE_BYTES {
        return Err(EngineError::Permission(format!(
            "chat {id}'s saved file is over {} MiB; compact it first",
            MAX_SAVED_FILE_BYTES / (1024 * 1024)
        )));
    }
    Ok(())
}

/// Reads another chat of this project for a change: its live session copied under its lock when
/// it is loaded, its file otherwise; a saved chat is never opened here. Refuses every target the
/// target-authority check refuses; the idle gate is left to the caller.
async fn target_view(ctx: &EngineCtx, id: &str) -> Result<TargetView, EngineError> {
    let caller = TargetFacts {
        this_chat: id == ctx.chat_id,
        caller_is_child: child_scope(ctx).await.0,
        supervised: Some(false),
        same_project: Some(true),
        ..Default::default()
    };
    if let Some(refusal) = history_gates::target_refusal(id, &caller) {
        return Err(refusal_error(refusal));
    }
    crate::chat::trajectories::validate_trajectory_id(id)
        .map_err(|e| EngineError::Type(format!("invalid chat_id: {}", e.message)))?;
    let mut facts = TargetFacts {
        background_agent: ctx
            .app
            .agents
            .find_agent_id_by_child_chat_id(id)
            .await
            .is_some(),
        supervised: crate::buddy::supervision_authority::chat_is_supervised(id).ok(),
        ..Default::default()
    };
    saved_file_refusal(ctx, id).await?;
    let file = crate::chat::trajectories::find_trajectory_path(ctx.app.gcx.clone(), id).await;
    let disk = crate::chat::trajectories::trajectory_disk_pin_at(file.clone()).await;
    let session = ctx.app.chat.sessions.read().await.get(id).cloned();
    let (
        loaded,
        thread,
        messages,
        derived,
        goal_active,
        goal,
        stopped,
        recovery,
        pinned,
        updated_at,
        idle,
        root,
        turn,
        version,
    ) = match session {
        Some(session) => {
            let s = tokio::time::timeout(VIEW_LOCK_WAIT, session.lock())
                .await
                .map_err(|_| EngineError::Failed(format!("chat {id} is busy; try again")))?;
            owner_facts(&mut facts, &s.thread, s.runner_lease.is_some(), s.closed);
            let derived: Vec<(std::path::PathBuf, crate::privacy::records::DerivedZone)> = s
                .derived_privacy_zones
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .map(|(path, zone)| (path.clone(), zone.clone()))
                .collect();
            (
                true,
                s.thread.clone(),
                s.messages.clone(),
                derived,
                goal_pursued(s.goal.as_ref()),
                s.goal.clone(),
                s.goal_stopped_by_abort,
                s.runtime.recovery.clone(),
                s.active_command.model_override.is_some(),
                None,
                idle_of_session(&s),
                crate::coordination::scope::session_source_root(&s),
                s.turn_origin.as_ref().map(|origin| origin.at_ms),
                Some(s.trajectory_version),
            )
        }
        None => {
            if file.is_none() {
                return Err(EngineError::Type(format!("chat {id} does not exist")));
            }
            let mut saved =
                crate::chat::trajectories::load_trajectory_for_chat(ctx.app.gcx.clone(), id)
                    .await
                    .ok_or_else(|| EngineError::Failed(format!("chat {id} could not be read")))?;
            crate::chat::trajectories::apply_mode_defaults_to_thread(
                ctx.app.gcx.clone(),
                &mut saved.thread,
                saved.auto_approve_editing_tools_present,
                saved.auto_approve_dangerous_commands_present,
            )
            .await;
            owner_facts(&mut facts, &saved.thread, false, false);
            let idle = idle_of_saved(&saved);
            let goal_active = goal_pursued(saved.goal.as_ref());
            (
                false,
                saved.thread,
                saved.messages,
                Vec::new(),
                goal_active,
                saved.goal,
                false,
                saved.recovery,
                false,
                Some(saved.updated_at),
                idle,
                None,
                None,
                None,
            )
        }
    };
    facts.same_project = same_project(ctx, file.as_deref(), root).await;
    if let Some(refusal) = history_gates::target_refusal(id, &facts) {
        return Err(refusal_error(refusal));
    }
    Ok(TargetView {
        id: id.to_string(),
        loaded,
        file,
        disk,
        thread,
        messages,
        derived,
        goal_active,
        goal,
        goal_stopped_by_abort: stopped,
        recovery,
        pinned_model: pinned,
        updated_at,
        idle,
        turn,
        version,
    })
}

/// A1: applying a change opens a saved chat, which lets its active goal continue.
fn with_opening_note(view: &TargetView, warning: Option<String>) -> Option<String> {
    if view.loaded || !view.goal_active {
        return warning;
    }
    let note = "That chat is not open; opening it to apply this lets its active goal continue.";
    Some(match warning {
        Some(warning) => format!("{warning} {note}"),
        None => format!(
            "{}: {note}",
            view.thread.title.chars().take(80).collect::<String>()
        ),
    })
}

/// The settings a change of another chat is approved under: its model as resolved and the
/// provider its requests go to, its mode, each approval flag, its frozen prompt and its goal
/// state. Compared when the answer comes and again under the lock that mutates or accepts.
fn settings_pin(
    thread: &crate::chat::types::ThreadParams,
    goal: Option<&refact_chat_api::GoalSnapshot>,
    stopped_by_abort: bool,
    resolver: &Resolver,
) -> Value {
    let frozen = serde_json::to_vec(&thread.frozen_request_prefix).unwrap_or_default();
    json!({
        "model": thread.model,
        "resolved_model": resolver.model(&thread.model),
        "provider": resolver.provider(&thread.model),
        "mode": thread.mode,
        "approvals": approval_flags(thread),
        "frozen": super::digest(&frozen),
        "goal": goal.map(|goal| json!({"status": goal.status, "stop_reason": goal.stop_reason})),
        "goal_pursued": goal_pursued(goal),
        "goal_stopped_by_abort": stopped_by_abort,
    })
}

/// The settings pin of a target's view, carried to the lock that mutates or accepts.
struct Approved {
    settings: Value,
}

impl Approved {
    async fn of(ctx: &EngineCtx, view: &TargetView) -> Self {
        Self {
            settings: settings_pin(
                &view.thread,
                view.goal.as_ref(),
                view.goal_stopped_by_abort,
                &Resolver::load(ctx).await,
            ),
        }
    }

    /// Compares the live session under its lock, resolving its model with the capabilities loaded
    /// now.
    fn check(
        &self,
        gcx: &crate::global_context::GlobalContext,
        s: &ChatSession,
    ) -> Result<(), String> {
        let resolver =
            Resolver::current(gcx).ok_or("model capabilities are being reloaded; try again")?;
        let now = settings_pin(
            &s.thread,
            s.goal.as_ref(),
            s.goal_stopped_by_abort,
            &resolver,
        );
        if now != self.settings {
            return Err(
                "its model, provider, mode, approval flags, frozen prompt or goal changed".into(),
            );
        }
        Ok(())
    }
}

/// What every history plan of a trajectory function is compared against when the answer comes:
/// the history pin, the file on disk and the settings it was approved under.
fn pin_state(view: &TargetView, approved: &Approved) -> Value {
    json!({
        "digest": view.digest(),
        "disk": view.disk.fingerprint(),
        "settings": approved.settings,
    })
}

/// Barrier before a handler opens a saved chat.
#[cfg(test)]
pub(super) static OPEN_GATE: TestGates = std::sync::Mutex::new(Vec::new());

/// The live session of a target for a handler after approval; a saved chat is opened only now,
/// and only from exactly the file its view was read from. `true` when this call opened it.
async fn live_target(
    ctx: &EngineCtx,
    view: &TargetView,
) -> Result<(SessionArc, bool), EngineError> {
    let sessions = ctx.app.chat.sessions.clone();
    let loaded = sessions.read().await.get(&view.id).cloned();
    if let Some(session) = loaded {
        return Ok((session, false));
    }
    #[cfg(test)]
    pass_test_gate(&OPEN_GATE, &view.id).await;
    match crate::chat::session_runtime::restore_session_if_unchanged(
        ctx.app.clone(),
        &sessions,
        &view.id,
        &view.disk,
    )
    .await
    {
        crate::chat::session_runtime::SessionRestore::Open => {}
        crate::chat::session_runtime::SessionRestore::NoTrajectory => {
            return Err(EngineError::Failed(format!(
                "chat {} no longer exists; nothing was applied",
                view.id
            )))
        }
        crate::chat::session_runtime::SessionRestore::Refused(error) => {
            let reason = match error {
                crate::chat::session_runtime::SessionOpenError::Failed(detail) => detail,
                other => other.to_string(),
            };
            return Err(EngineError::Failed(format!(
                "chat {} cannot be opened now: {reason}; nothing was applied",
                view.id
            )));
        }
    }
    let session = sessions
        .read()
        .await
        .get(&view.id)
        .cloned()
        .ok_or_else(|| {
            EngineError::Failed(format!(
                "chat {} could not be opened; nothing was applied",
                view.id
            ))
        })?;
    Ok((session, true))
}

fn refusal_text(refusal: Refusal) -> String {
    match refusal {
        Refusal::Type(message) | Refusal::Permission(message) | Refusal::Failed(message) => message,
    }
}

/// The owners of a live target, re-read under its lock right before a change.
fn live_owner_refusal(id: &str, s: &ChatSession) -> Option<Refusal> {
    let mut facts = TargetFacts {
        supervised: crate::buddy::supervision_authority::chat_is_supervised(id).ok(),
        same_project: Some(true),
        ..Default::default()
    };
    owner_facts(&mut facts, &s.thread, s.runner_lease.is_some(), s.closed);
    history_gates::target_refusal(id, &facts)
}

/// Re-checks a live target under its lock right before a change: its owners and the idle gate.
/// `own_guards` counts the native mutation guards the caller itself holds on the chat.
fn live_gates(
    view: &TargetView,
    s: &ChatSession,
    opened: bool,
    own_guards: usize,
) -> Result<(), EngineError> {
    let opened_note = if opened { " (the chat was opened)" } else { "" };
    if let Some(refusal) = live_owner_refusal(&view.id, s) {
        return Err(match refusal_error(refusal) {
            EngineError::Permission(m) => EngineError::Permission(format!("{m}{opened_note}")),
            EngineError::Type(m) => EngineError::Type(format!("{m}{opened_note}")),
            EngineError::Failed(m) => EngineError::Failed(format!("{m}{opened_note}")),
        });
    }
    let mut idle = idle_of_session(s);
    idle.native_in_flight = idle.native_in_flight.saturating_sub(own_guards);
    if let Some(reason) = history_gates::idle_refusal(&view.id, &idle) {
        return Err(EngineError::Permission(format!("{reason}{opened_note}")));
    }
    Ok(())
}

/// `live_gates`, and that the history and the settings are still the ones the plan was made
/// and approved from.
fn live_recheck(
    ctx: &EngineCtx,
    view: &TargetView,
    s: &ChatSession,
    opened: bool,
    own_guards: usize,
    approved: &Approved,
) -> Result<(), EngineError> {
    live_gates(view, s, opened, own_guards)?;
    let opened_note = if opened {
        " (opening it changed it)"
    } else {
        ""
    };
    if history_gates::history_pin(&view.id, view.file.as_deref(), &s.messages) != view.digest() {
        return Err(EngineError::Failed(format!(
            "chat {} changed since the preview{opened_note}; nothing was applied, call it again to see a fresh preview",
            view.id
        )));
    }
    approved.check(&ctx.app.gcx, s).map_err(|why| {
        EngineError::Failed(format!(
            "chat {} changed since the preview ({why}){opened_note}; nothing was applied, call it again to see a fresh preview",
            view.id
        ))
    })
}

/// Merges the privacy records of the target messages a card shows into this run before the card
/// is sent, whatever happens afterwards (P1).
fn show_records(
    ctx: &EngineCtx,
    view: &TargetView,
    positions: &[usize],
) -> Result<(), EngineError> {
    let shown: Vec<crate::call_validation::ChatMessage> = positions
        .iter()
        .filter_map(|index| view.messages.get(*index).cloned())
        .collect();
    let mut records = carried_records(&shown, &format!("chat {}'s", view.id))?;
    if !shown.is_empty() && !view.derived.is_empty() {
        let files: Vec<Value> = view
            .derived
            .iter()
            .map(|(path, zone)| json!({"path":path,"zone":zone.zone,"attribution":"observed"}))
            .collect();
        records.push(json!({"files":files}));
    }
    ctx.run_privacy
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .taint
        .extend(records);
    Ok(())
}

/// The model a chat's requests go to: its own model, or the default one when it names none.
pub(super) async fn effective_model(ctx: &EngineCtx, model: &str) -> String {
    Resolver::load(ctx).await.model(model)
}

/// Model capabilities loaded once, to resolve a chat's model to the model and provider its
/// requests go to the same way at the preview and under the lock that applies a change.
struct Resolver(Option<std::sync::Arc<crate::caps::CodeAssistantCaps>>);

impl Resolver {
    async fn load(ctx: &EngineCtx) -> Self {
        Self(
            crate::global_context::try_load_caps_quickly_if_not_present(ctx.app.gcx.clone(), 0)
                .await
                .ok(),
        )
    }

    /// The capabilities loaded right now, read without waiting, as a check under a chat's lock
    /// resolves with; `None` while they are being replaced.
    fn current(gcx: &crate::global_context::GlobalContext) -> Option<Self> {
        gcx.caps_state
            .try_read()
            .ok()
            .map(|state| Self(state.caps.clone()))
    }

    fn model(&self, model: &str) -> String {
        self.0
            .as_ref()
            .and_then(|caps| crate::caps::resolve_chat_model(caps.clone(), model).ok())
            .map(|record| record.base.id.clone())
            .unwrap_or_else(|| model.to_string())
    }

    fn provider(&self, model: &str) -> String {
        crate::privacy::records::provider_destination(&self.model(model))
            .id
            .0
    }
}

// ---------------------------------------------------------------------------
// chats.patch: rewrite another chat's history through the history-patch planner, applied
// whole or not at all under that chat's lock and saved over exactly the file it was planned from.

use refact_chat_history::history_patch::{
    plan_history_patch, FrozenSystem, PatchEnv, PatchItem, PatchOp, PatchOutcome, PatchPlan,
    PatchRefusal,
};

const PATCH_SOURCE: &str = "engine.chats.patch";
const PATCH_TITLE: &str = "rewrite another chat's history";

fn trajectory_functions() -> Vec<EngineFn> {
    let chat_only = || json!({"type":"object","additionalProperties":false,"required":["chat_id"],"properties":{"chat_id":{"type":"string","minLength":1}}});
    vec![
    EngineFn {
        name: "chats.patch",
        summary: "Rewrite another idle chat of this project in one step: edit (whole text, or find/replace for long messages), remove (with with_results for a message's tool results, or a range with through) and insert user, assistant (with closed tool_calls whose results you give) or a leading system message. Ids come from chats.read; pass its digest as expected_digest. Applied whole or not at all, saved, and seen by that chat's viewers at once; nothing is applied when its file changed on disk meanwhile (several engines on one project folder are coordinated only by this check). Refused for busy chats and for agent, runner, supervised, task (planner included), buddy and companion chats. Destructive when it edits or removes, risky when it only inserts. That chat's message text appears on the approval card only.",
        class: EngineClass::Risky,
        input: patch_schema,
        output: || Some(json!({"type":"object","required":["chat_id","digest","persisted","opened","edited","removed","inserted","ids","tool_call_ids"],"properties":{"chat_id":{"type":"string"},"digest":{"type":"string"},"persisted":{"type":"boolean"},"opened":{"type":"boolean","description":"True when this call opened the chat to change it."},"edited":{"type":"integer"},"removed":{"type":"integer"},"inserted":{"type":"integer"},"ids":{"type":"array","items":{"type":"string"},"description":"Ids of the inserted messages, in op order."},"tool_call_ids":{"type":"array","items":{"type":"array","items":{"type":"string"}}},"summary":{"type":"object"}}})),
        handler: patch,
        preview: Some(patch_preview),
        timeout: Some(std::time::Duration::from_secs(60)),
    },
    EngineFn {
        name: "chats.create",
        summary: "Create a new chat of this project (a root chat, listed like any other) with settings (title, model, mode and the other chats.set_params settings, with the approval flags editing_tools, dangerous_commands and all_confirmations), an optional system prompt used instead of its mode's, and first messages (user and assistant; an assistant message may carry closed tool_calls whose results you give). User messages are stamped as written by a script and stay literal (no @ commands). start=True starts an answer at once (the messages must end with a user message); that turn cannot raise engine change cards. Carries this run's privacy records into the messages and passes the egress check for the new chat's model first. Risky when an approval flag is on, a system prompt is given, or it starts a chat that runs tools without asking. Refusals name messages as ops in order, the system prompt first when given.",
        class: EngineClass::Change,
        input: create_schema,
        output: || Some(json!({"type":"object","required":["chat_id","messages","started","persisted"],"properties":{"chat_id":{"type":"string"},"messages":{"type":"integer"},"started":{"type":"boolean"},"request_id":{"type":["string","null"]},"persisted":{"type":"boolean"}}})),
        handler: create,
        preview: Some(create_preview),
        timeout: Some(std::time::Duration::from_secs(60)),
    },
    EngineFn {
        name: "chats.regenerate",
        summary: "Start a new answer in another idle chat of this project without a new message (its history must end with something to answer). The card shows that chat's mode, model, approval flags and goal; risky when it runs tools without asking or pursues an active goal. Refused when its goal is paused or stopped (resume or stop it with chats.goal_control first) or it has a recovery record (a new user message from chats.send or chats.retry_from archives it). The started turn cannot raise engine change cards.",
        class: EngineClass::Change,
        input: chat_only,
        output: || Some(json!({"type":"object","required":["chat_id","request_id","opened"],"properties":{"chat_id":{"type":"string"},"request_id":{"type":"string"},"opened":{"type":"boolean"}}})),
        handler: regenerate,
        preview: Some(regenerate_preview),
        timeout: None,
    },
    EngineFn {
        name: "chats.queue",
        summary: "Another chat's queued commands and pending deliveries (chats of this project, agent, runner and task chats included; a saved chat is read without opening it; refused for buddy and companion chats). Command text is reported as size only; delivery previews carry their privacy records.",
        class: EngineClass::Read,
        input: chat_only,
        output: || Some(json!({"type":"object","required":["chat_id","items","size","loaded"],"properties":{"chat_id":{"type":"string"},"items":{"type":"array","items":{"type":"object","properties":{"id":{"type":"string"},"kind":{"enum":["command","delivery"]},"command_type":{"type":"string"},"priority":{"type":"boolean"},"chars":{"type":"integer"},"preview":{"type":"string"},"push":{"type":"string"},"source":{"type":"string"},"enqueued_at_ms":{"type":"integer"}}}},"size":{"type":"integer"},"loaded":{"type":"boolean"}}})),
        handler: queue_read,
        preview: None,
        timeout: None,
    },
    EngineFn {
        name: "chats.cancel_queued",
        summary: "Remove one queued command or pending delivery of another open chat of this project (id from chats.queue; allowed while it is busy). Destructive for a queued user message or user delivery.",
        class: EngineClass::Change,
        input: || json!({"type":"object","additionalProperties":false,"required":["chat_id","id"],"properties":{"chat_id":{"type":"string","minLength":1},"id":{"type":"string","minLength":1}}}),
        output: || Some(json!({"type":"object","required":["chat_id","removed","kind"],"properties":{"chat_id":{"type":"string"},"removed":{"type":"boolean"},"kind":{"enum":["command","delivery"]}}})),
        handler: cancel_queued,
        preview: Some(cancel_queued_preview),
        timeout: None,
    },
    EngineFn {
        name: "chats.goal_control",
        summary: "Pause, stop or resume the goal of another chat of this project; it applies after that chat's current turn. Resume is risky: that chat keeps working on its goal without you.",
        class: EngineClass::Change,
        input: || json!({"type":"object","additionalProperties":false,"required":["chat_id","action"],"properties":{"chat_id":{"type":"string","minLength":1},"action":{"enum":["pause","resume","stop"]}}}),
        output: || Some(json!({"type":"object","required":["chat_id","request_id","action","opened"],"properties":{"chat_id":{"type":"string"},"request_id":{"type":"string"},"action":{"type":"string"},"opened":{"type":"boolean"}}})),
        handler: goal_control,
        preview: Some(goal_control_preview),
        timeout: None,
    },
    EngineFn {
        name: "chats.set_goal_budget",
        summary: "Change the soft budget (cooldown, no-progress threshold) of another chat's goal; supervised goals and goals with hard limits are refused. Applies after that chat's current turn.",
        class: EngineClass::Change,
        input: || json!({"type":"object","additionalProperties":false,"required":["chat_id"],"properties":{"chat_id":{"type":"string","minLength":1},"cooldown_ms":{"type":"integer","minimum":0,"maximum":3600000},"no_progress_token_threshold":{"type":"integer","minimum":0,"maximum":1000000}}}),
        output: || Some(json!({"type":"object","required":["chat_id","request_id","opened"],"properties":{"chat_id":{"type":"string"},"request_id":{"type":"string"},"old_budget":{"type":"object"},"new_budget":{"type":"object"},"opened":{"type":"boolean"}}})),
        handler: set_goal_budget,
        preview: Some(set_goal_budget_preview),
        timeout: None,
    },
    ]
}

fn patch_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"required":["chat_id","ops"],"properties":{
        "chat_id": {"type":"string","minLength":1},
        "expected_digest": {"type":"string","minLength":1,"description":"digest from chats.read; refused when that chat changed since."},
        "ops": patch_ops_schema("A message id, \"start\" (after a leading system message) or \"end\".")
    }})
}

/// The ops of a history patch; `after` describes the anchors an insert accepts.
pub(super) fn patch_ops_schema(after: &str) -> Value {
    let id = json!({"type":"string","minLength":1});
    json!({"type":"array","minItems":1,"maxItems":50,"items":{"oneOf":[
        {"type":"object","additionalProperties":false,"required":["op","message_id"],"properties":{
            "op":{"const":"edit"},
            "message_id": id.clone(),
            "text":{"type":"string","description":"The whole new text, up to 16 KB."},
            "find":{"type":"string","minLength":1,"description":"With replace: a literal fragment that occurs exactly once (overlaps count)."},
            "replace":{"type":"string"}
        }},
        {"type":"object","additionalProperties":false,"required":["op","message_id"],"properties":{
            "op":{"const":"remove"},
            "message_id": id.clone(),
            "through":{"type":"string","minLength":1,"description":"Remove the inclusive range up to this message (at most 1000)."},
            "with_results":{"type":"boolean","description":"Also remove the results of the removed messages' tool calls."}
        }},
        {"type":"object","additionalProperties":false,"required":["op","after","role","text"],"properties":{
            "op":{"const":"insert"},
            "after":{"type":"string","minLength":1,"description":after},
            "role":{"enum":["user","assistant","system"]},
            "text":{"type":"string"},
            "tool_calls": tool_calls_schema()
        }}
    ]}})
}

/// Closed tool calls of an inserted assistant message: the engine creates their ids and results.
fn tool_calls_schema() -> Value {
    json!({"type":"array","maxItems":16,"items":{"type":"object","additionalProperties":false,"required":["name","arguments","result"],"properties":{
        "name":{"type":"string","minLength":1,"maxLength":64},
        "arguments":{"type":"object"},
        "result":{"type":"string"},
        "failed":{"type":"boolean"}
    }}})
}

pub(super) fn patch_ops(args: &Value) -> Result<Vec<PatchOp>, EngineError> {
    serde_json::from_value(args["ops"].clone()).map_err(|e| EngineError::Type(format!("ops: {e}")))
}

pub(super) fn patch_refusal(refusal: PatchRefusal) -> EngineError {
    match refusal {
        PatchRefusal::Invalid(message) => EngineError::Type(message),
        PatchRefusal::Refused(message) => EngineError::Permission(message),
        PatchRefusal::NeedsRebuild(message) => EngineError::Failed(message),
    }
}

/// The frozen system prompt of a thread: any frozen prefix that does not hold both the prompt and
/// the tools is partial, a tools-only one included.
pub(super) fn frozen_system(thread: &crate::chat::types::ThreadParams) -> FrozenSystem<'_> {
    let Some(prefix) = &thread.frozen_request_prefix else {
        return FrozenSystem::Absent;
    };
    match prefix
        .system_prompt
        .as_deref()
        .filter(|prompt| !prompt.is_empty())
    {
        Some(prompt) if crate::chat::trajectories::frozen_prefix_is_complete(prefix) => {
            FrozenSystem::Complete(prompt)
        }
        _ => FrozenSystem::Partial,
    }
}

/// Merges this run's privacy records into a message the patch writes, refusing when they would
/// not stay readable on it.
pub(super) fn carry_records(
    records: &[refact_privacy::FileRecord],
) -> impl Fn(&mut crate::call_validation::ChatMessage) -> Result<(), String> + '_ {
    move |message: &mut crate::call_validation::ChatMessage| {
        if records.is_empty() {
            return Ok(());
        }
        crate::privacy::records::merge_records(message, records.to_vec());
        let kept: Vec<refact_privacy::FileRecord> =
            refact_privacy::records_from_messages(std::slice::from_ref(message))
                .map(|kept| kept.into_iter().map(|(_, record)| record).collect())
                .unwrap_or_default();
        if records
            .iter()
            .filter(|record| crate::privacy::records::record_is_persistable(record))
            .any(|record| !kept.contains(record))
        {
            return Err("they would not stay attached to the message".into());
        }
        Ok(())
    }
}

/// K3: a loaded history over the planner's size limit is refused before any copy of it is made.
pub(super) async fn history_size_refusal(ctx: &EngineCtx, id: &str) -> Result<(), EngineError> {
    use refact_chat_history::history_patch::MAX_HISTORY_BYTES;
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0 += buf.len();
            if self.0 > MAX_HISTORY_BYTES {
                return Err(std::io::Error::other("over the limit"));
            }
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let Some(session) = ctx.app.chat.sessions.read().await.get(id).cloned() else {
        return Ok(());
    };
    let s = tokio::time::timeout(VIEW_LOCK_WAIT, session.lock())
        .await
        .map_err(|_| EngineError::Failed(format!("chat {id} is busy; try again")))?;
    let mut counter = Counter(0);
    if serde_json::to_writer(&mut counter, &s.messages).is_err() && counter.0 > MAX_HISTORY_BYTES {
        return Err(EngineError::Permission(format!(
            "chat {id}'s history is over {} MiB; compact it first",
            MAX_HISTORY_BYTES / (1024 * 1024)
        )));
    }
    Ok(())
}

/// V16: every delivery id a removal drops must fit in the chat's duplicate filter, or a retry of
/// that delivery could land again.
pub(super) fn delivery_filter_refusal(plan: &PatchPlan) -> Result<(), EngineError> {
    delivery_count_refusal(plan.removed_delivery_ids.len())
}

fn delivery_count_refusal(count: usize) -> Result<(), EngineError> {
    let retention = crate::file_activity::settings().delivery_status_retention;
    if count > retention {
        return Err(EngineError::Permission(format!(
            "the change removes {count} delivered messages, more than the {retention} that chat's duplicate filter keeps, so some could be delivered again; remove them in parts"
        )));
    }
    Ok(())
}

pub(super) fn items_digest(items: &[PatchItem]) -> String {
    super::digest(&serde_json::to_vec(items).unwrap_or_default())
}

#[derive(Clone)]
struct PatchCallPlan {
    items: String,
}

fn patch_warning(view: &TargetView, plan: &PatchPlan) -> String {
    let mut warning = format!(
        "{}: {}",
        view.thread.title.chars().take(80).collect::<String>(),
        plan.warning
    );
    if plan.summary.inserted_tool_results > 0 && view.goal_active {
        warning.push_str(" That chat has an active goal: its model and its goal checks will take the inserted tool results as real evidence.");
    }
    if !view.loaded && view.goal_active {
        warning.push_str(
            " That chat is not open; opening it to apply the patch lets its active goal continue.",
        );
    }
    warning
}

fn patch_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let id = chat_id_arg(args);
        let ops = patch_ops(args)?;
        history_size_refusal(ctx, &id).await?;
        let view = target_view(ctx, &id).await?;
        require_idle(&id, &view.idle)?;
        let approved = Approved::of(ctx, &view).await;
        if let Some(expected) = args.get("expected_digest").and_then(Value::as_str) {
            if expected != view.digest() {
                return Err(EngineError::Type(format!(
                    "expected_digest does not match: chat {id} changed since it was read; read it again with chats.read"
                )));
            }
        }
        send_clearance(ctx, &effective_model(ctx, &view.thread.model).await).await?;
        let records = run_records(ctx).await?;
        let carry = carry_records(&records);
        let env = PatchEnv {
            frozen: frozen_system(&view.thread),
            stamp_source: PATCH_SOURCE,
            now_ms: crate::agents::approval::now_ms(),
            carry: &carry,
            secret_spans: &super::mask::secret_spans,
        };
        let scope = chat_scope(&view.thread.title, &id);
        let plan = match plan_history_patch(&view.messages, &ops, &env).map_err(patch_refusal)? {
            PatchOutcome::NothingToChange => {
                return Ok(Preview {
                    title: PATCH_TITLE.into(),
                    items: Vec::new(),
                    scope: Some(scope),
                    class: EngineClass::Risky,
                    warning: None,
                    plan: Plan::nothing(),
                })
            }
            PatchOutcome::Planned(plan) => plan,
        };
        delivery_filter_refusal(&plan)?;
        let warning = patch_warning(&view, &plan);
        show_records(ctx, &view, &plan.shown)?;
        Ok(Preview {
            title: PATCH_TITLE.into(),
            items: plan
                .items
                .iter()
                .map(|item| {
                    super::mask::named_item(&item.label, item.old.clone(), item.new.clone())
                })
                .collect(),
            scope: Some(scope),
            class: if plan.class == refact_chat_api::PythonEngineClass::Destructive {
                EngineClass::Destructive
            } else {
                EngineClass::Risky
            },
            warning: Some(warning),
            plan: Plan::new(
                pin_state(&view, &approved),
                PatchCallPlan {
                    items: items_digest(&plan.items),
                },
            ),
        })
    })
}

struct AppliedPatch {
    summary: refact_chat_history::history_patch::PatchSummary,
    inserted: Vec<refact_chat_history::history_patch::InsertedMessage>,
}

/// What a planned patch changes in a live chat: its thread (the frozen system prompt synced,
/// provider continuations dropped), its messages and the delivery ids it removes, which stay in
/// the duplicate filter as delivered.
pub(super) struct PatchCommit {
    thread: crate::chat::types::ThreadParams,
    messages: Vec<crate::call_validation::ChatMessage>,
    removed: Vec<refact_core::chat_types::DeliveryStatusRecord>,
}

impl PatchCommit {
    pub(super) fn of(
        s: &ChatSession,
        planned: PatchPlan,
    ) -> (
        Self,
        refact_chat_history::history_patch::PatchSummary,
        Vec<refact_chat_history::history_patch::InsertedMessage>,
    ) {
        let mut thread = s.thread.clone();
        if let (Some(prompt), Some(prefix)) = (
            planned.frozen_system_prompt.as_ref(),
            thread.frozen_request_prefix.as_mut(),
        ) {
            prefix.system_prompt = Some(prompt.clone());
        }
        thread.previous_response_id = None;
        thread.claude_code_identity = None;
        let removed = planned
            .removed_delivery_ids
            .iter()
            .map(|id| refact_core::chat_types::DeliveryStatusRecord {
                id: id.clone(),
                status: refact_core::chat_types::DeliveryStatus::Delivered,
            })
            .collect();
        let PatchPlan {
            messages,
            summary,
            inserted,
            ..
        } = planned;
        (
            Self {
                thread,
                messages,
                removed,
            },
            summary,
            inserted,
        )
    }

    pub(super) fn apply(self, s: &mut ChatSession) {
        s.thread = self.thread;
        s.replace_messages(self.messages);
        s.openai_codex_websocket = Default::default();
        for record in self.removed {
            s.record_dropped_delivery(record.id, record.status);
        }
    }
}

/// Test barriers at named points of a change, by chat id: `(chat id, entered, release)`; each is
/// passed once.
#[cfg(test)]
pub(super) type TestGates =
    std::sync::Mutex<Vec<(String, Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>)>>;

#[cfg(test)]
async fn pass_test_gate(gates: &TestGates, id: &str) {
    let gate = {
        let mut gates = gates.lock().unwrap();
        gates
            .iter()
            .position(|(gate, _, _)| gate == id)
            .map(|at| gates.remove(at))
    };
    if let Some((_, entered, release)) = gate {
        entered.notify_one();
        release.notified().await;
    }
}

/// Barrier before an approved patch takes its chat's lock.
#[cfg(test)]
pub(super) static PATCH_LOCK_GATE: TestGates = std::sync::Mutex::new(Vec::new());

/// Re-plans the patch on the live history under the chat's lock, then hands the commit to a task
/// that saves the result over exactly the file the preview saw and only then puts it in memory:
/// a conflict applies nothing, and once the save starts the task finishes the file and the live
/// history together whatever happens to this call (T1). The index entry and the sidebar events
/// follow the commit.
#[allow(clippy::too_many_arguments)]
async fn apply_patch(
    ctx: &EngineCtx,
    view: &TargetView,
    approved: &Approved,
    session: &SessionArc,
    opened: bool,
    ops: &[PatchOp],
    records: &[refact_privacy::FileRecord],
    expected_items: &str,
) -> Result<AppliedPatch, EngineError> {
    let opened_note = if opened { " (the chat was opened)" } else { "" };
    let moved = || {
        EngineError::Failed(format!(
            "chats.patch: chat {} changed since the preview; nothing was applied{opened_note}, call it again to see a fresh preview",
            view.id
        ))
    };
    #[cfg(test)]
    pass_test_gate(&PATCH_LOCK_GATE, &view.id).await;
    let transition =
        crate::chat::worktree_lifecycle::try_transition_lock(session).map_err(|e| {
            EngineError::Failed(format!(
                "chat {}: {e}; nothing was applied{opened_note}",
                view.id
            ))
        })?;
    let save_mutex = session.lock().await.trajectory_save_mutex.clone();
    let save = save_mutex.lock_owned().await;
    let mut s = session.clone().lock_owned().await;
    live_recheck(ctx, view, &s, opened, 0, approved)?;
    let carry = carry_records(records);
    let planned = {
        let env = PatchEnv {
            frozen: frozen_system(&s.thread),
            stamp_source: PATCH_SOURCE,
            now_ms: crate::agents::approval::now_ms(),
            carry: &carry,
            secret_spans: &super::mask::secret_spans,
        };
        match plan_history_patch(&s.messages, ops, &env).map_err(patch_refusal)? {
            PatchOutcome::Planned(plan) => plan,
            PatchOutcome::NothingToChange => return Err(moved()),
        }
    };
    delivery_filter_refusal(&planned)?;
    if items_digest(&planned.items) != expected_items {
        return Err(moved());
    }
    let guard = crate::chat::NativeMutationGuard::enter_locked(session, &mut s)
        .map_err(EngineError::Permission)?;
    let (commit, summary, inserted) = PatchCommit::of(&s, planned);
    let mut snapshot =
        crate::chat::trajectories::trajectory_snapshot_with(&s, &commit.thread, &commit.messages);
    snapshot.version = s.trajectory_version.saturating_add(1);
    snapshot
        .dropped_deliveries
        .extend(commit.removed.iter().cloned());
    let (gcx, disk, id) = (ctx.app.gcx.clone(), view.disk.clone(), view.id.clone());
    let (committed, outcome) = tokio::sync::oneshot::channel();
    *ctx.completing.lock().unwrap_or_else(|e| e.into_inner()) = Some(format!(
        "the approved patch of chat {id} was being saved and the engine completes it: that chat's file and live history change together or not at all; read it with chats.read to see the outcome"
    ));
    tokio::spawn(async move {
        let held = (transition, save, guard);
        let publication =
            match crate::chat::trajectories::save_snapshot_if_unchanged(gcx, snapshot, &disk).await
            {
                Ok(publication) => publication,
                Err(error) => {
                    let _ = committed.send(Err(error));
                    return;
                }
            };
        commit.apply(&mut s);
        let version = s.trajectory_version;
        s.complete_trajectory_commit(version);
        s.trajectory_commit_notify.notify_waiters();
        let event = s.snapshot();
        s.emit(event);
        drop(s);
        drop(held);
        let _ = committed.send(Ok(()));
        if let Err(error) = publication.publish().await {
            tracing::warn!(
                "chat {id} was patched and saved, but publishing it to the index failed: {error}"
            );
        }
    });
    let result = match outcome.await {
        Ok(Ok(())) => Ok(AppliedPatch {
            summary,
            inserted,
        }),
        Ok(Err(error)) => Err(EngineError::Failed(format!(
            "chat {} was not changed: {error}{opened_note}",
            view.id
        ))),
        Err(_) => Err(EngineError::Failed(format!(
            "the patch of chat {} stopped unexpectedly while it was being saved; whether it was applied is unknown, read it with chats.read",
            view.id
        ))),
    };
    *ctx.completing.lock().unwrap_or_else(|e| e.into_inner()) = None;
    result
}

fn patch<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let id = chat_id_arg(args);
        let ops = patch_ops(args)?;
        history_size_refusal(ctx, &id).await?;
        let view = target_view(ctx, &id).await?;
        require_idle(&id, &view.idle)?;
        let approved = Approved::of(ctx, &view).await;
        let plan: PatchCallPlan = ctx.take_plan("chats.patch", &pin_state(&view, &approved))?;
        send_clearance(ctx, &effective_model(ctx, &view.thread.model).await).await?;
        let records = run_records(ctx).await?;
        let (session, opened) = live_target(ctx, &view).await?;
        let applied = apply_patch(
            ctx,
            &view,
            &approved,
            &session,
            opened,
            &ops,
            &records,
            &plan.items,
        )
        .await?;
        let file = crate::chat::trajectories::find_trajectory_path(ctx.app.gcx.clone(), &id).await;
        let digest = {
            let s = session.lock().await;
            history_gates::history_pin(&id, file.as_deref(), &s.messages)
        };
        let summary = &applied.summary;
        let ids: Vec<&str> = applied
            .inserted
            .iter()
            .map(|inserted| inserted.message_id.as_str())
            .collect();
        let tool_call_ids: Vec<&Vec<String>> = applied
            .inserted
            .iter()
            .map(|inserted| &inserted.tool_call_ids)
            .collect();
        Ok(EngineOutput::new(
            json!({"chat_id":id,"digest":digest,"persisted":true,"opened":opened,"edited":summary.edited.len(),"removed":summary.removed.len(),"inserted":summary.inserted,"ids":ids,"tool_call_ids":tool_call_ids,"summary":summary}),
            format!(
                "patched {}: {} edited, {} removed, {} inserted, saved{}",
                view.thread.title.chars().take(200).collect::<String>(),
                summary.edited.len(),
                summary.removed.len(),
                summary.inserted,
                if opened { "; the chat was opened" } else { "" }
            ),
        ))
    })
}

// ---------------------------------------------------------------------------
// Turns of another chat (A5): what a turn started there may do, and the functions that start,
// steer or inspect one.

/// The A5 part of a card that starts or steers a turn in another chat: its mode, model, approval
/// flags and goal, with a warning when it runs a tool that changes something without asking (an
/// approval flag or an `auto` rule of its mode) or pursues an active goal.
async fn turn_card(
    ctx: &EngineCtx,
    view: &TargetView,
) -> (Vec<refact_chat_api::PythonEngineItem>, Option<String>) {
    let t = &view.thread;
    let model = Resolver::load(ctx).await.model(&t.model);
    let unasked = acts_unasked(ctx, t).await;
    let on = |flag: bool| if flag { "on" } else { "off" };
    let goal = match &view.goal {
        Some(goal) => {
            let status = serde_json::to_value(goal.status)
                .ok()
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_default();
            if view.goal_active {
                format!("{status}, pursued without you")
            } else if matches!(
                goal.status,
                refact_chat_api::GoalStatus::Paused | refact_chat_api::GoalStatus::Stopped
            ) {
                format!("{status}: that chat has a {status} goal, which it does not pursue until it is resumed")
            } else {
                status
            }
        }
        None => "none".into(),
    };
    let title = super::mask::scrub_text(&t.title.chars().take(80).collect::<String>());
    let items = vec![
        super::mask::named_item(
            "chat",
            None,
            Some(json!(format!("{title}, mode {}, model {model}", t.mode))),
        ),
        super::mask::item(
            "approval flags",
            None,
            Some(json!(format!(
                "editing tools {}, dangerous commands {}, all confirmations {}",
                on(t.auto_approve_editing_tools),
                on(t.auto_approve_dangerous_commands),
                on(t.autonomous_no_confirm)
            ))),
        ),
        super::mask::item("goal", None, Some(json!(goal))),
    ];
    let mut warnings = Vec::new();
    if unasked {
        warnings.push(
            "That chat runs tools without asking you, so this can make it change files or run commands.",
        );
    }
    if view.goal_active {
        warnings.push("It has an active goal and keeps working on it without you.");
    }
    let warning = (!warnings.is_empty()).then(|| format!("{title}: {}", warnings.join(" ")));
    (items, warning)
}

/// Whether a chat with these settings runs a tool that changes something without asking (A5),
/// judged on its effective tool catalog: an approval flag counts only when a tool it approves is
/// there, an `auto` rule only for a tool that changes something, `handoff_to_mode` always (it is
/// approved automatically), and an external tool counts as one that changes something. A mode
/// that cannot be read counts.
async fn acts_unasked(ctx: &EngineCtx, thread: &crate::chat::types::ThreadParams) -> bool {
    use crate::yaml_configs::customization_registry::{
        get_mode_config, map_legacy_mode_to_id, match_tool_confirm_action,
    };
    let Some(mode) = get_mode_config(
        ctx.app.gcx.clone(),
        map_legacy_mode_to_id(&thread.mode),
        Some(&thread.model),
    )
    .await
    else {
        return true;
    };
    let tools = crate::tools::tools_list::get_tools_for_mode(
        ctx.app.gcx.clone(),
        &thread.mode,
        Some(&thread.model),
    )
    .await;
    tools.iter().any(|tool| {
        let desc = tool.tool_description();
        let name = desc.name.as_str();
        let rule = match_tool_confirm_action(&mode.tool_confirm.rules, name);
        if rule.as_deref() == Some("deny") {
            return false;
        }
        if name == "handoff_to_mode" {
            return true;
        }
        let changes = crate::chat::tools::grants_script_writes(name)
            || matches!(
                desc.source.source_type,
                crate::tools::tools_description::ToolSourceType::Integration
            );
        changes
            && (rule.as_deref() == Some("auto")
                || crate::chat::tools::should_auto_approve_confirmation(thread, name))
    })
}

/// Why a regenerate would resume a held goal or reset a recovery, which it must not do (A5): the
/// same check at the preview and under the lock that queues it.
fn held_goal_refusal(
    id: &str,
    goal: Option<&refact_chat_api::GoalSnapshot>,
    stopped_by_abort: bool,
    recovery: Option<&refact_core::recovery::RecoveryRecord>,
) -> Option<String> {
    let held = stopped_by_abort
        || goal.is_some_and(|goal| {
            matches!(
                goal.status,
                refact_chat_api::GoalStatus::Paused | refact_chat_api::GoalStatus::Stopped
            ) || goal.stop_reason.as_deref() == Some("manual_abort")
        });
    if held {
        return Some(format!(
            "chat {id} has a paused or stopped goal, which a regenerate would resume; resume or stop it with chats.goal_control first"
        ));
    }
    recovery.is_some().then(|| format!(
        "chat {id} has a recovery record, which a regenerate would reset; a new user message archives it, so use chats.send or chats.retry_from instead"
    ))
}

/// The last message the model would answer, or why there is nothing to answer.
fn regenerate_anchor(view: &TargetView) -> Result<(usize, String, usize), EngineError> {
    let (index, message) = view
        .messages
        .iter()
        .enumerate()
        .rev()
        .find(|(_, message)| !refact_core::chat_types::is_never_model_visible(message))
        .ok_or_else(|| EngineError::Type(format!("chat {} has no messages to answer", view.id)))?;
    if message.role == "assistant" && message.tool_calls.as_ref().is_none_or(Vec::is_empty) {
        return Err(EngineError::Type(format!(
            "chat {} ends with a finished answer; there is nothing to answer, send or insert a user message first",
            view.id
        )));
    }
    Ok((
        index,
        message.role.clone(),
        message.content.content_text_only().chars().count(),
    ))
}

fn regenerate_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let id = chat_id_arg(args);
        let view = target_view(ctx, &id).await?;
        require_idle(&id, &view.idle)?;
        if let Some(refusal) = held_goal_refusal(
            &id,
            view.goal.as_ref(),
            view.goal_stopped_by_abort,
            view.recovery.as_ref(),
        ) {
            return Err(EngineError::Permission(refusal));
        }
        let (index, role, chars) = regenerate_anchor(&view)?;
        let (mut items, warning) = turn_card(ctx, &view).await;
        items.insert(
            0,
            super::mask::item(
                "last message",
                None,
                Some(json!(format!("#{index} {role}, {chars} characters"))),
            ),
        );
        let mut warning = warning;
        if !view.loaded && view.goal_active {
            warning = Some(format!(
                "{} Opening that chat to start the turn lets its active goal continue.",
                warning.unwrap_or_default()
            ));
        }
        Ok(Preview {
            title: "start a new answer in another chat".into(),
            items,
            scope: Some(chat_scope(&view.thread.title, &id)),
            class: if warning.is_some() {
                EngineClass::Risky
            } else {
                EngineClass::Change
            },
            warning,
            plan: Plan::new(pin_state(&view, &Approved::of(ctx, &view).await), ()),
        })
    })
}

fn regenerate<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let id = chat_id_arg(args);
        let view = target_view(ctx, &id).await?;
        require_idle(&id, &view.idle)?;
        let approved = Approved::of(ctx, &view).await;
        ctx.take_plan::<()>("chats.regenerate", &pin_state(&view, &approved))?;
        let (session, opened) = live_target(ctx, &view).await?;
        let request_id = queue_built(ctx, &session, |s| {
            #[cfg(test)]
            test_hook(&format!("regenerate:{id}"), s);
            live_recheck(ctx, &view, s, opened, 1, &approved)?;
            if let Some(refusal) = held_goal_refusal(
                &id,
                s.goal.as_ref(),
                s.goal_stopped_by_abort,
                s.runtime.recovery.as_ref(),
            ) {
                return Err(EngineError::Permission(refusal));
            }
            Ok(refact_chat_api::ChatCommand::Regenerate {})
        })
        .await?;
        Ok(EngineOutput::new(
            json!({"chat_id":id,"request_id":request_id,"opened":opened}),
            format!(
                "a new answer was started in {}{}",
                view.thread.title,
                if opened { " (the chat was opened)" } else { "" }
            ),
        ))
    })
}

/// Queue rows of a chat: commands as metadata only (their text has no privacy records), pending
/// deliveries with their text and records (P3).
fn queue_rows(
    items: &[refact_chat_api::QueuedItem],
    deliveries: &[refact_chat_api::PendingDelivery],
    whose: &str,
) -> Result<(Vec<Value>, Vec<Value>), EngineError> {
    let mut rows = Vec::new();
    let mut shown = Vec::new();
    for item in items {
        let delivery = item.command_type == refact_chat_api::DELIVERY_COMMAND_TYPE;
        let mut row = json!({"id":item.client_request_id,"kind":if delivery {"delivery"} else {"command"},"command_type":item.command_type,"priority":item.priority,"chars":item.preview.chars().count()});
        if delivery {
            if let Some(found) = deliveries.iter().find(|d| d.id == item.client_request_id) {
                row["preview"] = json!(crate::integrations::mcp::redaction::scrub_text(
                    &item.preview.chars().take(200).collect::<String>(),
                    &[]
                ));
                shown.extend(found.messages.iter().cloned());
            }
        }
        if let Some(push) = item.push {
            row["push"] = json!(push);
        }
        if let Some(source) = &item.source {
            row["source"] = json!(source);
        }
        if let Some(at) = item.enqueued_at_ms {
            row["enqueued_at_ms"] = json!(at);
        }
        rows.push(row);
    }
    let records = carried_records(&shown, whose)?;
    Ok((rows, records))
}

fn queue_read<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let id = chat_id_arg(args);
        if id == ctx.chat_id {
            return Err(EngineError::Type(
                "this is this chat; use chat.queue for this chat".into(),
            ));
        }
        crate::chat::trajectories::validate_trajectory_id(&id)
            .map_err(|e| EngineError::Type(format!("invalid chat_id: {}", e.message)))?;
        let (child, root) = child_scope(ctx).await;
        saved_file_refusal(ctx, &id).await?;
        let file = crate::chat::trajectories::find_trajectory_path(ctx.app.gcx.clone(), &id).await;
        let session = ctx.app.chat.sessions.read().await.get(&id).cloned();
        let (thread, items, deliveries, loaded, session_root) = match session {
            Some(session) => {
                let s = tokio::time::timeout(VIEW_LOCK_WAIT, session.lock())
                    .await
                    .map_err(|_| EngineError::Failed(format!("chat {id} is busy; try again")))?;
                let deliveries: Vec<refact_chat_api::PendingDelivery> = s
                    .pending_deliveries
                    .iter()
                    .chain(s.runner_pending_deliveries.iter())
                    .cloned()
                    .collect();
                (
                    s.thread.clone(),
                    s.build_queued_items(),
                    deliveries,
                    true,
                    crate::coordination::scope::session_source_root(&s),
                )
            }
            None => {
                if file.is_none() {
                    return Err(EngineError::Failed(format!("chat {id} was not found")));
                }
                let saved =
                    crate::chat::trajectories::load_trajectory_for_chat(ctx.app.gcx.clone(), &id)
                        .await
                        .ok_or_else(|| {
                            EngineError::Failed(format!("chat {id} could not be read"))
                        })?;
                let items = saved
                    .pending_deliveries
                    .iter()
                    .map(refact_chat_api::QueuedItem::from_pending_delivery)
                    .collect();
                (saved.thread, items, saved.pending_deliveries, false, None)
            }
        };
        if crate::buddy::companion::is_companion(&thread)
            || thread.buddy_meta.is_some()
            || thread.mode == "buddy"
        {
            return Err(EngineError::Permission(format!(
                "chat {id} is a buddy conversation; chats.queue does not read those"
            )));
        }
        if same_project(ctx, file.as_deref(), session_root).await != Some(true) {
            return Err(EngineError::Permission(format!(
                "chat {id} is not a chat of this project; chats.queue reads chats of this project only"
            )));
        }
        if child && thread.root_chat_id.as_deref().unwrap_or(&id) != root && id != root {
            return Err(EngineError::Permission(format!(
                "chat {id} is outside this chat's tree; a child chat reads only its own tree"
            )));
        }
        let (rows, records) = queue_rows(&items, &deliveries, &format!("chat {id}'s queued"))?;
        let mut output = EngineOutput::new(
            json!({"chat_id":id,"items":rows,"size":items.len(),"loaded":loaded}),
            format!(
                "{} queued in {}",
                items.len(),
                thread.title.chars().take(200).collect::<String>()
            ),
        );
        output.privacy = records;
        Ok(output)
    })
}

#[derive(Clone)]
struct CancelOtherPlan {
    delivery: bool,
}

/// The queued item of a loaded target a cancel would remove, with the state the plan pins.
async fn queued_in(
    ctx: &EngineCtx,
    view: &TargetView,
    item_id: &str,
) -> Result<
    (
        SessionArc,
        refact_chat_api::QueuedItem,
        Option<refact_chat_api::PendingDelivery>,
        Value,
    ),
    EngineError,
> {
    let session = ctx
        .app
        .chat
        .sessions
        .read()
        .await
        .get(&view.id)
        .cloned()
        .ok_or_else(|| {
            EngineError::Type(format!(
                "chat {} is not open, so nothing is queued in it now; see chats.queue",
                view.id
            ))
        })?;
    let s = session.lock().await;
    let item = s
        .build_queued_items()
        .into_iter()
        .find(|item| item.client_request_id == item_id)
        .ok_or_else(|| {
            EngineError::Type(format!(
                "nothing with id {item_id} is queued in chat {}; see chats.queue",
                view.id
            ))
        })?;
    let delivery = s
        .pending_deliveries
        .iter()
        .find(|d| d.id == item_id)
        .cloned();
    if delivery.is_none() && s.runner_pending_deliveries.iter().any(|d| d.id == item_id) {
        return Err(EngineError::Permission(format!(
            "{item_id} is owned by the background runner of chat {}",
            view.id
        )));
    }
    let state = json!({"id":item.client_request_id,"command_type":item.command_type,"preview":super::digest(item.preview.as_bytes()),"envelope":delivery.as_ref().map(envelope_pin)});
    drop(s);
    Ok((session, item, delivery, state))
}

/// The whole envelope of a pending delivery: its text, push mode, source and records.
fn envelope_pin(delivery: &refact_chat_api::PendingDelivery) -> String {
    super::digest(&serde_json::to_vec(delivery).unwrap_or_default())
}

fn cancel_queued_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let id = chat_id_arg(args);
        let item_id = args["id"].as_str().unwrap_or_default();
        let view = target_view(ctx, &id).await?;
        let (_, item, delivery, state) = queued_in(ctx, &view, item_id).await?;
        let user_message = item.command_type == "user_message"
            || delivery
                .as_ref()
                .is_some_and(|delivery| delivery.messages.iter().any(|m| m.role == "user"));
        let shown = match &delivery {
            Some(delivery) => {
                let mut records =
                    carried_records(&delivery.messages, &format!("chat {id}'s queued"))?;
                if !view.derived.is_empty() {
                    let files: Vec<Value> = view
                        .derived
                        .iter()
                        .map(|(path, zone)| json!({"path":path,"zone":zone.zone,"attribution":"observed"}))
                        .collect();
                    records.push(json!({"files":files}));
                }
                ctx.run_privacy
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .taint
                    .extend(records);
                json!(crate::integrations::mcp::redaction::scrub_text(
                    &item.preview,
                    &[]
                ))
            }
            None => json!(format!("{} characters", item.preview.chars().count())),
        };
        Ok(Preview {
            title: "cancel a queued item in another chat".into(),
            items: vec![super::mask::named_item(
                &item.command_type,
                Some(shown),
                Some(json!("removed")),
            )],
            scope: Some(chat_scope(&view.thread.title, &id)),
            class: if user_message {
                EngineClass::Destructive
            } else {
                EngineClass::Change
            },
            warning: user_message
                .then(|| "The queued message will be deleted and cannot be restored.".to_string()),
            plan: Plan::new(
                state,
                CancelOtherPlan {
                    delivery: delivery.is_some(),
                },
            ),
        })
    })
}

fn cancel_queued<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let id = chat_id_arg(args);
        let item_id = args["id"].as_str().unwrap_or_default().to_string();
        let view = target_view(ctx, &id).await?;
        let (session, item, _, state) = queued_in(ctx, &view, &item_id).await?;
        let plan: CancelOtherPlan = ctx.take_plan("chats.cancel_queued", &state)?;
        let removed = if plan.delivery {
            let shown = state["envelope"].clone();
            let admit = |s: &ChatSession,
                         pending: &refact_chat_api::PendingDelivery|
             -> Result<(), String> {
                if let Some(refusal) = live_owner_refusal(&id, s) {
                    return Err(refusal_text(refusal));
                }
                if json!(envelope_pin(pending)) != shown {
                    return Err(format!(
                        "chats.cancel_queued: that delivery of chat {id} changed since the preview; nothing was removed"
                    ));
                }
                Ok(())
            };
            crate::chat::delivery::cancel_pending_delivery_admitted(
                ctx.app.clone(),
                session.clone(),
                &item_id,
                &admit,
            )
            .await
            .map_err(EngineError::Failed)?;
            true
        } else {
            let mut s = session.lock().await;
            if let Some(refusal) = live_owner_refusal(&id, &s) {
                return Err(refusal_error(refusal));
            }
            let _guard = crate::chat::NativeMutationGuard::enter_locked(&session, &mut s)
                .map_err(EngineError::Permission)?;
            crate::chat::cancel_queued_locked(&mut s, &id, &item_id)
        };
        if !removed {
            return Err(EngineError::Failed(format!(
                "{item_id} was no longer queued in chat {id}; nothing was removed"
            )));
        }
        Ok(EngineOutput::new(
            json!({"chat_id":id,"removed":true,"kind":if plan.delivery {"delivery"} else {"command"}}),
            format!(
                "removed queued {} from {}",
                item.command_type, view.thread.title
            ),
        ))
    })
}

fn goal_action_arg(args: &Value) -> String {
    args["action"].as_str().unwrap_or_default().to_string()
}

/// The checks `validate_goal_control` runs, on a goal read without opening its chat.
fn goal_control_refusal(
    goal: Option<&refact_chat_api::GoalSnapshot>,
    action: &str,
) -> Option<String> {
    let Some(goal) = goal else {
        return Some("that chat has no goal to control".into());
    };
    match action {
        "pause" | "stop" => None,
        "resume" if !goal.active => {
            Some("that chat does not own its goal; control it where it is owned".into())
        }
        "resume"
            if matches!(
                goal.status,
                refact_chat_api::GoalStatus::Paused | refact_chat_api::GoalStatus::Stopped
            ) =>
        {
            None
        }
        "resume" => Some("resume needs a paused or stopped goal".into()),
        _ => Some("action must be pause, resume or stop".into()),
    }
}

/// What a goal change of another chat pins: the goal's identity, owner and status, and its budget
/// for a budget change; not its progress, attempts or events, nor the file it is saved in, which
/// change while that chat works.
fn goal_pin(goal: Option<&refact_chat_api::GoalSnapshot>, with_budget: bool) -> Value {
    goal.map_or(Value::Null, |goal| {
        json!({
            "version": goal.version,
            "content": super::digest(goal.content.as_bytes()),
            "owner": goal.owner,
            "active": goal.active,
            "status": goal.status,
            "budget": with_budget.then(|| goal.budget.clone()),
        })
    })
}

fn goal_control_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let id = chat_id_arg(args);
        let action = goal_action_arg(args);
        let view = target_view(ctx, &id).await?;
        if let Some(refusal) = goal_control_refusal(view.goal.as_ref(), &action) {
            return Err(EngineError::Type(format!("chat {id}: {refusal}")));
        }
        let status = view
            .goal
            .as_ref()
            .and_then(|goal| serde_json::to_value(goal.status).ok());
        let resume = action == "resume";
        let mut items = vec![super::mask::item(
            "goal status",
            status,
            Some(json!(match action.as_str() {
                "pause" => "paused",
                "stop" => "stopped",
                _ => "active",
            })),
        )];
        let mut warning = None;
        if resume {
            let (card, turn_warning) = turn_card(ctx, &view).await;
            items.extend(card);
            warning = Some(format!(
                "{}: That chat will keep working on its goal without you.{}",
                view.thread.title.chars().take(80).collect::<String>(),
                turn_warning
                    .map(|w| format!(
                        " {}",
                        w.split_once(": ").map_or(w.as_str(), |(_, rest)| rest)
                    ))
                    .unwrap_or_default()
            ));
        }
        Ok(Preview {
            title: format!("{action} another chat's goal"),
            items,
            scope: Some(chat_scope(&view.thread.title, &id)),
            class: if resume {
                EngineClass::Risky
            } else {
                EngineClass::Change
            },
            warning: with_opening_note(&view, warning),
            plan: Plan::new(goal_pin(view.goal.as_ref(), false), ()),
        })
    })
}

fn goal_control<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let id = chat_id_arg(args);
        let action = goal_action_arg(args);
        let view = target_view(ctx, &id).await?;
        let shown = goal_pin(view.goal.as_ref(), false);
        ctx.take_plan::<()>("chats.goal_control", &shown)?;
        let (session, opened) = live_target(ctx, &view).await?;
        let command_action = action.clone();
        let request_id = queue_built(ctx, &session, move |s| {
            if let Some(refusal) = live_owner_refusal(&s.chat_id.clone(), s) {
                return Err(refusal_error(refusal));
            }
            if goal_pin(s.goal.as_ref(), false) != shown {
                return Err(EngineError::Failed(
                    "chats.goal_control: that chat's goal changed since the preview; nothing was applied".into(),
                ));
            }
            crate::chat::queue::validate_goal_control(s, &command_action).map_err(EngineError::Type)?;
            Ok(refact_chat_api::ChatCommand::GoalControl {
                action: command_action,
            })
        })
        .await?;
        if action == "pause" || action == "stop" {
            crate::chat::cancel_recovery_for_goal_stop(&mut *session.lock().await);
        }
        Ok(EngineOutput::new(
            json!({"chat_id":id,"request_id":request_id,"action":action,"opened":opened}),
            format!(
                "goal {action} queued in {}; it applies after that chat's current turn{}",
                view.thread.title,
                if opened { " (the chat was opened)" } else { "" }
            ),
        ))
    })
}

#[derive(Clone)]
struct OtherGoalBudgetPlan {
    budget: refact_chat_api::GoalBudget,
}

fn set_goal_budget_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let id = chat_id_arg(args);
        if args.get("cooldown_ms").is_none() && args.get("no_progress_token_threshold").is_none() {
            return Err(EngineError::Type(
                "no fields given; give cooldown_ms or no_progress_token_threshold".into(),
            ));
        }
        let view = target_view(ctx, &id).await?;
        let goal = view
            .goal
            .clone()
            .ok_or_else(|| EngineError::Type(format!("chat {id} has no goal")))?;
        if !goal.active
            && goal.owner.as_ref().is_some_and(
                |owner| !matches!(owner, refact_chat_api::GoalOwner::Chat { id: owner } if *owner == id),
            )
        {
            return Err(EngineError::Permission(format!(
                "chat {id} does not own its goal; change the budget where the goal is owned"
            )));
        }
        let old = goal.budget.clone();
        if old.supervised || old.clone().without_hard_limits() != old {
            return Err(EngineError::Permission(
                "that goal is supervised or has hard limits (turns, minutes, tokens, cost or no-progress turns); changing its budget would remove them, so scripts may not change it".into(),
            ));
        }
        let mut budget = old.clone();
        let mut items = Vec::new();
        if let Some(n) = args.get("cooldown_ms").and_then(Value::as_u64) {
            budget.cooldown_ms = n;
            if n != old.cooldown_ms {
                items.push(super::mask::item(
                    "cooldown_ms",
                    Some(json!(old.cooldown_ms)),
                    Some(json!(n)),
                ));
            }
        }
        if let Some(n) = args
            .get("no_progress_token_threshold")
            .and_then(Value::as_u64)
        {
            budget.no_progress_token_threshold = n;
            if n != old.no_progress_token_threshold {
                items.push(super::mask::item(
                    "no_progress_token_threshold",
                    Some(json!(old.no_progress_token_threshold)),
                    Some(json!(n)),
                ));
            }
        }
        Ok(Preview {
            title: "change another chat's goal budget".into(),
            plan: if items.is_empty() {
                Plan::nothing()
            } else {
                Plan::new(
                    goal_pin(view.goal.as_ref(), true),
                    OtherGoalBudgetPlan {
                        budget,
                    },
                )
            },
            items,
            scope: Some(chat_scope(&view.thread.title, &id)),
            class: EngineClass::Change,
            warning: with_opening_note(&view, None),
        })
    })
}

fn set_goal_budget<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let id = chat_id_arg(args);
        let view = target_view(ctx, &id).await?;
        let shown = goal_pin(view.goal.as_ref(), true);
        let plan: OtherGoalBudgetPlan = ctx.take_plan("chats.set_goal_budget", &shown)?;
        let (session, opened) = live_target(ctx, &view).await?;
        let budget = plan.budget.clone();
        let request_id = queue_built(ctx, &session, move |s| {
            if let Some(refusal) = live_owner_refusal(&s.chat_id.clone(), s) {
                return Err(refusal_error(refusal));
            }
            if goal_pin(s.goal.as_ref(), true) != shown {
                return Err(EngineError::Failed(
                    "chats.set_goal_budget: that chat's goal changed since the preview; nothing was applied".into(),
                ));
            }
            Ok(refact_chat_api::ChatCommand::SetGoalBudget { budget })
        })
        .await?;
        Ok(EngineOutput::new(
            json!({"chat_id":id,"request_id":request_id,"old_budget":view.goal.map(|goal| goal.budget),"new_budget":plan.budget,"opened":opened}),
            format!(
                "goal budget change queued; it applies after that chat's current turn{}",
                if opened { " (the chat was opened)" } else { "" }
            ),
        ))
    })
}

// ---------------------------------------------------------------------------
// chats.create: a new root chat of this project with its settings and first messages (D5).

const CREATE_SOURCE: &str = "engine.chats.create";

fn create_schema() -> Value {
    let mut schema = object_schema(
        &[
            "model",
            "mode",
            "reasoning",
            "sampling",
            "context",
            "flags",
            "recovery",
        ],
        &[],
    );
    let flags = auto_approve_schema(false);
    for (key, value) in flags["properties"].as_object().into_iter().flatten() {
        schema["properties"][key] = value.clone();
    }
    schema["properties"]["title"] = json!({"type":"string","minLength":1,"maxLength":200});
    schema["properties"]["system"] = json!({"type":"string","minLength":1,"description":"A system prompt used instead of its mode's, up to 16 KB."});
    schema["properties"]["messages"] = json!({"type":"array","maxItems":50,"items":{"type":"object","additionalProperties":false,"required":["role","text"],"properties":{
        "role":{"enum":["user","assistant"]},
        "text":{"type":"string"},
        "tool_calls": tool_calls_schema()
    }}});
    schema["properties"]["start"] = json!({"type":"boolean","description":"Start an answer at once; the messages must end with a user message."});
    schema
}

/// What `chats.create` makes, planned from a fresh thread so that the handler plans the same.
struct NewChat {
    destination: String,
    title: Option<String>,
    patch: Value,
    approval_flags: [bool; 3],
    messages: Vec<crate::call_validation::ChatMessage>,
    start: bool,
    items: Vec<refact_chat_api::PythonEngineItem>,
    warnings: Vec<String>,
    state: Value,
}

/// New chats are stored in the engine's first project folder, so `chats.create` works only from a
/// chat of that project (A10).
async fn require_new_chat_project(ctx: &EngineCtx) -> Result<(), EngineError> {
    let first = crate::files_correction::get_project_dirs(ctx.app.gcx.clone())
        .await
        .into_iter()
        .next();
    let first = match first {
        Some(dir) => canonical(&dir).await,
        None => None,
    };
    if first.is_some() && first == this_project(ctx).await {
        return Ok(());
    }
    Err(EngineError::Permission(
        "new chats are stored in the engine's first project folder, which is not this chat's project; chats.create works only from a chat of that project".into(),
    ))
}

/// The planner ops of a new chat's first messages: the system prompt first, then the messages.
fn create_ops(args: &Value) -> Result<Vec<PatchOp>, EngineError> {
    let mut ops = Vec::new();
    if let Some(system) = args.get("system") {
        ops.push(json!({"op":"insert","after":"start","role":"system","text":system}));
    }
    for message in args
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let mut op =
            json!({"op":"insert","after":"end","role":message["role"],"text":message["text"]});
        if let Some(calls) = message.get("tool_calls") {
            op["tool_calls"] = calls.clone();
        }
        ops.push(op);
    }
    serde_json::from_value(Value::Array(ops))
        .map_err(|e| EngineError::Type(format!("messages: {e}")))
}

/// Card items of a new chat's messages: its system prompt, and `#k role` with each tool call.
fn message_items(
    messages: &[crate::call_validation::ChatMessage],
) -> Vec<refact_chat_api::PythonEngineItem> {
    let mut items = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        let text = message.content.content_text_only();
        if message.role == "system" {
            items.push(super::mask::item("system prompt", None, Some(json!(text))));
            continue;
        }
        items.push(super::mask::named_item(
            &format!("#{index} {}", message.role),
            None,
            Some(json!(text)),
        ));
        for call in message.tool_calls.iter().flatten() {
            let arguments = serde_json::from_str::<Value>(&call.function.arguments)
                .unwrap_or_else(|_| json!(call.function.arguments));
            items.push(super::mask::named_item(
                &format!("#{index} call {}", call.function.name),
                None,
                Some(arguments),
            ));
        }
    }
    items
}

/// Plans the initial settings, including the exact approval flags to set before insertion; its
/// first messages through the history-patch planner on an empty history; and its approval card.
async fn plan_new_chat(
    ctx: &EngineCtx,
    args: &Value,
    records: &[refact_privacy::FileRecord],
) -> Result<NewChat, EngineError> {
    if child_scope(ctx).await.0 {
        return Err(EngineError::Permission(
            "chats.* changes run only in a root chat".into(),
        ));
    }
    require_new_chat_project(ctx).await?;
    let title = match args.get("title") {
        Some(_) => Some(valid_title(args)?),
        None => None,
    };
    let caps = crate::global_context::try_load_caps_quickly_if_not_present(ctx.app.gcx.clone(), 0)
        .await
        .map_err(|e| {
            EngineError::Failed(format!("model capabilities are unavailable: {}", e.message))
        })?;
    let model = args
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| caps.defaults.chat_default_model.clone());
    let record = crate::caps::resolve_chat_model(caps, &model).map_err(EngineError::Type)?;
    let mut thread = crate::chat::types::ThreadParams::default();
    thread.model = model.clone();
    let mut settings = args.clone();
    if let Some(map) = settings.as_object_mut() {
        map.remove("model");
    }
    let mut params = if settings_given(&settings) {
        plan_params(ctx, &thread, false, 0, &settings).await?
    } else {
        ParamsPlan::default()
    };
    plan_flags(&thread, args, &mut params);
    let mode = params
        .patch
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or(thread.mode.as_str())
        .to_string();
    let mut flags = approval_flags(&thread);
    for ((_, key, _), flag) in APPROVAL_ARGS.iter().zip(&mut flags) {
        *flag = params
            .patch
            .get(*key)
            .and_then(Value::as_bool)
            .unwrap_or(*flag);
        params.patch.insert((*key).into(), json!(*flag));
    }
    let unasked = {
        let mut started = thread.clone();
        started.mode = mode.clone();
        [
            started.auto_approve_editing_tools,
            started.auto_approve_dangerous_commands,
            started.autonomous_no_confirm,
        ] = flags;
        acts_unasked(ctx, &started).await
    };
    let start = args.get("start").and_then(Value::as_bool).unwrap_or(false);
    let last_role = args
        .get("messages")
        .and_then(Value::as_array)
        .and_then(|messages| messages.last())
        .and_then(|message| message["role"].as_str());
    if start && last_role != Some("user") {
        return Err(EngineError::Type(
            "start needs messages that end with a user message; an answer starts from a user message".into(),
        ));
    }
    let ops = create_ops(args)?;
    let messages = if ops.is_empty() {
        Vec::new()
    } else {
        let carry = carry_records(records);
        let env = PatchEnv {
            frozen: FrozenSystem::Absent,
            stamp_source: CREATE_SOURCE,
            now_ms: crate::agents::approval::now_ms(),
            carry: &carry,
            secret_spans: &super::mask::secret_spans,
        };
        match plan_history_patch(&[], &ops, &env).map_err(patch_refusal)? {
            PatchOutcome::Planned(plan) => plan.messages,
            PatchOutcome::NothingToChange => Vec::new(),
        }
    };
    let mut items = vec![
        super::mask::item(
            "title",
            None,
            Some(json!(title
                .clone()
                .unwrap_or_else(|| "set from its first messages".into()))),
        ),
        super::mask::item("model", None, Some(json!(model))),
        super::mask::item("mode", None, Some(json!(mode))),
    ];
    items.extend(
        params
            .items
            .iter()
            .filter(|item| {
                !matches!(
                    item.label.as_str(),
                    "mode" | "tools added" | "tools removed" | "system prompt"
                ) && !item.label.starts_with("confirmation ")
                    && !item.label.starts_with("permission ")
                    && !APPROVAL_ARGS.iter().any(|(arg, _, _)| item.label == *arg)
            })
            .cloned(),
    );
    for ((arg, _, _), on) in APPROVAL_ARGS.iter().zip(flags) {
        items.push(super::mask::item(arg, None, Some(json!(on))));
    }
    if args.get("system").is_none() {
        items.push(super::mask::item(
            "system prompt",
            None,
            Some(json!("its mode's prompt, added when it first answers")),
        ));
    }
    items.extend(message_items(&messages));
    if start {
        items.push(super::mask::item(
            "start",
            None,
            Some(json!("yes: it answers at once")),
        ));
        items.push(super::mask::item("goal", None, Some(json!("none"))));
    }
    let mut warnings = params.warnings.clone();
    if args.get("system").is_some() {
        warnings.push("It starts with the given system prompt instead of its mode's.".into());
    }
    if start && unasked {
        warnings.push(
            "It starts answering at once and can change files or run commands without asking you."
                .into(),
        );
    }
    let mut patch = params.patch;
    patch.insert("model".into(), json!(model));
    let patch = Value::Object(patch);
    let state = json!({
        "patch": patch,
        "title": title,
        "items": super::digest(&serde_json::to_vec(&items).unwrap_or_default()),
        "start": start,
    });
    Ok(NewChat {
        destination: record.base.id.clone(),
        title,
        patch,
        approval_flags: flags,
        messages,
        start,
        items,
        warnings,
        state,
    })
}

fn create_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let records = run_records(ctx).await?;
        let new = plan_new_chat(ctx, args, &records).await?;
        send_clearance(ctx, &new.destination).await?;
        let warning =
            (!new.warnings.is_empty()).then(|| format!("New chat: {}", new.warnings.join(" ")));
        Ok(Preview {
            title: "create a chat".into(),
            items: new.items,
            scope: Some("new chat".into()),
            class: if warning.is_some() {
                EngineClass::Risky
            } else {
                EngineClass::Change
            },
            warning,
            plan: Plan::new(new.state, ()),
        })
    })
}

fn create<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let records = run_records(ctx).await?;
        let new = plan_new_chat(ctx, args, &records).await?;
        ctx.take_plan::<()>("chats.create", &new.state)?;
        send_clearance(ctx, &new.destination).await?;
        let new_id = uuid::Uuid::new_v4().to_string();
        let target = crate::chat::session_runtime::create_fresh_session_with_params(
            ctx.app.clone(),
            &ctx.app.chat.sessions,
            &new_id,
            &new.patch,
        )
        .await
        .map_err(|e| EngineError::Failed(format!("the new chat could not be created: {e:?}")))?;
        let count = new.messages.len();
        {
            let mut s = target.lock().await;
            if approval_flags(&s.thread) != new.approval_flags {
                drop(s);
                discard_new_chat(ctx, &new_id, &target).await;
                return Err(EngineError::Permission(
                    "the new chat's approval flags changed since approval; nothing was saved or started".into(),
                ));
            }
            if let Some(title) = &new.title {
                s.set_title(title.clone(), false);
            }
            s.replace_messages(new.messages);
            #[cfg(test)]
            test_hook(&format!("create:{}", s.thread.title), &mut s);
        }
        let saved = crate::chat::trajectories::try_save_trajectory_with_intent(
            ctx.app.clone(),
            target.clone(),
            crate::chat::types::TrajectoryCommitIntent::Required,
        )
        .await;
        if !matches!(saved, Ok(true)) {
            discard_new_chat(ctx, &new_id, &target).await;
            return Err(EngineError::Failed(format!(
                "the new chat could not be saved{}; nothing was created",
                saved.err().map(|e| format!(": {e}")).unwrap_or_default()
            )));
        }
        let request_id = if new.start {
            match queue_built(ctx, &target, |s| {
                if approval_flags(&s.thread) != new.approval_flags {
                    return Err(EngineError::Permission(
                        "the new chat's approval flags changed since approval; nothing was started"
                            .into(),
                    ));
                }
                Ok(refact_chat_api::ChatCommand::Regenerate {})
            })
            .await
            {
                Ok(request_id) => Some(request_id),
                Err(error) => {
                    discard_new_chat(ctx, &new_id, &target).await;
                    return Err(error);
                }
            }
        } else {
            None
        };
        Ok(EngineOutput::new(
            json!({"chat_id":new_id,"messages":count,"started":request_id.is_some(),"request_id":request_id,"persisted":true}),
            format!(
                "created chat {new_id} with {count} messages{}",
                if request_id.is_some() {
                    "; it is answering now"
                } else {
                    ""
                }
            ),
        ))
    })
}
