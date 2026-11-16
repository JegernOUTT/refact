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
            summary: "Another chat's messages by index range; unlike get_trajectory_context, the messages' privacy records and the chat's derived zones travel into this run. Use chat.messages for this chat.",
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
        let session = ctx.app.chat.sessions.read().await.get(&chat_id).cloned();
        let (title, messages, derived, chat_root, loaded, buddy) = match session {
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
                )
            }
            None => {
                let gcx = ctx.app.gcx.clone();
                if crate::chat::trajectories::find_trajectory_path(gcx.clone(), &chat_id)
                    .await
                    .is_none()
                {
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
        output.data = json!({"chat_id":chat_id,"title":title.chars().take(200).collect::<String>(),"total":total,"start":span.start,"loaded":loaded,"messages":rows});
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
            summary: "Change another idle chat's model, mode, reasoning, sampling, context, flags or retry policy (same arguments as the chat.set_* functions). Approval flags are not accepted.",
            class: EngineClass::Change,
            input: || {
                let mut schema = object_schema(&["model", "mode", "reasoning", "sampling", "context", "flags", "recovery"], &["chat_id"]);
                schema["properties"]["chat_id"] = json!({"type":"string","minLength":1});
                schema
            },
            output: || Some(json!({"type":"object","required":["chat_id","request_id","changed"],"properties":{"chat_id":{"type":"string"},"request_id":{"type":"string"},"changed":{"type":"object"}}})),
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
            summary: "Send a user message to another chat; it lands at that chat's next idle boundary and starts a turn. Carries this run's privacy records and passes the egress check for that chat's model first. Risky when that chat runs tools without asking.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"required":["chat_id","text"],"properties":{"chat_id":{"type":"string","minLength":1},"text":{"type":"string","minLength":1,"maxLength":20000}}}),
            output: || Some(json!({"type":"object","required":["chat_id","request_id","queued"],"properties":{"chat_id":{"type":"string"},"request_id":{"type":"string"},"queued":{"type":"boolean"}}})),
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
    format!("chat {} ({id})", title.chars().take(80).collect::<String>())
}

fn chat_id_arg(args: &Value) -> String {
    args["chat_id"].as_str().unwrap_or_default().to_string()
}

#[derive(Clone)]
struct OtherParamsPlan {
    patch: Value,
    changed: serde_json::Map<String, Value>,
}

async fn other_state(
    session: &SessionArc,
) -> (crate::chat::types::ThreadParams, bool, usize, Value) {
    let s = session.lock().await;
    (
        s.thread.clone(),
        s.active_command.model_override.is_some(),
        crate::chat::trajectory_ops::approx_token_count(&s.messages),
        json!({"thread":thread_state(&s.thread),"busy":!s.turn_finished()}),
    )
}

fn set_params_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let given = [
            "model",
            "mode",
            "recovery_mode",
            "max_retries",
            "delay_ms",
            "total_timeout_ms",
            "reset",
        ]
        .iter()
        .chain(REASONING_ARGS)
        .chain(SAMPLING_ARGS)
        .chain(CONTEXT_ARGS)
        .chain(FLAG_ARGS)
        .any(|k| args.get(*k).is_some());
        if !given {
            return Err(EngineError::Type("no settings given".into()));
        }
        let id = chat_id_arg(args);
        let session = other_chat(ctx, &id, true).await?;
        let (thread, pinned, tokens, state) = other_state(&session).await;
        let plan = plan_params(ctx, &thread, pinned, tokens, args).await?;
        let turns_on_approvals = plan
            .changed
            .iter()
            .any(|(key, change)| key.starts_with("auto_approve") && change["new"] == json!(true));
        if turns_on_approvals || plan.mode_widens {
            let (_, root) = child_scope(ctx).await;
            if !this_root(&root, &id, &thread) {
                return Err(EngineError::Permission(format!(
                    "chat {id} is in another chat tree and this change would {} there; approvals change only for chats of this chat's tree",
                    if turns_on_approvals {
                        "turn on automatic approvals"
                    } else {
                        "switch to a mode with more tools or fewer confirmations"
                    }
                )));
            }
        }
        let scope = chat_scope(&thread.title, &id);
        if plan.changed.is_empty() {
            return Ok(Preview {
                title: "change another chat's settings".into(),
                items: Vec::new(),
                scope: Some(scope),
                class: EngineClass::Change,
                warning: None,
                plan: Plan::nothing(),
            });
        }
        let warning = (!plan.warnings.is_empty())
            .then(|| format!("{}: {}", thread.title, plan.warnings.join(" ")));
        Ok(Preview {
            title: "change another chat's settings".into(),
            items: plan.items.clone(),
            scope: Some(scope),
            class: if warning.is_some() {
                EngineClass::Risky
            } else {
                EngineClass::Change
            },
            warning,
            plan: Plan::new(
                state,
                OtherParamsPlan {
                    patch: Value::Object(plan.patch.clone()),
                    changed: plan.changed.clone(),
                },
            ),
        })
    })
}

