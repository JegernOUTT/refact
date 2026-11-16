use std::collections::HashSet;
use std::path::Path;

use futures::future::BoxFuture;
use serde_json::{json, Map, Value};

use super::common::{failed, limit, limit_schema, redact, require_root_chat, to_value, PathFilter};
use super::mask::{item, named_item};
use super::{previewed, EngineClass, EngineCtx, EngineError, EngineFn, EngineOutput, Preview};
use crate::tasks::types::TaskStatus;

const FOREIGN_KEEP: &[&str] = &[
    "agent_id",
    "parent_chat_id",
    "child_chat_id",
    "kind",
    "status",
    "title",
    "step_count",
    "model",
    "tokens_used",
    "cost_usd",
    "pending_questions",
    "started_at",
    "finished_at",
    "last_activity",
];

pub fn functions() -> Vec<EngineFn> {
    vec![
        EngineFn {
            name: "agents.list",
            summary: "Every background agent in the engine with owner chat, status, progress, tokens and cost. Agents outside this chat's tree show ids, status, title, counts and times only.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{"status":{"type":"array","minItems":1,"items":{"type":"string"},"description":"Statuses to keep."},"root_chat_id":{"type":"string","minLength":1,"description":"Only agents under this root chat."},"limit":limit_schema()}}),
            output: || Some(json!({"type":"object","required":["agents","own_tree","total","truncated"],"properties":{"agents":{"type":"array","items":{"type":"object"}},"own_tree":{"type":"array","items":{"type":"string"}},"total":{"type":"integer"},"truncated":{"type":"boolean"}}})),
            handler: list,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "agents.wait_graph",
            summary: "Who is waiting on whom among the runs this chat may see, with deadlock candidates.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{"limit":{"type":"integer","minimum":1,"maximum":500},"cursor":{"type":"string","minLength":1}}}),
            output: || Some(json!({"type":"object","properties":{"wait_graph":{"type":"object","properties":{"nodes":{"type":"array"},"edges":{"type":"array"},"withheld_nodes":{"type":"integer"},"incomplete":{"type":"string"},"deadlock":{"type":"object"}}},"next_cursor":{"type":"string"}}})),
            handler: wait_graph,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "agents.attention",
            summary: "Questions and approvals waiting for the user, ranked, and the ones settled without them. The text of entries not waited on by this chat's tree is omitted. Read only: it never answers.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{}}),
            output: || Some(json!({"type":"object","required":["waiting","settled","outstanding"],"properties":{"waiting":{"type":"array","items":{"type":"object"}},"settled":{"type":"array","items":{"type":"object"}},"outstanding":{"type":"integer"},"cap":{"type":"integer"},"batch_window_ms":{"type":"integer"},"default_timeout_secs":{"type":"integer"}}})),
            handler: attention,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "agents.tasks",
            summary: "Task boards of this project with status, card counts, active agents and planner state; with task_id one task with its board summary, ready cards and planner instructions.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{"task_id":{"type":"string","minLength":1},"limit":limit_schema()}}),
            output: || Some(json!({"type":"object","properties":{"tasks":{"type":"array","items":{"type":"object"}},"total":{"type":"integer"},"truncated":{"type":"boolean"},"meta":{"type":"object"},"board_summary":{"type":"object","properties":{"rev":{"type":"integer"},"cards_count":{"type":"integer"},"ready":{"type":"array","items":{"type":"string"}}}},"planner_instructions":{"type":"string"}}})),
            handler: tasks,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "agents.worktrees",
            summary: "Registered and discovered worktrees of a workspace folder with status, references, sizes and cleanup candidates.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{"source_workspace_root":{"type":"string","minLength":1,"description":"A workspace folder; the first one by default."},"limit":limit_schema()}}),
            output: || Some(json!({"type":"object","required":["summary","worktrees","cleanup_candidates","total","truncated"],"properties":{"project_hash":{"type":"string"},"source_workspace_root":{"type":"string"},"generated_at":{"type":"string"},"summary":{"type":"object"},"worktrees":{"type":"array","items":{"type":"object"}},"cleanup_candidates":{"type":"array","items":{"type":"string"}},"total":{"type":"integer"},"truncated":{"type":"boolean"}}})),
            handler: worktrees,
            preview: None,
            timeout: Some(std::time::Duration::from_secs(120)),
        },
        EngineFn {
            name: "agents.worktree_diff",
            summary: "The diff of a registered worktree against its base, bounded in size and redacted. Files this chat's model may not see are listed but their hunks are withheld.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"required":["worktree_id"],"properties":{"worktree_id":{"type":"string","minLength":1},"max_patch_bytes":{"type":"integer","minimum":1,"maximum":1000000,"description":"200000 by default."},"source_workspace_root":{"type":"string","minLength":1}}}),
            output: || Some(json!({"type":"object","required":["files","patch","patch_truncated"],"properties":{"files":{"type":"array","items":{"type":"object"}},"stats":{"type":"object"},"patch":{"type":"string"},"patch_truncated":{"type":"boolean"},"withheld_files":{"type":"array","items":{"type":"string"}},"base_branch":{"type":"string"}}})),
            handler: worktree_diff,
            preview: None,
            timeout: Some(std::time::Duration::from_secs(120)),
        },
        EngineFn {
            name: "agents.checkpoints",
            summary: "Workspace checkpoints of this chat (or of another idle chat), one per user message that has them. Listing does not prove a checkpoint can still be restored.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{"chat_id":{"type":"string","minLength":1},"limit":limit_schema()}}),
            output: || Some(json!({"type":"object","required":["checkpoints","total","truncated"],"properties":{"checkpoints":{"type":"array","items":{"type":"object","properties":{"message_index":{"type":"integer"},"workspace_folder":{"type":"string"},"commit_hash":{"type":"string"}}}},"total":{"type":"integer"},"truncated":{"type":"boolean"}}})),
            handler: checkpoints,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "agents.set_task",
            summary: "Change a task's status, name, base branch or commit, or default agent model, after the user approves. Abandoning a task is destructive; a new base branch does not move existing worktrees.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"required":["task_id"],"properties":{"task_id":{"type":"string","minLength":1},"status":{"enum":["planning","active","paused","completed","abandoned"]},"task_name":{"type":"string","minLength":1,"maxLength":200,"description":"New task name (engine() takes the function name as `name`)."},"base_branch":{"type":"string","minLength":1},"base_commit":{"type":"string","pattern":"^[0-9a-fA-F]{7,64}$"},"default_agent_model":{"type":"string","minLength":1}}}),
            output: || Some(json!({"type":"object","required":["id","name","status"],"properties":{"id":{"type":"string"},"name":{"type":"string"},"status":{"type":"string"},"base_branch":{"type":"string"},"base_commit":{"type":"string"},"default_agent_model":{"type":"string"}}})),
            handler: set_task,
            preview: Some(set_task_preview),
            timeout: None,
        },
        EngineFn {
            name: "agents.message",
            summary: "Send a message to a running background agent of another chat, after the user approves. The agent will believe the user sent it. For this chat's own agents use agent_message.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"required":["agent_id","text"],"properties":{"agent_id":{"type":"string","minLength":1},"text":{"type":"string","minLength":1},"push":{"enum":["preempt","append","when_idle"],"description":"append by default."}}}),
            output: || Some(json!({"type":"object","required":["agent","queued"],"properties":{"agent":{"type":"object"},"queued":{"type":"boolean"},"outcome":{}}})),
            handler: message,
            preview: Some(message_preview),
            timeout: None,
        },
        EngineFn {
            name: "agents.set_planner_instructions",
            summary: "Replace a task's planner instructions, after the user approves. Running planner turns keep their prompt; the next turn uses the new text.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"required":["task_id","content"],"properties":{"task_id":{"type":"string","minLength":1},"content":{"type":"string","maxLength":65536}}}),
            output: || Some(json!({"type":"object","required":["task_id","saved","bytes"],"properties":{"task_id":{"type":"string"},"saved":{"type":"boolean"},"bytes":{"type":"integer"}}})),
            handler: set_planner_instructions,
            preview: Some(set_planner_instructions_preview),
            timeout: None,
        },
        EngineFn {
            name: "agents.cancel",
            summary: "Cancel a background agent of another chat, with its sub-agents by default, after the user approves. For this chat's own agents use agent_cancel.",
            class: EngineClass::Destructive,
            input: || json!({"type":"object","additionalProperties":false,"required":["agent_id"],"properties":{"agent_id":{"type":"string","minLength":1},"subtree":{"type":"boolean","description":"true by default."},"reason":{"type":"string"}}}),
            output: || Some(json!({"type":"object","required":["cancelled_agent_ids","agents"],"properties":{"cancelled_agent_ids":{"type":"array","items":{"type":"string"}},"agents":{"type":"array","items":{"type":"object"}}}})),
            handler: cancel,
            preview: Some(cancel_preview),
            timeout: None,
        },
        EngineFn {
            name: "agents.delete_task",
            summary: "Delete a task with its board, documents and planner history, after the user approves. Refused for active cards unless force=true, while its agents run, and for this chat's own task.",
            class: EngineClass::Destructive,
            input: || json!({"type":"object","additionalProperties":false,"required":["task_id"],"properties":{"task_id":{"type":"string","minLength":1},"force":{"type":"boolean"}}}),
            output: || Some(json!({"type":"object","required":["task_id","deleted"],"properties":{"task_id":{"type":"string"},"deleted":{"type":"boolean"}}})),
            handler: delete_task,
            preview: Some(delete_task_preview),
            timeout: None,
        },
        EngineFn {
            name: "agents.cleanup_worktrees",
            summary: "Remove worktrees that the cleanup plan marks safe (clean, old enough, unreferenced), optionally with their branches, after the user approves. Exactly the worktrees on the card are removed.",
            class: EngineClass::Destructive,
            input: || json!({"type":"object","additionalProperties":false,"properties":{"ids":{"type":"array","minItems":1,"items":{"type":"string","minLength":1},"description":"Worktree ids; every cleanup candidate by default."},"clean_only":{"type":"boolean"},"delete_branches":{"type":"boolean"},"allow_shared":{"type":"boolean"},"min_age_hours":{"type":"integer","minimum":0,"maximum":87600},"source_workspace_root":{"type":"string","minLength":1}}}),
            output: || Some(json!({"type":"object","required":["deleted","skipped"],"properties":{"generated_at":{"type":"string"},"deleted":{"type":"array"},"skipped":{"type":"array"},"warnings":{"type":"array"}}})),
            handler: cleanup_worktrees,
            preview: Some(cleanup_worktrees_preview),
            timeout: Some(std::time::Duration::from_secs(120)),
        },
        EngineFn {
            name: "agents.restore_checkpoint",
            summary: "Restore the workspace to a chat checkpoint (message_index from agents.checkpoints), after the user approves; the card lists the files that change.",
            class: EngineClass::Destructive,
            input: || json!({"type":"object","additionalProperties":false,"required":["message_index"],"properties":{"message_index":{"type":"integer","minimum":0},"chat_id":{"type":"string","minLength":1,"description":"This chat by default."}}}),
            output: || Some(json!({"type":"object","required":["restored","files_changed","reverted_to","checkpoint_for_undo"],"properties":{"restored":{"type":"boolean"},"files_changed":{"type":"array"},"reverted_to":{"type":"string"},"checkpoint_for_undo":{"type":"object"}}})),
            handler: restore_checkpoint,
            preview: Some(restore_checkpoint_preview),
            timeout: Some(std::time::Duration::from_secs(120)),
        },
        EngineFn {
            name: "agents.forget_provenance",
            summary: "Forget recorded file provenance (who wrote which file) for one path or the whole project, after the user approves.",
            class: EngineClass::Destructive,
            input: || json!({"type":"object","additionalProperties":false,"properties":{"path":{"type":"string","minLength":1,"description":"Every recorded file when absent."}}}),
            output: || Some(json!({"type":"object","required":["removed"],"properties":{"removed":{"type":"integer"}}})),
            handler: forget_provenance,
            preview: Some(forget_provenance_preview),
            timeout: None,
        },
    ]
}