fn set_params<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let id = chat_id_arg(args);
        let session = other_chat(ctx, &id, true).await?;
        let (thread, _, _, state) = other_state(&session).await;
        let plan: OtherParamsPlan = ctx.take_plan("chats.set_params", &state)?;
        let request_id = queue_command(
            ctx,
            &session,
            refact_chat_api::ChatCommand::SetParams {
                patch: plan.patch,
            },
        )
        .await?;
        Ok(EngineOutput::new(
            json!({"chat_id":id,"request_id":request_id,"changed":plan.changed}),
            format!(
                "{} settings of {} changed (the chat was opened)",
                plan.changed.len(),
                thread.title
            ),
        ))
    })
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
        let session = other_chat(ctx, &id, false).await?;
        let old = session.lock().await.thread.title.clone();
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

#[derive(Clone)]
struct SendPlan {
    model: String,
}

async fn send_target(
    ctx: &EngineCtx,
    id: &str,
) -> Result<(SessionArc, crate::chat::types::ThreadParams), EngineError> {
    let session = other_chat(ctx, id, false).await?;
    let thread = session.lock().await.thread.clone();
    Ok((session, thread))
}

async fn send_clearance(ctx: &EngineCtx, model: &str) -> Result<(), EngineError> {
    let destination = crate::privacy::records::provider_destination(model);
    ctx.clear_for(&destination).await.map_err(|refusal| {
        EngineError::Permission(format!(
            "the message cannot go to that chat's model: {refusal}"
        ))
    })
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
        let (_, thread) = send_target(ctx, &id).await?;
        send_clearance(ctx, &thread.model).await?;
        let flags = thread.auto_approve_dangerous_commands
            || thread.autonomous_no_confirm
            || thread.auto_approve_editing_tools;
        Ok(Preview {
            title: "send a message to another chat".into(),
            items: vec![
                super::mask::item("message", None, Some(json!(text))),
                super::mask::item("chat", None, Some(json!(format!("{}, mode {}, model {}", thread.title, thread.mode, thread.model)))),
            ],
            scope: Some(chat_scope(&thread.title, &id)),
            class: if flags { EngineClass::Risky } else { EngineClass::Change },
            warning: flags.then(|| format!("{}: That chat runs tools without asking you, so this message can make it change files or run commands.", thread.title)),
            plan: Plan::new(json!({"model":thread.model,"mode":thread.mode,"flags":flags}), SendPlan { model: thread.model.clone() }),
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
        let (_, thread) = send_target(ctx, &id).await?;
        let flags = thread.auto_approve_dangerous_commands
            || thread.autonomous_no_confirm
            || thread.auto_approve_editing_tools;
        let plan: SendPlan = ctx.take_plan(
            "chats.send",
            &json!({"model":thread.model,"mode":thread.mode,"flags":flags}),
        )?;
        send_clearance(ctx, &plan.model).await?;
        let mut message = crate::call_validation::ChatMessage::new("user".into(), text);
        crate::privacy::records::merge_records(&mut message, run_records(ctx).await?);
        let delivery = refact_chat_api::PendingDelivery::new(
            vec![message],
            refact_chat_api::PushMode::WhenIdle,
            "engine.chats.send",
            true,
        );
        let request_id = delivery.id.clone();
        crate::chat::deliver_to_chat(ctx.app.clone(), &id, delivery)
            .await
            .map_err(EngineError::Failed)?;
        Ok(EngineOutput::new(
            json!({"chat_id":id,"request_id":request_id,"queued":true}),
            format!("message sent to {}", thread.title),
        ))
    })
}