/// The summary serializes camelCase for the GUI; scripts get the snake_case names of every other output.
fn snake_keys(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| {
                    let mut snake = String::with_capacity(key.len() + 4);
                    for c in key.chars() {
                        if c.is_ascii_uppercase() {
                            snake.push('_');
                            snake.push(c.to_ascii_lowercase());
                        } else {
                            snake.push(c);
                        }
                    }
                    (snake, value)
                })
                .collect(),
        ),
        other => other,
    }
}

/// Agent ids and chat ids (root and child chats) of this chat's tree.
pub(super) async fn own_tree(ctx: &EngineCtx) -> (HashSet<String>, HashSet<String>) {
    let root = ctx.ccx.lock().await.root_chat_id.clone();
    let root = if root.is_empty() {
        ctx.chat_id.clone()
    } else {
        root
    };
    let records = crate::tools::tool_agent_interact::records_for_root(&ctx.app, &root).await;
    let mut chats: HashSet<String> = [root, ctx.chat_id.clone()].into_iter().collect();
    let mut agents = HashSet::new();
    for (record, _) in records {
        if let Some(child) = &record.child_chat_id {
            chats.insert(child.clone());
        }
        agents.insert(record.agent_id);
    }
    (agents, chats)
}

fn list<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        require_root_chat(ctx, "agents.list").await?;
        let (own, _) = own_tree(ctx).await;
        let statuses: Option<Vec<String>> = args.get("status").and_then(Value::as_array).map(|s| {
            s.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        });
        let root_filter = args.get("root_chat_id").and_then(Value::as_str);
        let mut rows = Vec::new();
        for record in ctx.app.agents.list_all().await {
            if root_filter.is_some_and(|root| {
                record
                    .parent_root_chat_id
                    .as_deref()
                    .unwrap_or(&record.parent_chat_id)
                    != root
            }) {
                continue;
            }
            let mut row = snake_keys(to_value(refact_chat_api::BackgroundAgentSummary::from(
                &record,
            ))?);
            if statuses.as_ref().is_some_and(|s| {
                !row["status"]
                    .as_str()
                    .is_some_and(|st| s.iter().any(|x| x == st))
            }) {
                continue;
            }
            if own.contains(&record.agent_id) {
                for key in ["error", "progress", "result_summary"] {
                    if let Some(text) = row[key].as_str() {
                        row[key] = json!(redact(text));
                    }
                }
            } else if let Some(map) = row.as_object_mut() {
                map.retain(|key, _| FOREIGN_KEEP.contains(&key.as_str()));
            }
            rows.push(row);
        }
        let total = rows.len();
        let running = rows.iter().filter(|r| r["status"] == "running").count();
        let in_tree = rows
            .iter()
            .filter(|r| r["agent_id"].as_str().is_some_and(|id| own.contains(id)))
            .count();
        let limit = limit(args);
        rows.truncate(limit);
        let mut own_ids: Vec<&String> = own.iter().collect();
        own_ids.sort();
        Ok(EngineOutput::new(
            json!({"agents":rows,"own_tree":own_ids,"total":total,"truncated":total > limit}),
            format!("{total} agents ({running} running), {in_tree} in this chat's tree"),
        ))
    })
}

fn wait_graph<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    use crate::coordination::service;
    Box::pin(async move {
        require_root_chat(ctx, "agents.wait_graph").await?;
        let model = ctx.ccx.lock().await.current_model.clone();
        let caller = crate::coordination::scope::model_caller(&ctx.app, &ctx.chat_id, &model)
            .await
            .map_err(|e| EngineError::Failed(e.message))?;
        let inspection = service::inspect(
            &ctx.app,
            &caller,
            service::InspectRequest {
                selector: service::InspectSelector::WaitGraph {},
                cursor: args.get("cursor").and_then(Value::as_str).map(String::from),
                limit: args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .map(|n| n as usize),
            },
        )
        .await
        .map_err(|e| {
            if e.code == "invalid" || e.code == "invalid_request" {
                EngineError::Type(e.message)
            } else {
                EngineError::Failed(e.message)
            }
        })?;
        let data = to_value(&inspection)?;
        let graph = &data["wait_graph"];
        let text = format!(
            "{} waits, {} deadlock candidates",
            graph["edges"].as_array().map_or(0, Vec::len),
            graph["deadlock"]["cycle"]
                .as_array()
                .map_or(usize::from(!graph["deadlock"].is_null()), Vec::len)
        );
        Ok(EngineOutput::new(data, text))
    })
}

fn attention<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        require_root_chat(ctx, "agents.attention").await?;
        let (agents, chats) = own_tree(ctx).await;
        let queue = ctx.app.attention().queue(chrono::Utc::now());
        let mut data = to_value(&queue)?;
        let ours = |waiter: &str| {
            agents.contains(waiter)
                || waiter
                    .strip_prefix("chat:")
                    .is_some_and(|chat| chats.contains(chat))
        };
        let mut irreversible = 0;
        for list in ["waiting", "settled"] {
            for entry in data[list].as_array_mut().into_iter().flatten() {
                let own = entry["waiters"]
                    .as_array()
                    .is_some_and(|w| w.iter().any(|w| w.as_str().is_some_and(&ours)))
                    || entry["asked_by"].as_str().is_some_and(&ours);
                if list == "waiting" && entry["irreversible"] == true {
                    irreversible += 1;
                }
                if own {
                    for key in ["text", "default_answer", "default_used"] {
                        if let Some(text) = entry[key].as_str() {
                            entry[key] = json!(redact(text));
                        }
                    }
                } else if let Some(map) = entry.as_object_mut() {
                    map.remove("text");
                    map.remove("default_answer");
                    map.remove("default_used");
                    map.insert("own".into(), json!(false));
                }
            }
        }
        let text = format!(
            "{} waiting for the user ({irreversible} irreversible), {} settled",
            data["waiting"].as_array().map_or(0, Vec::len),
            data["settled"].as_array().map_or(0, Vec::len)
        );
        Ok(EngineOutput::new(data, text))
    })
}

fn tasks<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    use crate::tasks::storage;
    Box::pin(async move {
        require_root_chat(ctx, "agents.tasks").await?;
        let gcx = ctx.app.gcx.clone();
        if let Some(task_id) = args.get("task_id").and_then(Value::as_str) {
            let meta = storage::load_task_meta(gcx.clone(), task_id)
                .await
                .map_err(|_| EngineError::Type(format!("no task with id {task_id:?}")))?;
            let board = storage::load_board(gcx.clone(), task_id)
                .await
                .map_err(failed)?;
            let instructions = storage::load_planner_instructions(gcx, task_id)
                .await
                .map_err(failed)?;
            let text = format!(
                "task {} ({:?}), {} cards",
                meta.name,
                meta.status,
                board.cards.len()
            );
            return Ok(EngineOutput::new(
                json!({"meta":meta,"board_summary":{"rev":board.rev,"cards_count":board.cards.len(),"ready":board.get_ready_cards().ready},"planner_instructions":instructions}),
                text,
            ));
        }
        let all = crate::http::routers::v1::tasks::list_tasks_with_session_state(gcx)
            .await
            .map_err(failed)?;
        let total = all.len();
        let active = all
            .iter()
            .filter(|t| t.status == TaskStatus::Active)
            .count();
        let limit = limit(args);
        let rows: Vec<_> = all.into_iter().take(limit).collect();
        Ok(EngineOutput::new(
            json!({"tasks":rows,"total":total,"truncated":total > limit}),
            format!("{total} tasks ({active} active)"),
        ))
    })
}

async fn service(
    ctx: &EngineCtx,
    args: &Value,
) -> Result<crate::worktrees::service::WorktreeService, EngineError> {
    crate::http::routers::v1::worktrees::service_for_request(
        &ctx.app,
        args.get("source_workspace_root")
            .and_then(Value::as_str)
            .map(String::from),
    )
    .await
    .map_err(|(status, body)| {
        let message = body.0["error"]["message"]
            .as_str()
            .or(body.0["message"].as_str())
            .unwrap_or("worktree service unavailable")
            .to_string();
        if status == hyper::StatusCode::BAD_REQUEST {
            EngineError::Type(message)
        } else {
            EngineError::Failed(message)
        }
    })
}

fn worktrees<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        require_root_chat(ctx, "agents.worktrees").await?;
        let inventory = service(ctx, args)
            .await?
            .inspect_worktrees()
            .await
            .map_err(failed)?;
        let mut data = to_value(&inventory)?;
        let total = inventory.worktrees.len();
        let limit = limit(args);
        if let Some(rows) = data["worktrees"].as_array_mut() {
            rows.truncate(limit);
        }
        data["total"] = json!(total);
        data["truncated"] = json!(total > limit);
        Ok(EngineOutput::new(
            data,
            format!(
                "{total} worktrees, {} cleanup candidates",
                inventory.cleanup_candidates.len()
            ),
        ))
    })
}

/// Undoes git's C-style quoting of a path (`"a/cl\303\251.env"`); `None` when malformed.
fn unquote_git_path(quoted: &str) -> Option<String> {
    let inner = quoted.strip_prefix('"')?.strip_suffix('"')?;
    let mut bytes = Vec::new();
    let mut chars = inner.bytes().peekable();
    while let Some(b) = chars.next() {
        if b != b'\\' {
            bytes.push(b);
            continue;
        }
        match chars.next()? {
            b'n' => bytes.push(b'\n'),
            b't' => bytes.push(b'\t'),
            b'"' => bytes.push(b'"'),
            b'\\' => bytes.push(b'\\'),
            b'a' => bytes.push(7),
            b'b' => bytes.push(8),
            b'f' => bytes.push(12),
            b'r' => bytes.push(b'\r'),
            b'v' => bytes.push(11),
            d @ b'0'..=b'7' => {
                let mut value = (d - b'0') as u32;
                for _ in 0..2 {
                    let next = chars.next()?;
                    if !(b'0'..=b'7').contains(&next) {
                        return None;
                    }
                    value = value * 8 + (next - b'0') as u32;
                }
                bytes.push(u8::try_from(value).ok()?);
            }
            _ => return None,
        }
    }
    String::from_utf8(bytes).ok()
}

/// A path as git prints it in `--name-status`: plain, or C-quoted.
pub(super) fn git_listed_path(path: &str) -> Option<String> {
    if path.starts_with('"') {
        unquote_git_path(path)
    } else {
        Some(path.to_string())
    }
}

/// The file of a `diff --git` header, or `None` when it cannot be read with certainty.
fn header_path(rest: &str) -> Option<String> {
    let rest = rest.trim_end_matches(['\n', '\r']);
    if rest.starts_with('"') {
        let end = rest[1..].find("\" ")? + 1;
        let a = unquote_git_path(&rest[..=end])?;
        let b = unquote_git_path(rest[end + 2..].trim())?;
        return (a.strip_prefix("a/")? == b.strip_prefix("b/")?).then(|| b[2..].to_string());
    }
    if rest.ends_with('"') {
        return None;
    }
    let body = rest.strip_prefix("a/")?;
    if body.len() < 3 || (body.len() - 3) % 2 != 0 {
        return None;
    }
    let half = (body.len() - 3) / 2;
    let (a, b) = (body.get(..half)?, body.get(half..)?);
    let b = b.strip_prefix(" b/")?;
    (a == b).then(|| b.to_string())
}

/// Splits a git patch into per-file sections; a section whose file is uncertain has `None`.
fn patch_sections(patch: &str) -> Vec<(Option<Option<String>>, String)> {
    let mut sections: Vec<(Option<Option<String>>, String)> = Vec::new();
    for line in patch.split_inclusive('\n') {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            sections.push((Some(header_path(rest)), String::new()));
        }
        match sections.last_mut() {
            Some((_, text)) => text.push_str(line),
            None => sections.push((None, line.to_string())),
        }
    }
    sections
}

/// The patch and file list of `diff` without the files `allowed` refuses; files whose path
/// cannot be read with certainty are withheld and counted.
pub(super) fn filter_diff(
    raw_patch: &str,
    raw_files: &[crate::worktrees::types::WorktreeDiffFile],
    allowed: impl Fn(&str) -> bool,
) -> (
    String,
    Vec<crate::worktrees::types::WorktreeDiffFile>,
    Vec<String>,
    usize,
) {
    let mut withheld = Vec::new();
    let mut patch = String::new();
    let mut unreadable = 0usize;
    for (path, text) in patch_sections(raw_patch) {
        match path {
            None => patch.push_str(&text),
            Some(Some(path)) if allowed(&path) => patch.push_str(&text),
            Some(Some(path)) => {
                if !withheld.contains(&path) {
                    withheld.push(path);
                }
            }
            Some(None) => unreadable += 1,
        }
    }

    let mut files = Vec::new();
    for file in raw_files {
        let old_path = file.old_path.as_deref().map(git_listed_path);
        if let Some(old) = &old_path {
            match old {
                Some(old) if allowed(old) => {}
                Some(old) => {
                    if !withheld.contains(old) {
                        withheld.push(old.clone());
                    }
                    continue;
                }
                None => {
                    unreadable += 1;
                    continue;
                }
            }
        }
        match git_listed_path(&file.path) {
            Some(path) if allowed(&path) && !withheld.contains(&path) => {
                let mut entry = file.clone();
                entry.path = path;
                entry.old_path = old_path.flatten();
                files.push(entry);
            }
            Some(path) => {
                if !withheld.contains(&path) {
                    withheld.push(path);
                }
            }
            None => unreadable += 1,
        }
    }
    if unreadable > 0 {
        patch.push_str(&format!(
            "\n[{unreadable} files withheld: their path could not be read with certainty]\n"
        ));
    }
    (patch, files, withheld, unreadable)
}