/// The privacy records of this chat and this run, which travel with text the script sends elsewhere.
async fn run_records(ctx: &EngineCtx) -> Result<Vec<refact_privacy::FileRecord>, EngineError> {
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
        let session = other_chat(ctx, &id, false).await?;
        let (state, busy, title) = {
            let s = session.lock().await;
            (
                s.runtime.state.to_string(),
                !s.turn_finished(),
                s.thread.title.clone(),
            )
        };
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
async fn fork_source(
    session: &SessionArc,
    message_id: &str,
) -> Result<(Vec<crate::call_validation::ChatMessage>, String, u64), EngineError> {
    let s = session.lock().await;
    let end = s
        .messages
        .iter()
        .position(|m| m.message_id == message_id)
        .ok_or_else(|| {
            EngineError::Type(format!(
                "message {message_id} is not in chat {}; see chats.read",
                s.chat_id
            ))
        })?;
    Ok((
        s.messages[..=end].to_vec(),
        s.thread.title.clone(),
        s.trajectory_version,
    ))
}

fn fork_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let id = chat_id_arg(args);
        let message_id = args["up_to_message_id"].as_str().unwrap_or_default();
        let session = other_chat(ctx, &id, false).await?;
        let (messages, title, version) = fork_source(&session, message_id).await?;
        crate::privacy::records::records_to_carry(&messages).map_err(|e| {
            EngineError::Failed(format!(
                "the source's privacy records could not be read ({}); nothing was forked",
                e.message
            ))
        })?;
        Ok(Preview {
            title: "fork another chat".into(),
            items: vec![
                super::mask::item("source", None, Some(json!(title))),
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
            plan: Plan::new(json!({"version":version,"message":message_id}), ()),
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
        let source = other_chat(ctx, &id, false).await?;
        let (messages, _, version) = fork_source(&source, &message_id).await?;
        ctx.take_plan::<()>(
            "chats.fork",
            &json!({"version":version,"message":message_id}),
        )?;
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
        let session = other_chat(ctx, &id, true).await?;
        let model = args.get("model").and_then(Value::as_str);
        let preview = compaction_preview(ctx, &session, model).await;
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
            plan: Plan::new(
                json!({"version":preview.trajectory_version}),
                preview.trajectory_version,
            ),
        })
    })
}

fn compact<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let id = chat_id_arg(args);
        let session = other_chat(ctx, &id, true).await?;
        let model = args.get("model").and_then(Value::as_str);
        let version = session.lock().await.trajectory_version;
        let expected: Option<u64> =
            ctx.take_plan("chats.compact", &json!({"version":Some(version)}))?;
        let result = crate::chat::context_rebuild::apply_manual_context_rebuild(
            ctx.app.gcx.clone(),
            &session,
            model,
            expected,
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

use super::chat::{auto_approve_schema, plan_auto_approve, queue_built, ENGINE_UNAFFECTED};

const MAX_SHOWN_CHARS: usize = 300;

fn risky_functions() -> Vec<EngineFn> {
    vec![
        EngineFn {
            name: "chats.set_auto_approve",
            summary: "Turn automatic approvals of another idle chat in this chat's tree on or off (editing tools, dangerous commands, all confirmations). Risky when a flag is turned on. These flags never affect engine functions.",
            class: EngineClass::Change,
            input: || auto_approve_schema(true),
            output: || Some(json!({"type":"object","required":["chat_id","request_id","changed"],"properties":{"chat_id":{"type":"string"},"request_id":{"type":"string"},"changed":{"type":"object"}}})),
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
            summary: "Cut another idle chat's history at a user message (index from chats.read) and send a new user message from there, which starts a turn. Carries this run's privacy records and passes the egress check for that chat's model first.",
            class: EngineClass::Destructive,
            input: || json!({"type":"object","additionalProperties":false,"required":["chat_id","index","text"],"properties":{"chat_id":{"type":"string","minLength":1},"index":{"type":"integer","minimum":0},"text":{"type":"string","minLength":1,"maxLength":20000}}}),
            output: || Some(json!({"type":"object","required":["chat_id","request_id","messages_removed"],"properties":{"chat_id":{"type":"string"},"request_id":{"type":"string"},"messages_removed":{"type":"integer"}}})),
            handler: retry_from,
            preview: Some(retry_from_preview),
            timeout: None,
        },
        EngineFn {
            name: "chats.edit_message",
            summary: "Replace the text of one user message in another idle chat (message_id from chats.read), optionally regenerating from it. Carries this run's privacy records and passes the egress check for that chat's model first.",
            class: EngineClass::Destructive,
            input: || json!({"type":"object","additionalProperties":false,"required":["chat_id","message_id","text"],"properties":{"chat_id":{"type":"string","minLength":1},"message_id":{"type":"string","minLength":1},"text":{"type":"string","minLength":1,"maxLength":20000},"regenerate":{"type":"boolean","description":"Delete the later messages and generate a new answer; false by default."}}}),
            output: || Some(json!({"type":"object","required":["chat_id","request_id","messages_removed"],"properties":{"chat_id":{"type":"string"},"request_id":{"type":"string"},"messages_removed":{"type":"integer"}}})),
            handler: edit_message,
            preview: Some(edit_message_preview),
            timeout: None,
        },
        EngineFn {
            name: "chats.remove_message",
            summary: "Delete one user message from another idle chat (message_id from chats.read), optionally regenerating from there.",
            class: EngineClass::Destructive,
            input: || json!({"type":"object","additionalProperties":false,"required":["chat_id","message_id"],"properties":{"chat_id":{"type":"string","minLength":1},"message_id":{"type":"string","minLength":1},"regenerate":{"type":"boolean","description":"Also delete the later messages and generate a new answer; false by default."}}}),
            output: || Some(json!({"type":"object","required":["chat_id","request_id","messages_removed"],"properties":{"chat_id":{"type":"string"},"request_id":{"type":"string"},"messages_removed":{"type":"integer"}}})),
            handler: remove_message,
            preview: Some(remove_message_preview),
            timeout: None,
        },
        EngineFn {
            name: "chats.delete",
            summary: "Delete another chat's saved trajectory and its Python workspace. Refuses loaded chats, chats with children, task-bound and agent chats.",
            class: EngineClass::Destructive,
            input: || json!({"type":"object","additionalProperties":false,"required":["chat_id"],"properties":{"chat_id":{"type":"string","minLength":1}}}),
            output: || Some(json!({"type":"object","required":["chat_id","deleted"],"properties":{"chat_id":{"type":"string"},"deleted":{"type":"boolean"},"remaining_copy":{"type":"boolean"}}})),
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

fn text_hash(text: &str) -> String {
    crate::chat::cache_diagnostics::sha256_hex(&json!(text))
}

fn this_root(ctx_root: &str, chat_id: &str, thread: &crate::chat::types::ThreadParams) -> bool {
    thread.root_chat_id.as_deref().unwrap_or(chat_id) == ctx_root || chat_id == ctx_root
}

fn set_auto_approve_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let id = chat_id_arg(args);
        let session = other_chat(ctx, &id, true).await?;
        let (thread, _, _, state) = other_state(&session).await;
        let (_, root) = child_scope(ctx).await;
        if !this_root(&root, &id, &thread) {
            return Err(EngineError::Permission(format!(
                "chat {id} is in another chat tree; approvals change only for chats of this chat's tree"
            )));
        }
        let plan = plan_auto_approve(&thread, args)?;
        let title = "change automatic approvals in another chat";
        let scope = chat_scope(&thread.title, &id);
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
        let warning = (!plan.warnings.is_empty())
            .then(|| format!("{}: {}", thread.title, plan.warnings.join(" ")));
        Ok(Preview {
            title: title.into(),
            items: plan.items.clone(),
            scope: Some(scope),
            class: if warning.is_some() {
                EngineClass::Risky
            } else {
                EngineClass::Change
            },
            warning,
            plan: Plan::new(
                state,
                OtherParamsPlan {
                    patch: Value::Object(plan.patch.clone()),
                    changed: plan.changed.clone(),
                },
            ),
        })
    })
}

fn set_auto_approve<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let id = chat_id_arg(args);
        let session = other_chat(ctx, &id, true).await?;
        let (thread, _, _, state) = other_state(&session).await;
        let plan: OtherParamsPlan = ctx.take_plan("chats.set_auto_approve", &state)?;
        let request_id = queue_command(
            ctx,
            &session,
            refact_chat_api::ChatCommand::SetParams {
                patch: plan.patch,
            },
        )
        .await?;
        Ok(EngineOutput::new(
            json!({"chat_id":id,"request_id":request_id,"changed":plan.changed}),
            format!(
                "{} approval flags of {} changed; {ENGINE_UNAFFECTED}",
                plan.changed.len(),
                thread.title
            ),
        ))
    })
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