fn worktree_diff<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        require_root_chat(ctx, "agents.worktree_diff").await?;
        let id = args["worktree_id"].as_str().unwrap_or_default();
        let max = args
            .get("max_patch_bytes")
            .and_then(Value::as_u64)
            .unwrap_or(200_000) as usize;
        let service = service(ctx, args).await?;
        let root = service
            .get_worktree(id)
            .await
            .map_err(EngineError::Type)?
            .meta
            .root;
        let diff = service
            .diff_worktree_with_limit(id, max)
            .await
            .map_err(failed)?;
        let filter = PathFilter::new(ctx).await?;
        let allowed = |path: &str| filter.allows(&root.join(path).to_string_lossy());
        let (patch, files, withheld, unreadable) = filter_diff(&diff.patch, &diff.files, allowed);
        let visible: Vec<std::path::PathBuf> = files.iter().map(|f| root.join(&f.path)).collect();
        let mut data = to_value(&diff)?;
        data["files"] = to_value(&files)?;
        data["unreadable_withheld"] = json!(unreadable);
        data["patch"] = json!(redact(&patch));
        data["withheld_files"] = json!(withheld);
        let mut output = EngineOutput::new(Value::Null, "");
        if !visible.is_empty() {
            let records = crate::privacy::records::declared_file_records(&ctx.app.gcx, visible)
                .map_err(|e| {
                    EngineError::Failed(format!(
                        "privacy records of the diff could not be made: {e}"
                    ))
                })?;
            output.privacy.push(json!({"files":to_value(records)?}));
        }
        output.text = format!(
            "{} files changed (+{} -{})",
            diff.files.len(),
            data["stats"]["additions"].as_u64().unwrap_or_default(),
            data["stats"]["deletions"].as_u64().unwrap_or_default()
        );
        output.data = data;
        Ok(output)
    })
}

fn checkpoints<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        require_root_chat(ctx, "agents.checkpoints").await?;
        let chat_id = args
            .get("chat_id")
            .and_then(Value::as_str)
            .unwrap_or(&ctx.chat_id)
            .to_string();
        let messages = if chat_id == ctx.chat_id {
            ctx.ccx.lock().await.messages.clone()
        } else {
            let session = ctx
                .app
                .chat
                .sessions
                .read()
                .await
                .get(&chat_id)
                .cloned()
                .ok_or_else(|| EngineError::Type(format!("no open chat {chat_id}")))?;
            let session = session.try_lock().map_err(|_| {
                EngineError::Failed(format!("chat {chat_id} is busy; try again when it is idle"))
            })?;
            if session.runtime.state != refact_chat_api::SessionState::Idle {
                return Err(EngineError::Failed(format!(
                    "chat {chat_id} is not idle; another chat's checkpoints are read only while it is idle"
                )));
            }
            session.messages.clone()
        };
        let rows: Vec<Value> = messages
            .iter()
            .enumerate()
            .flat_map(|(index, message)| {
                message.checkpoints.iter().map(move |c| {
                    json!({"message_index":index,"workspace_folder":c.workspace_folder.to_string_lossy(),"commit_hash":c.commit_hash})
                })
            })
            .collect();
        let total = rows.len();
        let limit = limit(args);
        let rows: Vec<Value> = rows.into_iter().rev().take(limit).collect();
        Ok(EngineOutput::new(
            json!({"checkpoints":rows,"total":total,"truncated":total > limit}),
            format!("{total} checkpoints"),
        ))
    })
}

fn status_name(status: TaskStatus) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default()
}

/// (argument, task meta field)
const FIELDS: &[(&str, &str)] = &[
    ("status", "status"),
    ("task_name", "name"),
    ("base_branch", "base_branch"),
    ("base_commit", "base_commit"),
    ("default_agent_model", "default_agent_model"),
];

async fn git_has(root: &Path, rev: &str) -> bool {
    tokio::process::Command::new("git")
        .args([
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{rev}^{{commit}}"),
        ])
        .current_dir(root)
        .output()
        .await
        .is_ok_and(|o| o.status.success())
}

struct TaskPlan {
    name: String,
    old: Map<String, Value>,
    new: Map<String, Value>,
}

async fn plan_set_task(ctx: &EngineCtx, args: &Value) -> Result<TaskPlan, EngineError> {
    use crate::tasks::storage;
    require_root_chat(ctx, "agents.set_task").await?;
    let task_id = args["task_id"].as_str().unwrap_or_default();
    if !FIELDS.iter().any(|(arg, _)| args.get(*arg).is_some()) {
        return Err(EngineError::Type(format!(
            "agents.set_task needs at least one of {}",
            FIELDS
                .iter()
                .map(|(arg, _)| *arg)
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    let gcx = ctx.app.gcx.clone();
    let meta = storage::load_task_meta(gcx.clone(), task_id)
        .await
        .map_err(|_| EngineError::Type(format!("no task with id {task_id:?}")))?;
    let current = to_value(&meta)?;
    let mut old = Map::new();
    let mut new = Map::new();
    for (arg, field) in FIELDS {
        if let Some(value) = args.get(*arg) {
            old.insert(field.to_string(), current[*field].clone());
            new.insert(field.to_string(), value.clone());
        }
    }
    let root = ctx
        .project_root
        .clone()
        .ok_or_else(|| EngineError::Type("no project is open".into()))?;
    for key in ["base_branch", "base_commit"] {
        if let Some(rev) = args.get(key).and_then(Value::as_str) {
            if rev.starts_with('-') || !git_has(&root, rev).await {
                return Err(EngineError::Type(format!(
                    "{key} {rev:?} does not name a commit in {}",
                    root.display()
                )));
            }
        }
    }
    if let Some(model) = args.get("default_agent_model").and_then(Value::as_str) {
        let caps = crate::global_context::try_load_caps_quickly_if_not_present(gcx.clone(), 0)
            .await
            .map_err(|e| {
                EngineError::Failed(format!("model capabilities unavailable: {}", e.message))
            })?;
        crate::caps::resolve_chat_model(caps, model)
            .map_err(|e| EngineError::Type(format!("unknown model {model:?}: {e}")))?;
    }
    if args.get("status").and_then(Value::as_str) == Some("completed") {
        let board = storage::load_board(gcx, task_id).await.map_err(failed)?;
        let doing = board.cards.iter().filter(|c| c.column == "doing").count();
        if doing > 0 {
            return Err(EngineError::Type(format!(
                "{doing} cards are still in doing; finish or move them before completing the task"
            )));
        }
    }
    Ok(TaskPlan {
        name: meta.name,
        old,
        new,
    })
}

fn set_task_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let plan = plan_set_task(ctx, args).await?;
        let abandon = args.get("status").and_then(Value::as_str) == Some("abandoned")
            && plan.old.get("status").and_then(Value::as_str) != Some("abandoned");
        let items = plan
            .new
            .iter()
            .map(|(field, new)| {
                let old = plan.old.get(field).cloned().filter(|v| !v.is_null());
                item(field, old, Some(new.clone()))
            })
            .collect();
        let approved = previewed::plan(Value::Object(plan.old.clone()));
        Ok(Preview {
            plan: approved,
            title: "change a task".into(),
            items,
            scope: Some(format!("task {}", plan.name)),
            class: if abandon {
                EngineClass::Destructive
            } else {
                EngineClass::Change
            },
            warning: abandon.then(|| {
                "Abandoning the task stops planning on it; running agents are not cancelled and their work stays unmerged.".to_string()
            }),
        })
    })
}

fn set_task<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    use crate::http::routers::v1::tasks::{update_task_meta, TaskMetaPatch};
    Box::pin(async move {
        let plan = plan_set_task(ctx, args).await?;
        previewed::take(ctx, "agents.set_task", &Value::Object(plan.old.clone()))?;
        let text_of = |key: &str| args.get(key).and_then(Value::as_str).map(String::from);
        let status = match args.get("status") {
            Some(status) => Some(
                serde_json::from_value::<TaskStatus>(status.clone())
                    .map_err(|e| EngineError::Type(format!("status: {e}")))?,
            ),
            None => None,
        };
        let meta = update_task_meta(
            ctx.app.gcx.clone(),
            args["task_id"].as_str().unwrap_or_default(),
            TaskMetaPatch {
                status,
                name: text_of("task_name"),
                base_branch: text_of("base_branch"),
                base_commit: text_of("base_commit"),
                default_agent_model: text_of("default_agent_model"),
            },
        )
        .await
        .map_err(|(_, message)| EngineError::Failed(message))?;
        let text = format!("task {} updated ({})", meta.name, status_name(meta.status));
        Ok(EngineOutput::new(to_value(&meta)?, text))
    })
}