/// The card's view of one message of another chat: what a later answer is compared against.
struct MessageTarget {
    title: String,
    model: String,
    total: usize,
    index: usize,
    text: String,
    state: Value,
}

fn message_state(s: &ChatSession, index: usize) -> Value {
    let m = &s.messages[index];
    json!({
        "version": s.trajectory_version,
        "total": s.messages.len(),
        "idle": s.turn_finished(),
        "index": index,
        "message_id": m.message_id,
        "role": m.role,
        "text": text_hash(&m.content.content_text_only()),
        "model": s.thread.model,
    })
}

enum MessageRef<'a> {
    Index(usize),
    Id(&'a str),
}

/// Finds a user message of another idle chat that the history commands can change.
fn find_user_message(s: &ChatSession, at: &MessageRef) -> Result<usize, EngineError> {
    let index = match at {
        MessageRef::Index(index) => {
            if *index >= s.messages.len() {
                return Err(EngineError::Type(format!(
                    "index {index} is out of range: chat {} has {} messages; see chats.read",
                    s.chat_id,
                    s.messages.len()
                )));
            }
            *index
        }
        MessageRef::Id(id) => s
            .messages
            .iter()
            .position(|m| m.message_id == *id)
            .ok_or_else(|| {
                EngineError::Type(format!(
                    "message {id} is not in chat {}; see chats.read",
                    s.chat_id
                ))
            })?,
    };
    let role = &s.messages[index].role;
    if role != "user" {
        return Err(EngineError::Permission(format!(
            "message {index} of chat {} is a {role} message; only user messages can be changed",
            s.chat_id
        )));
    }
    if let MessageRef::Id(_) = at {
        let start = refact_core::active_context::active_context_mutable_start(&s.messages)
            .map_err(|e| {
                EngineError::Failed(format!("the chat's history cannot be edited now: {e:?}"))
            })?;
        if index < start {
            return Err(EngineError::Permission(format!(
                "message {index} of chat {} is inside compressed history and cannot be changed",
                s.chat_id
            )));
        }
    }
    Ok(index)
}

async fn message_target(
    session: &SessionArc,
    at: &MessageRef<'_>,
) -> Result<MessageTarget, EngineError> {
    let s = session.lock().await;
    let index = find_user_message(&s, at)?;
    Ok(MessageTarget {
        title: s.thread.title.clone(),
        model: s.thread.model.clone(),
        total: s.messages.len(),
        index,
        text: s.messages[index].content.content_text_only(),
        state: message_state(&s, index),
    })
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
}