const CANCEL_WARNING: &str =
    "The agent's run stops now and its unfinished work cannot be resumed from this point.";
const DELETE_TASK_WARNING: &str = "The task folder with its board, documents and planner trajectories is removed and cannot be restored by the engine.";
const CLEANUP_WARNING: &str =
    "Deleted worktrees and their uncommitted files cannot be restored by the engine.";
const RESTORE_WARNING: &str = "Files changed after the checkpoint are overwritten; only the undo checkpoint created now can bring them back.";
const FORGET_WARNING: &str = "Recorded authorship of file changes is erased, so later reviews cannot tell which agent wrote these files.";

/// The run's privacy records (this chat's carried records, the session taint and the run's reads)
/// on `message`, so the text keeps them wherever it is delivered.
async fn carry_run_records(
    ctx: &EngineCtx,
    message: &mut crate::call_validation::ChatMessage,
) -> Result<(), EngineError> {
    let mut carrier = crate::call_validation::ChatMessage::default();
    let (history, derived) = {
        let context = ctx.ccx.lock().await;
        (
            context.messages.clone(),
            context.derived_privacy_zones.clone(),
        )
    };
    crate::privacy::records::carry_records_into(&mut carrier, &history)
        .map_err(|e| EngineError::Failed(e.to_string()))?;
    let (taint, reads) = {
        let run = ctx.run_privacy.lock().unwrap_or_else(|e| e.into_inner());
        (run.taint.clone(), run.reads.clone())
    };
    crate::python_tool::run_output::merge_taint(&mut carrier, &taint)
        .map_err(EngineError::Failed)?;
    let direct = crate::privacy::records::observed_file_records_with_derived(
        &ctx.app.gcx,
        reads.iter().map(std::path::PathBuf::from),
        &derived,
    )
    .map_err(EngineError::Failed)?;
    crate::privacy::records::merge_records(&mut carrier, direct);
    if let Some(privacy) = carrier.extra.get("privacy") {
        let records: refact_privacy::PrivacyRecord =
            serde_json::from_value(privacy.clone()).map_err(failed)?;
        crate::privacy::records::merge_records(message, records.files);
    }
    Ok(())
}

/// The model task planners run on: the task planner slot, else the default chat model.
async fn planner_model(ctx: &EngineCtx) -> Result<String, EngineError> {
    let caps = crate::global_context::try_load_caps_quickly_if_not_present(ctx.app.gcx.clone(), 0)
        .await
        .map_err(|e| EngineError::Failed(format!("the planner's model is unknown: {e:?}")))?;
    let model = [
        caps.defaults.task_planner_agent_model.trim(),
        caps.defaults.chat_default_model.trim(),
    ]
    .into_iter()
    .find(|m| !m.is_empty())
    .map(String::from);
    model.ok_or_else(|| EngineError::Failed("no task planner model is configured".into()))
}

/// A running agent of another chat's tree; own-tree and finished agents are refused.
async fn foreign_agent(
    ctx: &EngineCtx,
    agent_id: &str,
    own_tool: &str,
) -> Result<crate::agents::types::BackgroundAgent, EngineError> {
    let record = ctx
        .app
        .agents
        .get_any(agent_id)
        .await
        .map_err(|_| EngineError::Type(format!("no background agent {agent_id}")))?;
    let (own, _) = own_tree(ctx).await;
    if own.contains(agent_id) {
        return Err(EngineError::Type(format!(
            "agent {agent_id} belongs to this chat's tree; use {own_tool}"
        )));
    }
    if record.status.is_terminal() {
        return Err(EngineError::Failed(format!(
            "agent {agent_id} already ended ({:?})",
            record.status
        )));
    }
    Ok(record)
}

/// What identifies a run to cancel: the agent, its run generation and that it is still running;
/// counters that move while it works are left out.
fn agent_state(record: &crate::agents::types::BackgroundAgent) -> Value {
    json!({
        "agent": record.agent_id,
        "run_generation": record.run_generation,
        "running": !record.status.is_terminal(),
    })
}

fn message_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        require_root_chat(ctx, "agents.message").await?;
        let agent_id = args["agent_id"].as_str().unwrap_or_default();
        let text = args["text"].as_str().unwrap_or_default().trim().to_string();
        let max = ctx.settings.agent_message_max_chars;
        if text.is_empty() || text.chars().count() > max {
            return Err(EngineError::Type(format!(
                "text must hold 1 to {max} characters"
            )));
        }
        let record = foreign_agent(ctx, agent_id, "agent_message").await?;
        super::knowledge::egress(
            ctx,
            crate::privacy::records::provider_destination(&record.model),
            "the message",
        )
        .await?;
        let owner = record
            .parent_root_chat_id
            .clone()
            .unwrap_or_else(|| record.parent_chat_id.clone());
        Ok(Preview {
            title: "message another chat's agent".into(),
            items: vec![
                item(
                    "agent",
                    None,
                    Some(json!(format!("{} ({agent_id})", record.title))),
                ),
                item("owner", None, Some(json!(owner))),
                item("message", None, Some(json!(text))),
                item(
                    "delivery",
                    None,
                    Some(json!(match args.get("push").and_then(Value::as_str) {
                        Some("preempt") => "preempt: interrupts the agent's current turn",
                        Some("when_idle") => "when_idle: after the agent's current work",
                        _ => "append: at the agent's next step",
                    })),
                ),
            ],
            scope: Some(format!("background agent {agent_id}")),
            class: EngineClass::Change,
            warning: None,
            plan: previewed::plan(json!({"agent": agent_id, "running": true})),
        })
    })
}

fn message<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    use refact_core::chat_types::{PendingDelivery, PushMode};
    Box::pin(async move {
        let agent_id = args["agent_id"].as_str().unwrap_or_default();
        let record = ctx
            .app
            .agents
            .get_any(agent_id)
            .await
            .map_err(|_| EngineError::Failed(format!("agent {agent_id} is gone")))?;
        previewed::take(
            ctx,
            "agents.message",
            &json!({"agent": agent_id, "running": !record.status.is_terminal()}),
        )?;
        let text = args["text"].as_str().unwrap_or_default().trim();
        let push = match args.get("push").and_then(Value::as_str) {
            Some("preempt") => PushMode::Preempt,
            Some("when_idle") => PushMode::WhenIdle,
            _ => PushMode::Append,
        };
        super::knowledge::egress(
            ctx,
            crate::privacy::records::provider_destination(&record.model),
            "the message",
        )
        .await?;
        let mut notice = crate::chat::internal_roles::event(
            crate::chat::internal_roles::EventSubkind::SystemNotice,
            crate::agents::push::NOTICE_MESSAGE,
            json!({"from": "user"}),
            format!("[message from user]\n{text}"),
        );
        carry_run_records(ctx, &mut notice).await?;
        let outcome = crate::agents::delivery::deliver_to_agent(
            ctx.app.clone(),
            agent_id,
            PendingDelivery::new(
                vec![notice],
                push,
                crate::agents::push::NOTICE_MESSAGE.to_string(),
                true,
            ),
        )
        .await
        .map_err(EngineError::Failed)?;
        crate::agents::spawn::emit_background_agent_update(ctx.app.clone(), &record).await;
        let summary = crate::agents::types::BackgroundAgentSummary::from(&record);
        Ok(EngineOutput::new(
            json!({"agent": snake_keys(to_value(&summary)?), "queued": true, "outcome": outcome}),
            format!("message queued for {}", record.title),
        ))
    })
}

fn set_planner_instructions_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    use crate::tasks::storage;
    Box::pin(async move {
        require_root_chat(ctx, "agents.set_planner_instructions").await?;
        let task_id = args["task_id"].as_str().unwrap_or_default();
        let content = args["content"].as_str().unwrap_or_default();
        let gcx = ctx.app.gcx.clone();
        let meta = storage::load_task_meta(gcx.clone(), task_id)
            .await
            .map_err(|_| EngineError::Type(format!("no task with id {task_id:?}")))?;
        let old = storage::load_planner_instructions(gcx, task_id)
            .await
            .map_err(|e| {
                EngineError::Failed(format!(
                    "the current planner instructions cannot be read: {e}"
                ))
            })?;
        if old == content {
            return Ok(Preview {
                title: "replace planner instructions".into(),
                items: Vec::new(),
                scope: Some(format!("task {}", meta.name)),
                class: EngineClass::Change,
                warning: None,
                plan: super::Plan::nothing(),
            });
        }
        let model = planner_model(ctx).await?;
        super::knowledge::egress(
            ctx,
            crate::privacy::records::provider_destination(&model),
            "the planner instructions",
        )
        .await?;
        Ok(Preview {
            title: "replace planner instructions".into(),
            items: vec![
                item(
                    "instructions",
                    (!old.is_empty()).then(|| json!(old)),
                    Some(json!(content)),
                ),
                item(
                    "read by",
                    None,
                    Some(json!(format!("the task planner on {model}"))),
                ),
            ],
            scope: Some(format!("task {}", meta.name)),
            class: EngineClass::Change,
            warning: None,
            plan: previewed::plan(json!({"instructions": super::digest(old.as_bytes())})),
        })
    })
}

fn set_planner_instructions<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    use crate::tasks::storage;
    Box::pin(async move {
        let task_id = args["task_id"].as_str().unwrap_or_default();
        let content = args["content"].as_str().unwrap_or_default();
        let gcx = ctx.app.gcx.clone();
        let current = storage::load_planner_instructions(gcx.clone(), task_id)
            .await
            .map_err(|e| {
                EngineError::Failed(format!(
                    "the current planner instructions cannot be read: {e}"
                ))
            })?;
        previewed::take(
            ctx,
            "agents.set_planner_instructions",
            &json!({"instructions": super::digest(current.as_bytes())}),
        )?;
        super::knowledge::egress(
            ctx,
            crate::privacy::records::provider_destination(&planner_model(ctx).await?),
            "the planner instructions",
        )
        .await?;
        storage::save_planner_instructions(gcx, task_id, content)
            .await
            .map_err(EngineError::Failed)?;
        Ok(EngineOutput::new(
            json!({"task_id": task_id, "saved": true, "bytes": content.len()}),
            "planner instructions saved",
        ))
    })
}

async fn cancel_plan(
    ctx: &EngineCtx,
    args: &Value,
) -> Result<(crate::agents::types::BackgroundAgent, Vec<String>), EngineError> {
    require_root_chat(ctx, "agents.cancel").await?;
    let agent_id = args["agent_id"].as_str().unwrap_or_default();
    let record = foreign_agent(ctx, agent_id, "agent_cancel").await?;
    let subtree = args.get("subtree").and_then(Value::as_bool) != Some(false);
    let mut running = Vec::new();
    if subtree {
        let all = ctx.app.agents.list_all().await;
        let mut frontier = vec![record.child_chat_id.clone().unwrap_or_default()];
        while let Some(chat) = frontier.pop() {
            for child in all
                .iter()
                .filter(|a| !chat.is_empty() && a.parent_chat_id == chat)
            {
                if !child.status.is_terminal() {
                    running.push(child.agent_id.clone());
                }
                if let Some(next) = &child.child_chat_id {
                    frontier.push(next.clone());
                }
            }
        }
    }
    running.sort();
    Ok((record, running))
}

fn cancel_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let (record, running) = cancel_plan(ctx, args).await?;
        let owner = record
            .parent_root_chat_id
            .clone()
            .unwrap_or_else(|| record.parent_chat_id.clone());
        let mut items = vec![item(
            "agent",
            Some(json!(format!(
                "{} ({}, {:?})",
                record.title, record.agent_id, record.status
            ))),
            Some(json!("cancelled")),
        )];
        if args.get("subtree").and_then(Value::as_bool) != Some(false) {
            items.push(item(
                "sub-agents",
                Some(json!(format!(
                    "{} running: {}",
                    running.len(),
                    running.join(", ")
                ))),
                Some(json!("cancelled")),
            ));
        }
        Ok(Preview {
            title: "cancel another chat's agent".into(),
            items,
            scope: Some(format!("chat {owner}")),
            class: EngineClass::Destructive,
            warning: Some(CANCEL_WARNING.into()),
            plan: previewed::plan(json!({"agent": agent_state(&record), "sub_agents": running})),
        })
    })
}

fn cancel<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let (record, running) = cancel_plan(ctx, args).await.map_err(|e| match e {
            EngineError::Failed(m) | EngineError::Type(m) | EngineError::Permission(m) => {
                EngineError::Failed(format!(
                    "agents.cancel: changed since the preview ({m}); nothing was applied"
                ))
            }
        })?;
        previewed::take(
            ctx,
            "agents.cancel",
            &json!({"agent": agent_state(&record), "sub_agents": running}),
        )?;
        let subtree = args.get("subtree").and_then(Value::as_bool) != Some(false);
        let reason = args
            .get("reason")
            .and_then(Value::as_str)
            .map(String::from)
            .or_else(|| Some("cancelled from a script after the user approved".into()));
        let cancelled = ctx
            .app
            .agents
            .cancel_subtree(&record.parent_chat_id, &record.agent_id, subtree, reason)
            .await
            .map_err(EngineError::Failed)?;
        for agent in &cancelled {
            crate::agents::spawn::emit_background_agent_update(ctx.app.clone(), agent).await;
        }
        let ids: Vec<String> = cancelled.iter().map(|a| a.agent_id.clone()).collect();
        let agents: Vec<Value> = cancelled
            .iter()
            .map(|a| {
                to_value(&crate::agents::types::BackgroundAgentSummary::from(a)).map(snake_keys)
            })
            .collect::<Result<_, _>>()?;
        Ok(EngineOutput::new(
            json!({"cancelled_agent_ids": ids, "agents": agents}),
            format!("cancelled {} agents", ids.len()),
        ))
    })
}

struct TaskDeletePlan {
    name: String,
    state: Value,
    label: String,
}

async fn delete_task_plan(ctx: &EngineCtx, args: &Value) -> Result<TaskDeletePlan, EngineError> {
    use crate::tasks::storage;
    require_root_chat(ctx, "agents.delete_task").await?;
    let task_id = args["task_id"].as_str().unwrap_or_default();
    let force = args.get("force").and_then(Value::as_bool) == Some(true);
    if ctx
        .ccx
        .lock()
        .await
        .task_meta
        .as_ref()
        .is_some_and(|meta| meta.task_id == task_id)
    {
        return Err(EngineError::Permission(
            "this chat belongs to that task; a task cannot be deleted from inside it".into(),
        ));
    }
    let gcx = ctx.app.gcx.clone();
    let meta = storage::load_task_meta(gcx.clone(), task_id)
        .await
        .map_err(|_| EngineError::Type(format!("no task with id {task_id:?}")))?;
    let board = storage::load_board(gcx.clone(), task_id)
        .await
        .map_err(failed)?;
    let active: Vec<String> = board
        .cards
        .iter()
        .filter(|c| c.column == "doing" || c.agent_worktree.is_some() || c.agent_branch.is_some())
        .map(|c| c.id.clone())
        .collect();
    if !active.is_empty() && !force {
        return Err(EngineError::Type(format!(
            "task has active agent cards: {}; pass force=true to delete it anyway",
            active.join(", ")
        )));
    }
    let running_agents = board
        .cards
        .iter()
        .any(|c| c.column == "doing" && c.agent_chat_id.is_some())
        && meta.agents_active > 0;
    if running_agents {
        return Err(EngineError::Permission(
            "agents of this task are running; cancel them first".into(),
        ));
    }
    let dir = storage::find_task_dir(gcx, task_id).await.map_err(failed)?;
    let docs = std::fs::read_dir(dir.join("documents"))
        .map(|entries| entries.count())
        .unwrap_or(0);
    let label = format!(
        "{} ({}, {} cards, {docs} docs)",
        meta.name,
        status_name(meta.status),
        board.cards.len()
    );
    Ok(TaskDeletePlan {
        name: meta.name.clone(),
        state: json!({"meta": to_value(&meta)?, "rev": board.rev, "cards": board.cards.len()}),
        label,
    })
}