/// Queues the planned history command, checking under the queueing lock that the chat still is
/// what the card showed; the run's privacy records go onto the message that keeps them.
async fn apply_history_plan(
    ctx: &EngineCtx,
    function: &str,
    id: &str,
    at: MessageRef<'_>,
) -> Result<EngineOutput, EngineError> {
    let session = other_chat(ctx, id, true).await?;
    let target = message_target(&session, &at).await?;
    let plan: HistoryPlan = ctx.take_plan(function, &target.state)?;
    let records = if plan.carry {
        send_clearance(ctx, &target.model).await?;
        run_records(ctx).await?
    } else {
        Vec::new()
    };
    let shown = target.state.clone();
    let command = plan.command.clone();
    let request_id = queue_built(ctx, &session, move |s| {
        let index = find_user_message(s, &at)?;
        if message_state(s, index) != shown {
            return Err(EngineError::Failed(format!(
                "{function}: chat {id} changed since the preview; nothing was applied"
            )));
        }
        if !records.is_empty() {
            if matches!(command, refact_chat_api::ChatCommand::RetryFromIndex { .. }) {
                return Err(EngineError::Permission(RETRY_RECORDS_REFUSAL.into()));
            }
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
        json!({"chat_id":id,"request_id":request_id,"messages_removed":plan.removed}),
        format!(
            "{} queued in {} ({} messages removed)",
            function.trim_start_matches("chats."),
            target.title,
            plan.removed
        ),
    ))
}

const RETRY_RECORDS_REFUSAL: &str = "this run carries privacy records (it read guarded files or chat content), and the new message of a retry cannot carry them; chats.retry_from runs only from a script that has read nothing guarded, or use chats.edit_message, which keeps the records on the edited message";

fn history_preview(
    title: &str,
    target: &MessageTarget,
    id: &str,
    items: Vec<refact_chat_api::PythonEngineItem>,
    warning: String,
    plan: HistoryPlan,
) -> Preview {
    Preview {
        title: title.into(),
        items,
        scope: Some(chat_scope(&target.title, id)),
        class: EngineClass::Destructive,
        warning: Some(warning),
        plan: Plan::new(target.state.clone(), plan),
    }
}

fn retry_from_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let id = chat_id_arg(args);
        let text = validate_text(args)?;
        let index = args["index"].as_u64().unwrap_or_default() as usize;
        let session = other_chat(ctx, &id, true).await?;
        let target = message_target(&session, &MessageRef::Index(index)).await?;
        send_clearance(ctx, &target.model).await?;
        if !run_records(ctx).await?.is_empty() {
            return Err(EngineError::Permission(RETRY_RECORDS_REFUSAL.into()));
        }
        let removed = target.total - index;
        let items = vec![
            super::mask::item(
                "cut at message",
                Some(json!(shown_text(&target.text))),
                None,
            ),
            super::mask::item("messages removed", Some(json!(removed)), Some(json!(0))),
            super::mask::item("new message", None, Some(json!(text))),
        ];
        Ok(history_preview(
            "rewrite another chat from a message",
            &target,
            &id,
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
            },
        ))
    })
}

fn retry_from<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let index = args["index"].as_u64().unwrap_or_default() as usize;
        apply_history_plan(
            ctx,
            "chats.retry_from",
            &chat_id_arg(args),
            MessageRef::Index(index),
        )
        .await
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
        let id = chat_id_arg(args);
        let text = validate_text(args)?;
        let message_id = args["message_id"].as_str().unwrap_or_default();
        let session = other_chat(ctx, &id, true).await?;
        let target = message_target(&session, &MessageRef::Id(message_id)).await?;
        let regenerate = regenerate_arg(args) && target.index + 1 < target.total;
        send_clearance(ctx, &target.model).await?;
        if target.text == text && !regenerate {
            return Ok(Preview {
                title: "edit a message in another chat".into(),
                items: Vec::new(),
                scope: Some(chat_scope(&target.title, &id)),
                class: EngineClass::Destructive,
                warning: None,
                plan: Plan::nothing(),
            });
        }
        let removed = if regenerate {
            target.total - target.index - 1
        } else {
            0
        };
        let mut items = vec![super::mask::item(
            "message",
            Some(json!(shown_text(&target.text))),
            Some(json!(text)),
        )];
        let mut warning =
            "The original text will be overwritten and cannot be restored.".to_string();
        if regenerate {
            items.push(super::mask::item(
                "messages removed",
                Some(json!(removed)),
                Some(json!(0)),
            ));
            warning.push_str(" Later messages will be deleted.");
        }
        Ok(history_preview(
            "edit a message in another chat",
            &target,
            &id,
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
            },
        ))
    })
}

fn edit_message<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let message_id = args["message_id"].as_str().unwrap_or_default();
        apply_history_plan(
            ctx,
            "chats.edit_message",
            &chat_id_arg(args),
            MessageRef::Id(message_id),
        )
        .await
    })
}

fn remove_message_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let id = chat_id_arg(args);
        let message_id = args["message_id"].as_str().unwrap_or_default();
        let regenerate = regenerate_arg(args);
        let session = other_chat(ctx, &id, true).await?;
        let target = message_target(&session, &MessageRef::Id(message_id)).await?;
        let removed = if regenerate {
            target.total - target.index
        } else {
            1
        };
        let mut items = vec![super::mask::item(
            "message",
            Some(json!(shown_text(&target.text))),
            None,
        )];
        let mut warning = "The message will be deleted and cannot be restored.".to_string();
        if regenerate {
            items.push(super::mask::item(
                "messages removed",
                Some(json!(removed)),
                Some(json!(0)),
            ));
            warning.push_str(" Later messages will be deleted too.");
        }
        Ok(history_preview(
            "delete a message from another chat",
            &target,
            &id,
            items,
            warning,
            HistoryPlan {
                command: refact_chat_api::ChatCommand::RemoveMessage {
                    message_id: message_id.to_string(),
                    regenerate,
                },
                removed,
                carry: false,
            },
        ))
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
            &chat_id_arg(args),
            MessageRef::Id(message_id),
        )
        .await
    })
}