fn delete_task_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let plan = delete_task_plan(ctx, args).await?;
        let root = ctx
            .project_root
            .as_ref()
            .map(|r| r.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok(Preview {
            title: "delete a task".into(),
            items: vec![item(
                "task",
                Some(json!(plan.label)),
                Some(json!("deleted")),
            )],
            scope: Some(format!("project {root}")),
            class: EngineClass::Destructive,
            warning: Some(DELETE_TASK_WARNING.into()),
            plan: previewed::plan(plan.state),
        })
    })
}

fn delete_task<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let plan = delete_task_plan(ctx, args).await?;
        previewed::take(ctx, "agents.delete_task", &plan.state)?;
        let task_id = args["task_id"].as_str().unwrap_or_default();
        crate::tasks::storage::delete_task(ctx.app.gcx.clone(), task_id)
            .await
            .map_err(EngineError::Failed)?;
        Ok(EngineOutput::new(
            json!({"task_id": task_id, "deleted": true}),
            format!("deleted task {}", plan.name),
        ))
    })
}

#[derive(Clone)]
struct CleanupPlan {
    request: crate::worktrees::types::WorktreeCleanupRequest,
}

async fn cleanup_plan(
    ctx: &EngineCtx,
    args: &Value,
) -> Result<
    (
        crate::worktrees::types::WorktreeCleanupRequest,
        crate::worktrees::types::WorktreeCleanupPlan,
    ),
    EngineError,
> {
    require_root_chat(ctx, "agents.cleanup_worktrees").await?;
    let clean_only = args.get("clean_only").and_then(Value::as_bool) != Some(false);
    let allow_shared = args.get("allow_shared").and_then(Value::as_bool) == Some(true);
    if !clean_only && allow_shared {
        return Err(EngineError::Type(
            "clean_only=false together with allow_shared=true would remove dirty shared checkouts; choose one".into(),
        ));
    }
    let service = service(ctx, args).await?;
    let ids: Vec<String> = match args.get("ids").and_then(Value::as_array) {
        Some(ids) => ids
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect(),
        None => {
            service
                .inspect_worktrees()
                .await
                .map_err(failed)?
                .cleanup_candidates
        }
    };
    let mut request = crate::worktrees::types::WorktreeCleanupRequest {
        ids,
        clean_only,
        delete_branches: args.get("delete_branches").and_then(Value::as_bool) == Some(true),
        allow_shared,
        min_age_hours: args
            .get("min_age_hours")
            .and_then(Value::as_u64)
            .unwrap_or(24),
        source_workspace_root: args
            .get("source_workspace_root")
            .and_then(Value::as_str)
            .map(String::from),
    };
    if request.ids.is_empty() {
        let plan = crate::worktrees::types::WorktreeCleanupPlan {
            generated_at: String::new(),
            request: request.clone(),
            candidates: Vec::new(),
            skipped: Vec::new(),
        };
        return Ok((request, plan));
    }
    let own_root = ctx
        .ccx
        .lock()
        .await
        .execution_scope
        .as_ref()
        .map(|scope| scope.effective_root().to_path_buf());
    let running: HashSet<String> = ctx
        .app
        .agents
        .list_all()
        .await
        .into_iter()
        .filter(|a| !a.status.is_terminal())
        .filter_map(|a| a.worktree_id)
        .collect();
    let mut plan = service
        .cleanup_worktrees_dry_run(request.clone())
        .await
        .map_err(EngineError::Type)?;
    plan.candidates
        .retain(|target| own_root.as_ref() != Some(&target.root) && !running.contains(&target.id));
    request.ids = plan.candidates.iter().map(|t| t.id.clone()).collect();
    Ok((request, plan))
}

fn cleanup_worktrees_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let (request, plan) = cleanup_plan(ctx, args).await?;
        let root = request
            .source_workspace_root
            .clone()
            .or_else(|| {
                ctx.project_root
                    .as_ref()
                    .map(|r| r.to_string_lossy().into_owned())
            })
            .unwrap_or_default();
        if plan.candidates.is_empty() {
            return Ok(Preview {
                title: "delete worktrees".into(),
                items: Vec::new(),
                scope: Some(format!("project {root}")),
                class: EngineClass::Destructive,
                warning: Some(CLEANUP_WARNING.into()),
                plan: super::Plan::nothing(),
            });
        }
        let dirty: Vec<&crate::worktrees::types::WorktreeCleanupTarget> = plan
            .candidates
            .iter()
            .filter(|t| t.changed_files > 0)
            .collect();
        let warning = if dirty.is_empty() {
            CLEANUP_WARNING.to_string()
        } else {
            format!(
                "{CLEANUP_WARNING} {} of these worktrees hold uncommitted work ({} files, +{} -{} lines) that is lost with them.",
                dirty.len(),
                dirty.iter().map(|t| t.changed_files).sum::<usize>(),
                dirty.iter().map(|t| t.additions).sum::<usize>(),
                dirty.iter().map(|t| t.deletions).sum::<usize>(),
            )
        };
        let items = plan
            .candidates
            .iter()
            .map(|target| {
                let work = if target.changed_files > 0 {
                    format!(
                        "; uncommitted: {} files, +{} -{} lines",
                        target.changed_files, target.additions, target.deletions
                    )
                } else {
                    "; no uncommitted changes".to_string()
                };
                named_item(
                    &target.id,
                    Some(json!(format!(
                        "{} at {}{work}",
                        target.branch.as_deref().unwrap_or("(detached)"),
                        target.root.display()
                    ))),
                    Some(json!(if target.delete_branch {
                        "deleted and branch"
                    } else {
                        "deleted"
                    })),
                )
            })
            .collect();
        let state = to_value(&plan.candidates)?;
        Ok(Preview {
            title: "delete worktrees".into(),
            items,
            scope: Some(format!("project {root}")),
            class: EngineClass::Destructive,
            warning: Some(warning),
            plan: super::Plan::new(
                state,
                CleanupPlan {
                    request,
                },
            ),
        })
    })
}

fn cleanup_worktrees<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let (_, current) = cleanup_plan(ctx, args).await?;
        let plan: CleanupPlan =
            ctx.take_plan("agents.cleanup_worktrees", &to_value(&current.candidates)?)?;
        let result = service(ctx, args)
            .await?
            .cleanup_worktrees(plan.request)
            .await
            .map_err(EngineError::Failed)?;
        let text = format!(
            "deleted {} worktrees, skipped {}",
            result.deleted.len(),
            result.skipped.len()
        );
        Ok(EngineOutput::new(to_value(&result)?, text))
    })
}

struct RestorePlan {
    checkpoint: refact_core::chat_types::Checkpoint,
    worktree_root: Option<std::path::PathBuf>,
    chat_id: String,
}

async fn restore_plan(ctx: &EngineCtx, args: &Value) -> Result<RestorePlan, EngineError> {
    require_root_chat(ctx, "agents.restore_checkpoint").await?;
    let chat_id = args
        .get("chat_id")
        .and_then(Value::as_str)
        .unwrap_or(&ctx.chat_id)
        .to_string();
    let index = args["message_index"].as_u64().unwrap_or_default() as usize;
    let messages = if chat_id == ctx.chat_id {
        ctx.ccx.lock().await.messages.clone()
    } else {
        let session = ctx
            .app
            .chat
            .sessions
            .read()
            .await
            .get(&chat_id)
            .cloned()
            .ok_or_else(|| EngineError::Type(format!("no open chat {chat_id}")))?;
        let session = session.try_lock().map_err(|_| {
            EngineError::Failed(format!("chat {chat_id} is busy; try again when it is idle"))
        })?;
        if session.runtime.state != refact_chat_api::SessionState::Idle {
            return Err(EngineError::Permission(format!(
                "chat {chat_id} is not idle; its checkpoints are restored only while it is idle"
            )));
        }
        session.messages.clone()
    };
    let message = messages
        .get(index)
        .ok_or_else(|| EngineError::Type(format!("no message {index} in chat {chat_id}")))?;
    let checkpoint = match message.checkpoints.as_slice() {
        [one] => one.clone(),
        [] => {
            return Err(EngineError::Type(format!(
                "message {index} has no checkpoint; agents.checkpoints lists them"
            )))
        }
        _ => {
            return Err(EngineError::Type(
                "restoring a message with several checkpoints is not supported".into(),
            ))
        }
    };
    let meta = crate::call_validation::ChatMeta {
        chat_id: chat_id.clone(),
        ..Default::default()
    };
    let worktree_root =
        crate::http::routers::v1::git::checkpoint_worktree_for_meta(ctx.app.gcx.clone(), &meta)
            .await
            .map(|w| w.root);
    Ok(RestorePlan {
        checkpoint,
        worktree_root,
        chat_id,
    })
}

/// Files a restore lists on its card before it says how many more there are.
const RESTORE_LISTED: usize = 50;

/// What a restore will overwrite: each file with its status and a digest of its current bytes.
async fn restore_targets(
    ctx: &EngineCtx,
    plan: &RestorePlan,
) -> Result<
    (
        Vec<crate::git::FileChange>,
        chrono::DateTime<chrono::Utc>,
        refact_core::chat_types::Checkpoint,
        Value,
    ),
    EngineError,
> {
    use crate::git::checkpoints::{
        preview_changes_for_workspace_checkpoint, preview_changes_for_workspace_checkpoint_for_root,
    };
    let gcx = ctx.app.gcx.clone();
    let (mut files, reverted_to, undo) = match &plan.worktree_root {
        Some(root) => {
            preview_changes_for_workspace_checkpoint_for_root(
                gcx,
                root,
                &plan.checkpoint,
                &plan.chat_id,
            )
            .await
        }
        None => {
            preview_changes_for_workspace_checkpoint(gcx, &plan.checkpoint, &plan.chat_id).await
        }
    }
    .map_err(EngineError::Failed)?;
    files.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    let mut state = Vec::new();
    for change in &files {
        let digest = match tokio::fs::read(&change.absolute_path).await {
            Ok(bytes) => Some(super::digest(&bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                return Err(EngineError::Failed(format!(
                    "cannot read a file the restore overwrites: {e}"
                )))
            }
        };
        state.push(json!([
            change.relative_path.to_string_lossy(),
            format!("{:?}", change.status),
            digest
        ]));
    }
    let state = json!({"checkpoint": plan.checkpoint.commit_hash, "files": state});
    Ok((files, reverted_to, undo, state))
}

fn restore_checkpoint_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let plan = restore_plan(ctx, args).await?;
        let (files, _, _, state) = restore_targets(ctx, &plan).await?;
        let filter = PathFilter::new(ctx).await?;
        let mut items = files
            .iter()
            .take(RESTORE_LISTED)
            .map(|change| {
                let path = change.absolute_path.to_string_lossy();
                let label = if filter.allows(&path) {
                    change.relative_path.to_string_lossy().into_owned()
                } else {
                    "<a file this chat's model may not see>".to_string()
                };
                let new = match change.status {
                    crate::git::FileChangeStatus::ADDED => "added",
                    crate::git::FileChangeStatus::DELETED => "deleted",
                    crate::git::FileChangeStatus::MODIFIED => "modified back",
                };
                item("file", Some(json!(label)), Some(json!(new)))
            })
            .collect::<Vec<_>>();
        let scope = Some(format!(
            "workspace {}",
            plan.checkpoint.workspace_folder.display()
        ));
        if items.is_empty() {
            return Ok(Preview {
                title: "restore a checkpoint".into(),
                items,
                scope,
                class: EngineClass::Destructive,
                warning: Some(RESTORE_WARNING.into()),
                plan: super::Plan::nothing(),
            });
        }
        let total = files.len();
        items.push(item(
            "files overwritten",
            None,
            Some(json!(if total > RESTORE_LISTED {
                format!("{total} in total, the first {RESTORE_LISTED} listed above")
            } else {
                format!("{total} in total")
            })),
        ));
        Ok(Preview {
            title: "restore a checkpoint".into(),
            items,
            scope,
            class: EngineClass::Destructive,
            warning: Some(format!("{RESTORE_WARNING} {total} files are overwritten.")),
            plan: previewed::plan(state),
        })
    })
}

fn restore_checkpoint<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    use crate::git::checkpoints::{restore_workspace_checkpoint, restore_workspace_checkpoint_for_root};
    Box::pin(async move {
        let plan = restore_plan(ctx, args).await?;
        let (files, reverted_to, undo, state) = restore_targets(ctx, &plan).await?;
        previewed::take(ctx, "agents.restore_checkpoint", &state)?;
        let gcx = ctx.app.gcx.clone();
        match &plan.worktree_root {
            Some(root) => {
                restore_workspace_checkpoint_for_root(gcx, root, &plan.checkpoint, &plan.chat_id)
                    .await
            }
            None => restore_workspace_checkpoint(gcx, &plan.checkpoint, &plan.chat_id).await,
        }
        .map_err(EngineError::Failed)?;
        let filter = PathFilter::new(ctx).await?;
        let changed: Vec<Value> = files
            .iter()
            .filter(|c| filter.allows(&c.absolute_path.to_string_lossy()))
            .map(|c| json!({"path": c.relative_path.to_string_lossy(), "status": format!("{:?}", c.status).to_lowercase()}))
            .collect();
        let text = format!(
            "restored {} files to {}",
            files.len(),
            reverted_to.to_rfc3339()
        );
        Ok(EngineOutput::new(
            json!({
                "restored": true,
                "files_changed": changed,
                "reverted_to": reverted_to.to_rfc3339(),
                "checkpoint_for_undo": {
                    "workspace_folder": undo.workspace_folder.to_string_lossy(),
                    "commit_hash": undo.commit_hash,
                },
            }),
            text,
        ))
    })
}

async fn provenance_key(ctx: &EngineCtx, args: &Value) -> Result<Option<String>, EngineError> {
    let Some(path) = args.get("path").and_then(Value::as_str) else {
        return Ok(None);
    };
    let root = ctx
        .project_root
        .clone()
        .ok_or_else(|| EngineError::Type("no project is open".into()))?;
    let caller = crate::coordination::scope::WorkCaller {
        source_root: root.clone(),
        checkout_root: root,
        root_chat_id: None,
        principal: ctx.chat_id.clone(),
        model: None,
    };
    let resolved = crate::coordination::scope::resolve_path(path, &caller)
        .map_err(|e| EngineError::Type(format!("path: {}", e.message)))?;
    Ok(Some(crate::file_activity::resolve_key(&resolved).0))
}

async fn provenance_count(key: Option<&str>) -> Result<usize, EngineError> {
    crate::coordination::file_provenance::count(key)
        .await
        .map_err(failed)?
        .ok_or_else(|| {
            EngineError::Failed(
                "no provenance store is configured (coordination is off); nothing is recorded"
                    .into(),
            )
        })
}

fn forget_provenance_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        require_root_chat(ctx, "agents.forget_provenance").await?;
        let key = provenance_key(ctx, args).await?;
        let count = provenance_count(key.as_deref()).await?;
        let label = args
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("all files")
            .to_string();
        let root = ctx
            .project_root
            .as_ref()
            .map(|r| r.to_string_lossy().into_owned())
            .unwrap_or_default();
        Ok(Preview {
            title: "forget file provenance".into(),
            items: vec![named_item(
                &label,
                Some(json!(format!("{count} records"))),
                Some(json!("0")),
            )],
            scope: Some(format!("project {root}")),
            class: EngineClass::Destructive,
            warning: Some(FORGET_WARNING.into()),
            plan: if count == 0 {
                super::Plan::nothing()
            } else {
                previewed::plan(json!(count))
            },
        })
    })
}

fn forget_provenance<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let key = provenance_key(ctx, args).await?;
        let count = provenance_count(key.as_deref()).await?;
        previewed::take(ctx, "agents.forget_provenance", &json!(count))?;
        let removed = crate::coordination::file_provenance::forget(key.as_deref())
            .await
            .map_err(failed)?;
        Ok(EngineOutput::new(
            json!({"removed": removed}),
            format!("forgot {removed} provenance records"),
        ))
    })
}