#[derive(Clone)]
struct DeletePlan;

/// The saved chat a delete would remove, refusing every chat the spec keeps.
async fn delete_target(
    ctx: &EngineCtx,
    id: &str,
) -> Result<(String, usize, String, Value), EngineError> {
    if id == ctx.chat_id {
        return Err(EngineError::Type(
            "this is this chat; it cannot delete itself".into(),
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
    if ctx.app.chat.sessions.read().await.contains_key(id) {
        return Err(EngineError::Permission(format!(
            "chat {id} is loaded in the engine; close it before deleting it"
        )));
    }
    let gcx = ctx.app.gcx.clone();
    let path = crate::chat::trajectories::find_trajectory_path(gcx.clone(), id)
        .await
        .ok_or_else(|| EngineError::Type(format!("chat {id} does not exist")))?;
    let loaded = crate::chat::trajectories::load_trajectory_for_chat(gcx, id)
        .await
        .ok_or_else(|| EngineError::Failed(format!("chat {id} could not be read")))?;
    if loaded.thread.task_meta.is_some() {
        return Err(EngineError::Permission(format!(
            "chat {id} belongs to a task; the task owns it"
        )));
    }
    if loaded.thread.buddy_meta.is_some() || crate::buddy::companion::is_companion(&loaded.thread) {
        return Err(EngineError::Permission(format!(
            "chat {id} is a buddy conversation"
        )));
    }
    let children = crate::chat::trajectories::list_all_trajectories_meta(ctx.app.clone(), false)
        .await
        .map_err(|e| EngineError::Failed(format!("the chat list could not be read: {e}")))?
        .iter()
        .filter(|m| m.parent_id.as_deref() == Some(id))
        .count();
    let loaded_children = {
        let sessions: Vec<SessionArc> = ctx
            .app
            .chat
            .sessions
            .read()
            .await
            .values()
            .cloned()
            .collect();
        let mut count = 0;
        for session in sessions {
            if session.lock().await.thread.parent_id.as_deref() == Some(id) {
                count += 1;
            }
        }
        count
    };
    if children + loaded_children > 0 {
        return Err(EngineError::Permission(format!(
            "chat {id} has child chats; delete or keep them first"
        )));
    }
    let title = loaded.thread.title.clone();
    let count = loaded.messages.len();
    let state = json!({"path":path,"updated_at":loaded.updated_at,"messages":count,"title":title});
    Ok((title, count, loaded.updated_at, state))
}

fn delete_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let id = chat_id_arg(args);
        let (title, count, updated_at, state) = delete_target(ctx, &id).await?;
        Ok(Preview {
            title: "delete another chat".into(),
            items: vec![super::mask::item(
                "chat",
                Some(json!(format!(
                    "{}, {count} messages, updated {updated_at}",
                    title.chars().take(200).collect::<String>()
                ))),
                Some(json!("deleted")),
            )],
            scope: Some(chat_scope(&title, &id)),
            class: EngineClass::Destructive,
            warning: Some(
                "This chat and its Python workspace will be deleted and cannot be restored.".into(),
            ),
            plan: Plan::new(state, DeletePlan),
        })
    })
}

fn delete<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let id = chat_id_arg(args);
        let (title, _, _, state) = delete_target(ctx, &id).await?;
        ctx.take_plan::<DeletePlan>("chats.delete", &state)?;
        let remaining_copy = crate::chat::trajectories::delete_trajectory(&ctx.app, &id)
            .await
            .map_err(|e| {
                EngineError::Failed(format!("chat {id} was not deleted: {}", e.message))
            })?;
        let mut data = json!({"chat_id":id,"deleted":true});
        if remaining_copy {
            data["remaining_copy"] = json!(true);
        }
        Ok(EngineOutput::new(
            data,
            if remaining_copy {
                format!("chat {title} deleted; another saved copy of it remains")
            } else {
                format!("chat {title} deleted")
            },
        ))
    })
}
