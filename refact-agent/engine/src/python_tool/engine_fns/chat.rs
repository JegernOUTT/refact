use futures::future::BoxFuture;
use serde_json::{json, Map, Value};

use super::common::to_value;
use super::{EngineClass, EngineCtx, EngineError, EngineFn, EngineOutput, Plan, Preview};
use crate::call_validation::ChatMessage;
use crate::integrations::mcp::redaction::scrub_text;

pub(super) const ROLES: &[&str] = &[
    "user",
    "assistant",
    "tool",
    "system",
    "context_file",
    "plan",
    "goal",
    "event",
    "compression_report",
];

pub fn functions() -> Vec<EngineFn> {
    let none = || json!({"type":"object","additionalProperties":false,"properties":{}});
    vec![
        EngineFn {
            name: "chat.get",
            summary: "This chat's parameters, state, goal counters and what is pending (queue, pauses, Python approvals).",
            class: EngineClass::Read,
            input: none,
            output: || Some(json!({"type":"object","required":["chat_id","model","mode","state"],"properties":{"chat_id":{"type":"string"},"title":{"type":"string"},"model":{"type":"string"},"mode":{"type":"string"},"tool_use":{"type":"string"},"reasoning":{"type":"object"},"sampling":{"type":"object"},"context":{"type":"object"},"flags":{"type":"object"},"approvals":{"type":"object"},"recovery_policy":{"type":"object"},"recovery_policy_source":{"enum":["global","project","thread"]},"active_skill":{"type":"string"},"worktree":{"type":"object"},"parent_id":{"type":"string"},"root_chat_id":{"type":"string"},"link_type":{"type":"string"},"state":{"type":"string"},"paused":{"type":"boolean"},"pause_reasons":{"type":"array","items":{"type":"object","properties":{"type":{"type":"string"},"tool_name":{"type":"string"},"rule":{"type":"string"}}}},"error":{"type":"string"},"queue_size":{"type":"integer"},"goal":{"type":"object"},"python":{"type":"object","properties":{"runs":{"type":"integer"},"pending_approvals":{"type":"integer"}}}}})),
            handler: get,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "chat.messages",
            summary: "This chat's messages by index range with roles, sizes and token estimates; their privacy records travel with the content into this run.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{
                "start":{"type":"integer","minimum":0,"description":"First index; the last `limit` messages by default."},
                "limit":{"type":"integer","minimum":1,"maximum":200,"description":"Messages to return; 50 by default."},
                "roles":{"type":"array","minItems":1,"uniqueItems":true,"items":{"enum":ROLES},"description":"Keep only these roles."},
                "content":{"type":"boolean","description":"Include message text; true by default, false gives metadata only."},
                "max_chars":{"type":"integer","minimum":100,"maximum":20000,"description":"Characters of text per message; 4000 by default."}
            }}),
            output: || Some(messages_schema(false)),
            handler: messages,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "chat.queue",
            summary: "This chat's queued commands and pending deliveries; delivery previews carry their privacy records.",
            class: EngineClass::Read,
            input: none,
            output: || Some(json!({"type":"object","required":["items","size"],"properties":{"items":{"type":"array","items":{"type":"object","properties":{"id":{"type":"string"},"kind":{"enum":["command","delivery"]},"command_type":{"type":"string"},"priority":{"type":"boolean"},"preview":{"type":"string"},"push":{"type":"string"},"source":{"type":"string"},"enqueued_at_ms":{"type":"integer"}}}},"size":{"type":"integer"}}})),
            handler: queue,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "chat.usage",
            summary: "Token usage and metered cost still recorded on this chat's messages, plus goal counters; complete is false after compression or a model switch dropped usage.",
            class: EngineClass::Read,
            input: none,
            output: || Some(json!({"type":"object","required":["messages_with_usage","prompt_tokens","completion_tokens","cache_read_tokens","cache_creation_tokens","complete"],"properties":{"messages_with_usage":{"type":"integer"},"prompt_tokens":{"type":"integer"},"completion_tokens":{"type":"integer"},"cache_read_tokens":{"type":"integer"},"cache_creation_tokens":{"type":"integer"},"total_usd":{"type":"number"},"goal":{"type":"object","properties":{"turns_used":{"type":"integer"},"tokens_used":{"type":"integer"}}},"complete":{"type":"boolean"}}})),
            handler: usage,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "chat.tools",
            summary: "The tools this chat can use now with source and the mode's confirmation rule for each; for a per-call verdict use tools.check.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{"names_only":{"type":"boolean","description":"Return only tool names; false by default."}}}),
            output: || Some(json!({"type":"object","required":["mode","tools","catalog_ready"],"properties":{"mode":{"type":"string"},"tools":{"type":"array","items":{"type":["object","string"],"properties":{"name":{"type":"string"},"source":{"type":"string"},"allow_parallel":{"type":"boolean"},"mode_rule":{"enum":["ask","deny","auto",null]},"auto_approved_by_flag":{"type":"boolean"},"editing_tool":{"type":"boolean"}}}},"mcp_lazy":{"type":"boolean"},"mcp_total":{"type":"integer"},"catalog_ready":{"type":"boolean"}}})),
            handler: tools,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "chat.set_model",
            summary: "Switch this chat's model, validated against the loaded models. Queued: takes effect after this turn. Risky when the provider changes.",
            class: EngineClass::Change,
            input: || object_schema(&["model"], &["model"]),
            output: change_output,
            handler: set_model,
            preview: Some(set_model_preview),
            timeout: None,
        },
        EngineFn {
            name: "chat.set_mode",
            summary: "Switch this chat's mode in place: the new mode's tools, confirmation rules and thread defaults apply from the next turn; the chat keeps its current system prompt (a handoff starts a chat with the new prompt). Queued: takes effect after this turn. Risky when the new mode runs tools without asking.",
            class: EngineClass::Change,
            input: || object_schema(&["mode"], &["mode"]),
            output: change_output,
            handler: set_mode,
            preview: Some(set_mode_preview),
            timeout: None,
        },
        EngineFn {
            name: "chat.set_reasoning",
            summary: "Set this chat's boost, reasoning effort or thinking budget, checked against the model. Queued: takes effect after this turn.",
            class: EngineClass::Change,
            input: || object_schema(&["reasoning"], &[]),
            output: change_output,
            handler: set_reasoning,
            preview: Some(set_reasoning_preview),
            timeout: None,
        },
        EngineFn {
            name: "chat.set_sampling",
            summary: "Set this chat's temperature, frequency penalty or max output tokens; out-of-range values are refused. Queued: takes effect after this turn.",
            class: EngineClass::Change,
            input: || object_schema(&["sampling"], &[]),
            output: change_output,
            handler: set_sampling,
            preview: Some(set_sampling_preview),
            timeout: None,
        },
        EngineFn {
            name: "chat.set_context",
            summary: "Set this chat's context window cap and automatic compression threshold. Queued: takes effect after this turn.",
            class: EngineClass::Change,
            input: || object_schema(&["context"], &[]),
            output: change_output,
            handler: set_context,
            preview: Some(set_context_preview),
            timeout: None,
        },
        EngineFn {
            name: "chat.set_flags",
            summary: "Toggle this chat's parallel tool calls, project info, checkpoints, auto-enrichment or auto-compaction. Queued: takes effect after this turn. Risky when checkpoints are turned off.",
            class: EngineClass::Change,
            input: || object_schema(&["flags"], &[]),
            output: change_output,
            handler: set_flags,
            preview: Some(set_flags_preview),
            timeout: None,
        },
        EngineFn {
            name: "chat.set_recovery",
            summary: "Set this chat's automatic retry policy for failed model requests, or reset it. Queued: takes effect after this turn. Risky when retries increase.",
            class: EngineClass::Change,
            input: || {
                let mut schema = object_schema(&["recovery"], &[]);
                let props = schema["properties"].as_object_mut().unwrap();
                let mode = props.remove("recovery_mode").unwrap_or_default();
                props.insert("mode".into(), mode);
                schema
            },
            output: change_output,
            handler: set_recovery,
            preview: Some(set_recovery_preview),
            timeout: None,
        },
        EngineFn {
            name: "chat.set_goal_budget",
            summary: "Change the soft budget of this chat's goal (cooldown, no-progress threshold); hard limits cannot be set. Queued: takes effect after this turn.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"properties":{"cooldown_ms":{"type":"integer","minimum":0,"maximum":3600000},"no_progress_token_threshold":{"type":"integer","minimum":0,"maximum":1000000}}}),
            output: change_output,
            handler: set_goal_budget,
            preview: Some(set_goal_budget_preview),
            timeout: None,
        },
        EngineFn {
            name: "chat.stop_goal",
            summary: "Stop pursuit of this chat's goal (it can be resumed later). Queued: takes effect after this turn.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"properties":{}}),
            output: change_output,
            handler: stop_goal,
            preview: Some(stop_goal_preview),
            timeout: None,
        },
        EngineFn {
            name: "chat.cancel_queued",
            summary: "Remove one queued command or pending delivery from this chat's queue (id from chat.queue). Destructive for a queued user message.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"required":["id"],"properties":{"id":{"type":"string","minLength":1}}}),
            output: || Some(json!({"type":"object","required":["removed","kind"],"properties":{"removed":{"type":"boolean"},"kind":{"enum":["command","delivery"]}}})),
            handler: cancel_queued,
            preview: Some(cancel_queued_preview),
            timeout: None,
        },
        EngineFn {
            name: "chat.set_auto_approve",
            summary: "Turn this chat's automatic approval of editing tools, dangerous commands or all confirmations on or off. Queued: takes effect after this turn. Risky when a flag is turned on. These flags never affect engine functions, which always ask you.",
            class: EngineClass::Change,
            input: || auto_approve_schema(false),
            output: change_output,
            handler: set_auto_approve,
            preview: Some(set_auto_approve_preview),
            timeout: None,
        },
    ]
}

pub(super) fn change_output() -> Option<Value> {
    Some(
        json!({"type":"object","required":["request_id","changed"],"properties":{"request_id":{"type":"string"},"changed":{"type":"object"},"applies":{"const":"after_this_turn"},"old_model":{"type":"string"},"new_model":{"type":"string"},"old_mode":{"type":"string"},"new_mode":{"type":"string"},"old_budget":{"type":"object"},"new_budget":{"type":"object"},"old_status":{},"new_status":{"type":"string"}}}),
    )
}

pub(super) fn messages_schema(other_chat: bool) -> Value {
    let mut schema = json!({"type":"object","required":["total","start","messages"],"properties":{"total":{"type":"integer"},"start":{"type":"integer"},"messages":{"type":"array","items":{"type":"object","properties":{"index":{"type":"integer"},"message_id":{"type":"string"},"role":{"type":"string"},"chars":{"type":"integer"},"approx_tokens":{"type":"integer"},"tool_calls":{"type":"array","items":{"type":"object","properties":{"id":{"type":"string"},"name":{"type":"string"}}}},"tool_call_id":{"type":"string"},"content":{"type":"string"},"truncated":{"type":"boolean"}}}}}});
    if other_chat {
        schema["required"] = json!(["chat_id", "total", "start", "loaded", "messages"]);
        schema["properties"]["chat_id"] = json!({"type":"string"});
        schema["properties"]["title"] = json!({"type":"string"});
        schema["properties"]["loaded"] = json!({"type":"boolean"});
    }
    schema
}

pub(super) async fn this_session(
    ctx: &EngineCtx,
) -> Result<std::sync::Arc<tokio::sync::Mutex<crate::chat::types::ChatSession>>, EngineError> {
    let session = ctx
        .app
        .chat
        .sessions
        .read()
        .await
        .get(&ctx.chat_id)
        .cloned();
    session.ok_or_else(|| EngineError::Failed("this chat has no live session".into()))
}

fn opt(map: &mut Map<String, Value>, key: &str, value: Option<Value>) {
    if let Some(value) = value.filter(|v| !v.is_null()) {
        map.insert(key.into(), value);
    }
}

fn get<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let session = this_session(ctx).await?;
        let s = session.lock().await;
        let t = &s.thread;
        let mut reasoning = Map::new();
        opt(
            &mut reasoning,
            "boost_reasoning",
            t.boost_reasoning.map(Value::from),
        );
        opt(
            &mut reasoning,
            "reasoning_effort",
            t.reasoning_effort.clone().map(Value::from),
        );
        opt(
            &mut reasoning,
            "thinking_budget",
            t.thinking_budget.map(Value::from),
        );
        let mut sampling = Map::new();
        opt(
            &mut sampling,
            "temperature",
            t.temperature.map(|v| json!(v)),
        );
        opt(
            &mut sampling,
            "frequency_penalty",
            t.frequency_penalty.map(|v| json!(v)),
        );
        opt(&mut sampling, "max_tokens", t.max_tokens.map(Value::from));
        let mut context = Map::new();
        opt(
            &mut context,
            "context_tokens_cap",
            t.context_tokens_cap.map(Value::from),
        );
        opt(
            &mut context,
            "auto_compression_cap",
            t.auto_compression_cap.map(Value::from),
        );
        context.insert(
            "auto_compression_cap_pending".into(),
            json!(t.auto_compression_cap_pending),
        );
        let mut flags = Map::new();
        opt(
            &mut flags,
            "parallel_tool_calls",
            t.parallel_tool_calls.map(Value::from),
        );
        flags.insert("include_project_info".into(), json!(t.include_project_info));
        flags.insert("checkpoints_enabled".into(), json!(t.checkpoints_enabled));
        opt(
            &mut flags,
            "auto_enrichment_enabled",
            t.auto_enrichment_enabled.map(Value::from),
        );
        opt(
            &mut flags,
            "auto_compact_enabled",
            t.auto_compact_enabled.map(Value::from),
        );
        let mut data = Map::new();
        data.insert("chat_id".into(), json!(s.chat_id));
        data.insert("title".into(), json!(t.title));
        data.insert("model".into(), json!(t.model));
        data.insert("mode".into(), json!(t.mode));
        data.insert("tool_use".into(), json!(t.tool_use));
        data.insert("reasoning".into(), Value::Object(reasoning));
        data.insert("sampling".into(), Value::Object(sampling));
        data.insert("context".into(), Value::Object(context));
        data.insert("flags".into(), Value::Object(flags));
        data.insert("approvals".into(), json!({"auto_approve_editing_tools":t.auto_approve_editing_tools,"auto_approve_dangerous_commands":t.auto_approve_dangerous_commands,"autonomous_no_confirm":t.autonomous_no_confirm}));
        opt(
            &mut data,
            "recovery_policy",
            t.recovery_policy.as_ref().map(to_value).transpose()?,
        );
        data.insert(
            "recovery_policy_source".into(),
            to_value(s.recovery_policy_source)?,
        );
        opt(
            &mut data,
            "active_skill",
            t.active_skill.clone().map(Value::from),
        );
        opt(
            &mut data,
            "worktree",
            t.worktree
                .as_ref()
                .map(|w| json!({"kind":w.kind,"root":w.root,"branch":w.branch})),
        );
        opt(&mut data, "parent_id", t.parent_id.clone().map(Value::from));
        opt(
            &mut data,
            "root_chat_id",
            t.root_chat_id.clone().map(Value::from),
        );
        opt(&mut data, "link_type", t.link_type.clone().map(Value::from));
        let r = &s.runtime;
        data.insert("state".into(), json!(r.state.to_string()));
        data.insert("paused".into(), json!(r.paused));
        data.insert(
            "pause_reasons".into(),
            json!(r
                .pause_reasons
                .iter()
                .map(|p| json!({"type":p.reason_type,"tool_name":p.tool_name,"rule":p.rule}))
                .collect::<Vec<_>>()),
        );
        opt(
            &mut data,
            "error",
            r.error.as_deref().map(|e| json!(scrub_text(e, &[]))),
        );
        let queue_size =
            s.command_queue.len() + s.pending_deliveries.len() + s.runner_pending_deliveries.len();
        data.insert("queue_size".into(), json!(queue_size));
        opt(&mut data, "goal", s.goal.as_ref().map(|g| json!({"status":g.status,"active":s.goal_active,"turns_used":s.goal_turns_used,"tokens_used":s.goal_tokens_used,"no_progress_turns":s.goal_no_progress_turns})));
        data.insert(
            "python".into(),
            json!({"runs":r.python_runs.len(),"pending_approvals":r.python_approvals.len()}),
        );
        let text = format!(
            "{}, mode {}, {}, {} queued",
            t.model, t.mode, r.state, queue_size
        );
        Ok(EngineOutput::new(Value::Object(data), text))
    })
}

pub(super) struct Range {
    pub start: usize,
    pub end: usize,
}

/// The index range of a message read: `start` or the last `limit`, refusing a start past the end.
pub(super) fn range(args: &Value, total: usize) -> Result<Range, EngineError> {
    let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize;
    let start = match args.get("start").and_then(Value::as_u64) {
        Some(start) if start as usize >= total && total > 0 => {
            return Err(EngineError::Type(format!(
                "start {start} is past the last message; the chat has {total} messages (0..{})",
                total - 1
            )))
        }
        Some(start) if total == 0 && start > 0 => {
            return Err(EngineError::Type(format!(
                "start {start} is past the end; the chat has no messages"
            )))
        }
        Some(start) => start as usize,
        None => total.saturating_sub(limit),
    };
    Ok(Range {
        start,
        end: (start + limit).min(total),
    })
}

/// Rows of a message read and the messages behind them (for privacy records).
pub(super) fn message_rows(
    messages: &[ChatMessage],
    first_index: usize,
    args: &Value,
    with_tokens: bool,
) -> (Vec<Value>, Vec<ChatMessage>) {
    let roles: Option<Vec<&str>> = args
        .get("roles")
        .and_then(Value::as_array)
        .map(|r| r.iter().filter_map(Value::as_str).collect());
    let content = args.get("content").and_then(Value::as_bool) != Some(false);
    let max_chars = args
        .get("max_chars")
        .and_then(Value::as_u64)
        .unwrap_or(4000) as usize;
    let mut rows = Vec::new();
    let mut selected = Vec::new();
    for (offset, message) in messages.iter().enumerate() {
        if roles
            .as_ref()
            .is_some_and(|r| !r.contains(&message.role.as_str()))
        {
            continue;
        }
        let text = message.content.content_text_only();
        let chars = text.chars().count();
        let mut row = json!({"index":first_index + offset,"message_id":message.message_id,"role":message.role,"chars":chars,"truncated":false});
        if with_tokens {
            row["approx_tokens"] = json!(crate::chat::trajectory_ops::approx_token_count(
                std::slice::from_ref(message)
            ));
        }
        if let Some(calls) = message.tool_calls.as_ref().filter(|c| !c.is_empty()) {
            row["tool_calls"] = json!(calls
                .iter()
                .map(|c| json!({"id":c.id,"name":c.function.name}))
                .collect::<Vec<_>>());
        }
        if !message.tool_call_id.is_empty() {
            row["tool_call_id"] = json!(message.tool_call_id);
        }
        if content {
            row["content"] = json!(text.chars().take(max_chars).collect::<String>());
            row["truncated"] = json!(chars > max_chars);
        }
        rows.push(row);
        selected.push(message.clone());
    }
    (rows, selected)
}

/// Privacy records the returned messages carry, in the shape the run taint expects.
pub(super) fn carried_records(
    messages: &[ChatMessage],
    whose: &str,
) -> Result<Vec<Value>, EngineError> {
    let records = crate::privacy::records::records_to_carry(messages).map_err(|e| {
        EngineError::Failed(format!(
            "the privacy records of {whose} messages could not be read ({}); nothing was returned",
            e.message
        ))
    })?;
    let mut out = Vec::new();
    if !records.is_empty() {
        out.push(json!({"files":to_value(records)?}));
    }
    for message in messages {
        for key in ["privacy_shell", "privacy_observation"] {
            if let Some(observation) = message.extra.get(key) {
                out.push(json!({key: observation}));
            }
        }
    }
    Ok(out)
}

fn messages<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let session = this_session(ctx).await?;
        let (total, range, slice) = {
            let s = session.lock().await;
            let total = s.messages.len();
            let range = range(args, total)?;
            (
                total,
                range.start,
                s.messages[range.start..range.end].to_vec(),
            )
        };
        let (rows, selected) = message_rows(&slice, range, args, true);
        let mut output = EngineOutput::new(Value::Null, "");
        if args.get("content").and_then(Value::as_bool) != Some(false) {
            output.privacy = carried_records(&selected, "this chat's")?;
        }
        let end = range + slice.len();
        output.text = format!("messages {range}..{end} of {total}");
        output.data = json!({"total":total,"start":range,"messages":rows});
        Ok(output)
    })
}

fn queue<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let session = this_session(ctx).await?;
        let (items, deliveries) = {
            let s = session.lock().await;
            let deliveries: Vec<ChatMessage> = s
                .pending_deliveries
                .iter()
                .chain(s.runner_pending_deliveries.iter())
                .flat_map(|d| d.messages.iter().cloned())
                .collect();
            (s.build_queued_items(), deliveries)
        };
        let delivery_count = items
            .iter()
            .filter(|i| i.command_type == refact_chat_api::DELIVERY_COMMAND_TYPE)
            .count();
        let rows: Vec<Value> = items
            .iter()
            .map(|item| {
                let delivery = item.command_type == refact_chat_api::DELIVERY_COMMAND_TYPE;
                let mut row = json!({"id":item.client_request_id,"kind":if delivery {"delivery"} else {"command"},"command_type":item.command_type,"priority":item.priority,"preview":scrub_text(&item.preview.chars().take(200).collect::<String>(), &[])});
                if let Some(push) = item.push {
                    row["push"] = json!(push);
                }
                if let Some(source) = &item.source {
                    row["source"] = json!(source);
                }
                if let Some(at) = item.enqueued_at_ms {
                    row["enqueued_at_ms"] = json!(at);
                }
                row
            })
            .collect();
        let mut output = EngineOutput::new(
            json!({"items":rows,"size":items.len()}),
            format!("{} queued ({delivery_count} deliveries)", items.len()),
        );
        output.privacy = carried_records(&deliveries, "the queued deliveries'")?;
        Ok(output)
    })
}

fn usage<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let session = this_session(ctx).await?;
        let s = session.lock().await;
        let (mut count, mut prompt, mut completion, mut cache_read, mut cache_creation) =
            (0usize, 0usize, 0usize, 0usize, 0usize);
        let mut usd: Option<f64> = None;
        for usage in s
            .messages
            .iter()
            .filter_map(|m| m.usage.as_ref())
            .chain(s.draft_usage.as_ref())
        {
            count += 1;
            prompt += usage.prompt_tokens;
            completion += usage.completion_tokens;
            cache_read += usage.cache_read_tokens.unwrap_or(0);
            cache_creation += usage.cache_creation_tokens.unwrap_or(0);
            if let Some(metered) = &usage.metering_usd {
                *usd.get_or_insert(0.0) += metered.total_usd;
            }
        }
        let complete = !s.messages.iter().any(|m| m.role == "compression_report");
        let mut data = json!({"messages_with_usage":count,"prompt_tokens":prompt,"completion_tokens":completion,"cache_read_tokens":cache_read,"cache_creation_tokens":cache_creation,"complete":complete});
        if let Some(usd) = usd {
            data["total_usd"] = json!(usd);
        }
        if s.goal.is_some() {
            data["goal"] = json!({"turns_used":s.goal_turns_used,"tokens_used":s.goal_tokens_used});
        }
        let cost = usd.map(|u| format!(", ${u:.4}")).unwrap_or_default();
        Ok(EngineOutput::new(
            data,
            format!("{prompt} prompt + {completion} completion tokens{cost}"),
        ))
    })
}

fn tools<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let session = this_session(ctx).await?;
        let (thread, catalog) = {
            let s = session.lock().await;
            (s.thread.clone(), s.tool_catalog.clone())
        };
        let Some(catalog) = catalog else {
            return Ok(EngineOutput::new(
                json!({"mode":thread.mode,"tools":[],"catalog_ready":false}),
                "this chat has no tool catalog yet",
            ));
        };
        let names: Vec<String> = catalog.index.tools.iter().map(|t| t.name.clone()).collect();
        if args.get("names_only").and_then(Value::as_bool) == Some(true) {
            return Ok(EngineOutput::new(
                json!({"mode":thread.mode,"tools":names,"mcp_lazy":catalog.index.mcp_lazy_mode,"mcp_total":catalog.index.mcp_total_count,"catalog_ready":true}),
                format!("{} tools in mode {}", names.len(), thread.mode),
            ));
        }
        let mode = crate::yaml_configs::customization_registry::get_mode_config(
            ctx.app.gcx.clone(),
            &thread.mode,
            Some(&thread.model),
        )
        .await;
        let rules = mode
            .as_ref()
            .map(|m| m.tool_confirm.rules.clone())
            .unwrap_or_default();
        let mut asks = 0;
        let rows: Vec<Value> = catalog
            .index
            .tools
            .iter()
            .map(|tool| {
                let rule = crate::yaml_configs::customization_registry::match_tool_confirm_action(&rules, &tool.name);
                asks += usize::from(rule.as_deref() == Some("ask"));
                json!({"name":tool.name,"source":tool.source.source_type,"allow_parallel":tool.allow_parallel,"mode_rule":rule,"auto_approved_by_flag":crate::chat::tools::should_auto_approve_confirmation(&thread, &tool.name),"editing_tool":crate::chat::tools::is_editing_tool(&tool.name)})
            })
            .collect();
        Ok(EngineOutput::new(
            json!({"mode":thread.mode,"tools":rows,"mcp_lazy":catalog.index.mcp_lazy_mode,"mcp_total":catalog.index.mcp_total_count,"catalog_ready":true}),
            format!(
                "{} tools in mode {} ({asks} with ask rules)",
                rows.len(),
                thread.mode
            ),
        ))
    })
}

// ---------------------------------------------------------------------------
// Changes of a chat's parameters. The plan is a SetParams patch computed from the thread the
// preview showed; this chat gets it queued (lands after the turn), another idle chat runs it now.

pub(super) const APPLIES_LATER: &str = "after_this_turn";
pub(super) const THIS_CHAT_SCOPE: &str = "this chat, from its next turn";

pub(super) const REASONING_ARGS: &[&str] =
    &["boost_reasoning", "reasoning_effort", "thinking_budget"];
pub(super) const SAMPLING_ARGS: &[&str] = &["temperature", "frequency_penalty", "max_tokens"];
pub(super) const CONTEXT_ARGS: &[&str] = &["context_tokens_cap", "auto_compression_cap"];
pub(super) const FLAG_ARGS: &[&str] = &[
    "parallel_tool_calls",
    "include_project_info",
    "checkpoints_enabled",
    "auto_enrichment_enabled",
    "auto_compact_enabled",
];

/// Input schema properties of each parameter group, shared with `chats.set_params`.
pub(super) fn param_properties(group: &str) -> Map<String, Value> {
    let value = match group {
        "model" => {
            json!({"model":{"type":"string","minLength":1,"description":"A loaded chat model id, e.g. openai/gpt-4.1."}})
        }
        "mode" => {
            json!({"mode":{"type":"string","minLength":1,"description":"Mode id from modes.list."},"reason":{"type":"string","maxLength":200,"description":"Recorded in the mode-switch event."}})
        }
        "reasoning" => json!({
            "boost_reasoning":{"type":["boolean","null"]},
            "reasoning_effort":{"type":["string","null"],"description":"One of the model's reasoning_effort_options; null clears."},
            "thinking_budget":{"type":["integer","null"],"minimum":1024,"description":"Up to the model's max_thinking_tokens; null clears."}
        }),
        "sampling" => json!({
            "temperature":{"type":["number","null"],"minimum":0,"maximum":2},
            "frequency_penalty":{"type":["number","null"],"minimum":-2,"maximum":2},
            "max_tokens":{"type":["integer","null"],"minimum":1,"maximum":1000000}
        }),
        "context" => json!({
            "context_tokens_cap":{"type":["integer","null"],"minimum":4096,"description":"null: the model's window."},
            "auto_compression_cap":{"type":["integer","null"],"minimum":1024,"description":"null: recompute the default."}
        }),
        "flags" => json!({
            "parallel_tool_calls":{"type":["boolean","null"]},
            "include_project_info":{"type":"boolean"},
            "checkpoints_enabled":{"type":"boolean"},
            "auto_enrichment_enabled":{"type":["boolean","null"]},
            "auto_compact_enabled":{"type":["boolean","null"]}
        }),
        "recovery" => json!({
            "recovery_mode":{"enum":["off","transient"]},
            "max_retries":{"type":"integer","minimum":0,"maximum":100},
            "delay_ms":{"type":"integer","minimum":0,"maximum":3600000},
            "total_timeout_ms":{"type":"integer","minimum":1000,"maximum":86400000},
            "reset":{"type":"boolean","description":"Remove this chat's override; other recovery fields must be absent."}
        }),
        _ => json!({}),
    };
    value.as_object().cloned().unwrap_or_default()
}

pub(super) fn object_schema(groups: &[&str], required: &[&str]) -> Value {
    let mut properties = Map::new();
    for group in groups {
        properties.extend(param_properties(group));
    }
    json!({"type":"object","additionalProperties":false,"required":required,"properties":properties})
}

/// Thread values as the preview shows them.
fn thread_value(t: &crate::chat::types::ThreadParams, key: &str) -> Value {
    match key {
        "model" => json!(t.model),
        "mode" => json!(t.mode),
        "boost_reasoning" => json!(t.boost_reasoning),
        "reasoning_effort" => json!(t.reasoning_effort),
        "thinking_budget" => json!(t.thinking_budget),
        "temperature" => json!(t.temperature),
        "frequency_penalty" => json!(t.frequency_penalty),
        "max_tokens" => json!(t.max_tokens),
        "context_tokens_cap" => json!(t.context_tokens_cap),
        "auto_compression_cap" => json!(t.auto_compression_cap),
        "parallel_tool_calls" => json!(t.parallel_tool_calls),
        "include_project_info" => json!(t.include_project_info),
        "checkpoints_enabled" => json!(t.checkpoints_enabled),
        "auto_enrichment_enabled" => json!(t.auto_enrichment_enabled),
        "auto_compact_enabled" => json!(t.auto_compact_enabled),
        "auto_approve_editing_tools" => json!(t.auto_approve_editing_tools),
        "auto_approve_dangerous_commands" => json!(t.auto_approve_dangerous_commands),
        "autonomous_no_confirm" => json!(t.autonomous_no_confirm),
        "recovery_policy" => json!(t.recovery_policy),
        "title" => json!(t.title),
        _ => Value::Null,
    }
}

fn shown(value: &Value) -> Option<Value> {
    (!value.is_null()).then(|| value.clone())
}

/// What a parameter change will do, computed from one thread and model record.
#[derive(Clone, Default)]
pub(super) struct ParamsPlan {
    pub patch: Map<String, Value>,
    pub items: Vec<refact_chat_api::PythonEngineItem>,
    pub changed: Map<String, Value>,
    pub warnings: Vec<String>,
    pub mode_widens: bool,
}

impl ParamsPlan {
    fn set(&mut self, thread: &crate::chat::types::ThreadParams, key: &str, new: Value) {
        let old = thread_value(thread, key);
        self.patch.insert(key.into(), new.clone());
        if old != new {
            self.items
                .push(super::mask::item(key, shown(&old), shown(&new)));
            self.changed
                .insert(key.into(), json!({"old":old,"new":new}));
        }
    }
    fn warn(&mut self, warning: &str) {
        if !self.warnings.iter().any(|w| w == warning) {
            self.warnings.push(warning.to_string());
        }
    }
}

const PROVIDER_WARNING: &str =
    "This chat's history, including file contents, will be sent to a different provider.";
const SYSTEM_PROMPT_KEPT: &str =
    "kept: the chat keeps its current system prompt (a handoff starts a chat with the new mode's prompt)";
const MODE_WARNING: &str = "The new mode lets tools run without asking you.";
const CHECKPOINT_WARNING: &str =
    "File changes made from now on in this chat cannot be rolled back with checkpoints.";
const RETRY_WARNING: &str = "Failed requests will be retried more, which can increase cost.";

/// Old mode names `map_legacy_mode_to_id` translates; any other name must already be an id.
const LEGACY_MODES: &[&str] = &[
    "NO_TOOLS",
    "EXPLORE",
    "AGENT",
    "CONFIGURE",
    "TASK_PLANNER",
    "TASK_AGENT",
    "BRAINSTORM",
    "BRAINSTORMING",
];

/// Modes entered only through handoff or task flows, never switched in place.
const BOUND_MODES: &[&str] = &["task_agent", "task_planner", "buddy"];

async fn caps_of(
    ctx: &EngineCtx,
) -> Result<std::sync::Arc<crate::caps::CodeAssistantCaps>, EngineError> {
    crate::global_context::try_load_caps_quickly_if_not_present(ctx.app.gcx.clone(), 0)
        .await
        .map_err(|e| {
            EngineError::Failed(format!("model capabilities are unavailable: {}", e.message))
        })
}

fn provider_of(model: &str) -> String {
    crate::privacy::records::provider_destination(model).id.0
}

/// Validates the given parameter arguments against `thread` and its model and plans the patch.
pub(super) async fn plan_params(
    ctx: &EngineCtx,
    thread: &crate::chat::types::ThreadParams,
    active_model_override: bool,
    messages_tokens: usize,
    args: &Value,
) -> Result<ParamsPlan, EngineError> {
    let mut plan = ParamsPlan::default();
    let caps = caps_of(ctx).await?;
    let mut model_id = thread.model.clone();
    if let Some(model) = args.get("model").and_then(Value::as_str) {
        if active_model_override {
            return Err(EngineError::Permission(
                "a recovery retry with a pinned model is running in this chat; change the model after it ends".into(),
            ));
        }
        let new =
            crate::caps::resolve_chat_model(caps.clone(), model).map_err(EngineError::Type)?;
        if model != thread.model {
            let old_ctx = crate::caps::resolve_chat_model(caps.clone(), &thread.model)
                .map(|m| m.base.n_ctx)
                .ok();
            plan.set(thread, "model", json!(model));
            plan.items.push(super::mask::item(
                "context window",
                old_ctx.map(|n| json!(n)),
                Some(json!(new.base.n_ctx)),
            ));
            plan.items.push(super::mask::item(
                "history",
                Some(json!("as is")),
                Some(json!("reasoning blocks and usage removed")),
            ));
            if !thread.model.is_empty() && provider_of(&thread.model) != provider_of(model) {
                plan.warn(PROVIDER_WARNING);
            }
        }
        model_id = model.to_string();
    }
    let record = crate::caps::resolve_chat_model(caps.clone(), &model_id).ok();
    if let Some(mode) = args.get("mode").and_then(Value::as_str) {
        plan_mode(
            ctx,
            thread,
            &model_id,
            mode,
            args.get("reason").and_then(Value::as_str),
            &mut plan,
        )
        .await?;
    }
    if let Some(value) = args.get("boost_reasoning") {
        plan.set(thread, "boost_reasoning", value.clone());
    }
    if let Some(value) = args.get("reasoning_effort") {
        if let Some(effort) = value.as_str() {
            let options = record
                .as_ref()
                .and_then(|m| m.reasoning_effort_options.clone())
                .unwrap_or_default();
            if !options.iter().any(|o| o == effort) {
                return Err(EngineError::Type(format!(
                    "reasoning_effort {effort:?} is not offered by {model_id}; options: {}",
                    if options.is_empty() {
                        "none".to_string()
                    } else {
                        options.join(", ")
                    }
                )));
            }
        }
        plan.set(thread, "reasoning_effort", value.clone());
    }
    if let Some(value) = args.get("thinking_budget") {
        if let Some(budget) = value.as_u64() {
            let record = record
                .as_ref()
                .filter(|m| m.supports_thinking_budget)
                .ok_or_else(|| {
                    EngineError::Type(format!("{model_id} does not support a thinking budget"))
                })?;
            if let Some(max) = record
                .max_thinking_tokens
                .filter(|max| budget as usize > *max)
            {
                return Err(EngineError::Type(format!(
                    "thinking_budget {budget} is above {model_id}'s maximum {max}"
                )));
            }
        }
        plan.set(thread, "thinking_budget", value.clone());
    }
    if let Some(value) = args.get("temperature") {
        if !value.is_null() && record.as_ref().is_some_and(|m| !m.supports_temperature) {
            return Err(EngineError::Type(format!(
                "{model_id} does not accept a temperature"
            )));
        }
        plan.set(thread, "temperature", value.clone());
    }
    if let Some(value) = args.get("frequency_penalty") {
        plan.set(thread, "frequency_penalty", value.clone());
    }
    if let Some(value) = args.get("max_tokens") {
        if let (Some(n), Some(limit)) = (
            value.as_u64(),
            record.as_ref().and_then(|m| m.max_output_tokens),
        ) {
            if n as usize > limit {
                return Err(EngineError::Type(format!(
                    "max_tokens {n} is above {model_id}'s output limit {limit}"
                )));
            }
        }
        plan.set(thread, "max_tokens", value.clone());
    }
    let n_ctx = record.as_ref().map(|m| m.base.n_ctx).filter(|n| *n > 0);
    let mut window = thread
        .context_tokens_cap
        .unwrap_or(usize::MAX)
        .min(n_ctx.unwrap_or(usize::MAX));
    if let Some(value) = args.get("context_tokens_cap") {
        if let Some(cap) = value.as_u64() {
            if n_ctx.is_some_and(|n| cap as usize > n) {
                return Err(EngineError::Type(format!(
                    "context_tokens_cap {cap} is above {model_id}'s window {}",
                    n_ctx.unwrap_or_default()
                )));
            }
        }
        window = value
            .as_u64()
            .map(|c| c as usize)
            .unwrap_or(n_ctx.unwrap_or(usize::MAX))
            .min(n_ctx.unwrap_or(usize::MAX));
        plan.set(thread, "context_tokens_cap", value.clone());
        if window != usize::MAX && messages_tokens > window {
            plan.items.push(super::mask::item(
                "next turn",
                Some(json!("-")),
                Some(json!("history will be compressed")),
            ));
        }
    }
    if let Some(value) = args.get("auto_compression_cap") {
        if let Some(cap) = value.as_u64() {
            if window != usize::MAX && cap as usize > window {
                return Err(EngineError::Type(format!(
                    "auto_compression_cap {cap} is above the effective window {window}"
                )));
            }
        }
        plan.set(thread, "auto_compression_cap", value.clone());
    }
    for key in FLAG_ARGS {
        if let Some(value) = args.get(*key) {
            if *key == "checkpoints_enabled" && thread.checkpoints_enabled && value == &json!(false)
            {
                plan.warn(CHECKPOINT_WARNING);
            }
            plan.set(thread, key, value.clone());
        }
    }
    plan_recovery(ctx, thread, args, &mut plan).await?;
    Ok(plan)
}

async fn plan_mode(
    ctx: &EngineCtx,
    thread: &crate::chat::types::ThreadParams,
    model: &str,
    requested: &str,
    reason: Option<&str>,
    plan: &mut ParamsPlan,
) -> Result<(), EngineError> {
    use crate::yaml_configs::customization_registry::{get_mode_config, map_legacy_mode_to_id};
    let id = map_legacy_mode_to_id(requested).to_string();
    let legacy = LEGACY_MODES.contains(&requested.to_ascii_uppercase().as_str());
    if id != requested && !legacy {
        return Err(EngineError::Type(format!(
            "unknown mode {requested:?}; see modes.list"
        )));
    }
    let gcx = ctx.app.gcx.clone();
    let to = get_mode_config(gcx.clone(), &id, Some(model))
        .await
        .ok_or_else(|| EngineError::Type(format!("unknown mode {requested:?}; see modes.list")))?;
    if to.specific && (BOUND_MODES.contains(&id.as_str()) || id.starts_with("setup_")) {
        return Err(EngineError::Permission(format!(
            "mode {id} is entered through its own flow (handoff or task), not switched in place"
        )));
    }
    if id == thread.mode {
        return Ok(());
    }
    let from = get_mode_config(gcx, &thread.mode, Some(model)).await;
    plan.set(thread, "mode", json!(id));
    if let Some(reason) = reason {
        plan.patch.insert("reason".into(), json!(reason));
    }
    plan.mode_widens = match &from {
        Some(from) => super::modes::widens(&to_value(from)?, &to_value(&to)?),
        None => true,
    };
    if let Some(from) = &from {
        let diff = crate::chat::queue::mode_switch_diff(from, &to);
        for (label, key) in [("tools added", "added"), ("tools removed", "removed")] {
            let delta = &diff["tools"][key];
            if delta["count"].as_u64().unwrap_or(0) > 0 {
                plan.items.push(super::mask::item(
                    label,
                    None,
                    Some(json!(format!(
                        "{} ({})",
                        delta["count"],
                        delta["names"]
                            .as_array()
                            .map(|n| n
                                .iter()
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join(", "))
                            .unwrap_or_default()
                    ))),
                ));
            }
        }
        if diff["tools"]["added"]["count"].as_u64().unwrap_or(0) > 0 {
            plan.warn(MODE_WARNING);
        }
        plan.items.push(super::mask::item(
            "system prompt",
            None,
            Some(json!(SYSTEM_PROMPT_KEPT)),
        ));
        plan.items.push(super::mask::item(
            "tool confirmation rules",
            Some(diff["tool_confirm"]["from_rules_count"].clone()),
            Some(diff["tool_confirm"]["to_rules_count"].clone()),
        ));
    }
    if to.tool_confirm.rules.iter().any(|r| r.action == "auto") {
        plan.warn(MODE_WARNING);
    }
    let d = &to.thread_defaults;
    for (key, value) in [
        ("include_project_info", d.include_project_info),
        ("checkpoints_enabled", d.checkpoints_enabled),
        ("auto_approve_editing_tools", d.auto_approve_editing_tools),
        (
            "auto_approve_dangerous_commands",
            d.auto_approve_dangerous_commands,
        ),
    ] {
        if let Some(value) = value {
            if value && thread_value(thread, key) == json!(false) && key.starts_with("auto_approve")
            {
                plan.warn(MODE_WARNING);
            }
            if key == "checkpoints_enabled" && !value && thread.checkpoints_enabled {
                plan.warn(CHECKPOINT_WARNING);
            }
            plan.set(thread, key, json!(value));
        }
    }
    Ok(())
}

async fn plan_recovery(
    ctx: &EngineCtx,
    thread: &crate::chat::types::ThreadParams,
    args: &Value,
    plan: &mut ParamsPlan,
) -> Result<(), EngineError> {
    use refact_core::recovery::{RecoveryMode, RecoveryPolicyOverrides};
    let fields = [
        "recovery_mode",
        "max_retries",
        "delay_ms",
        "total_timeout_ms",
    ];
    let given = fields.iter().any(|f| args.get(*f).is_some());
    let reset = args.get("reset").and_then(Value::as_bool) == Some(true);
    if !given && !reset {
        return Ok(());
    }
    if reset && given {
        return Err(EngineError::Type(
            "reset removes this chat's retry override; give no other retry fields with it".into(),
        ));
    }
    let (base, _) = crate::runtime_settings::effective_recovery_policy(ctx.project_root.as_deref())
        .await
        .map_err(EngineError::Failed)?;
    let old_effective = thread
        .recovery_policy
        .as_ref()
        .map(|o| o.resolve(base.clone()))
        .transpose()
        .map_err(EngineError::Failed)?
        .unwrap_or_else(|| base.clone());
    let new_override = if reset {
        None
    } else {
        let mut o = thread.recovery_policy.clone().unwrap_or_default();
        if let Some(mode) = args.get("recovery_mode").and_then(Value::as_str) {
            o.mode = Some(if mode == "off" {
                RecoveryMode::Off
            } else {
                RecoveryMode::Transient
            });
        }
        if let Some(n) = args.get("max_retries").and_then(Value::as_u64) {
            o.max_retries = Some(n as u32);
        }
        if let Some(n) = args.get("delay_ms").and_then(Value::as_u64) {
            o.delay_ms = Some(n);
        }
        if let Some(n) = args.get("total_timeout_ms").and_then(Value::as_u64) {
            o.total_timeout_ms = Some(n);
        }
        Some(o)
    };
    let patch = json!({"recovery_policy": new_override});
    crate::chat::recovery::validate_policy_patch(&patch).map_err(EngineError::Type)?;
    let new_effective = new_override
        .as_ref()
        .map(|o: &RecoveryPolicyOverrides| o.resolve(base.clone()))
        .transpose()
        .map_err(EngineError::Type)?
        .unwrap_or(base);
    let old = to_value(&old_effective)?;
    let new = to_value(&new_effective)?;
    for key in ["mode", "max_retries", "delay_ms", "total_timeout_ms"] {
        if old[key] != new[key] {
            plan.items.push(super::mask::item(
                &format!("retry {key}"),
                shown(&old[key]),
                shown(&new[key]),
            ));
        }
    }
    let retries = |p: &refact_core::recovery::RecoveryPolicy| {
        if p.mode == RecoveryMode::Off {
            0
        } else {
            p.max_retries
        }
    };
    if retries(&new_effective) > retries(&old_effective)
        || (retries(&new_effective) > 0
            && new_effective.total_timeout_ms > old_effective.total_timeout_ms)
    {
        plan.warn(RETRY_WARNING);
    }
    if thread.recovery_policy != new_override {
        plan.changed.insert(
            "recovery_policy".into(),
            json!({"old":thread.recovery_policy,"new":new_override}),
        );
        if old == new {
            plan.items.push(super::mask::item(
                "retry override",
                Some(json!("set")),
                Some(json!("removed")),
            ));
        }
    }
    plan.patch
        .insert("recovery_policy".into(), json!(new_override));
    Ok(())
}

/// The state a parameter plan of a chat is computed from.
pub(super) fn thread_state(thread: &crate::chat::types::ThreadParams) -> Value {
    let mut t = to_value(thread).unwrap_or_default();
    if let Some(map) = t.as_object_mut() {
        for key in [
            "frozen_request_prefix",
            "previous_response_id",
            "claude_code_identity",
            "reactive_compact_attempts",
            "is_title_generated",
            "auto_compression_cap_pending",
        ] {
            map.remove(key);
        }
    }
    t
}

/// Queues a command on a chat exactly as the command route does and starts or wakes its processor.
pub(super) async fn queue_command(
    ctx: &EngineCtx,
    session_arc: &std::sync::Arc<tokio::sync::Mutex<crate::chat::types::ChatSession>>,
    command: refact_chat_api::ChatCommand,
) -> Result<String, EngineError> {
    queue_built(ctx, session_arc, |_| Ok(command)).await
}

/// Like `queue_command`, but builds the command under the same session lock that enqueues it,
/// so a check of the chat's state and the enqueue cannot be separated by another change.
pub(super) async fn queue_built(
    ctx: &EngineCtx,
    session_arc: &std::sync::Arc<tokio::sync::Mutex<crate::chat::types::ChatSession>>,
    build: impl FnOnce(
        &mut crate::chat::types::ChatSession,
    ) -> Result<refact_chat_api::ChatCommand, EngineError>,
) -> Result<String, EngineError> {
    use refact_chat_api::EnqueueCommandOutcome;
    let request_id = format!("engine-{}", uuid::Uuid::new_v4());
    let mut session = session_arc.lock().await;
    if session.closed {
        return Err(EngineError::Failed(
            "the chat is closed; nothing was queued".into(),
        ));
    }
    let _guard = crate::chat::NativeMutationGuard::enter_locked(session_arc, &mut session)
        .map_err(EngineError::Permission)?;
    if crate::buddy::companion::is_companion(&session.thread) {
        return Err(EngineError::Permission(
            "companion conversations accept only user messages".into(),
        ));
    }
    let request = refact_chat_api::CommandRequest {
        native_approval_guard: None,
        supervision_guard: None,
        client_request_id: request_id.clone(),
        priority: false,
        command: build(&mut session)?,
    };
    let chat_id = session.chat_id.clone();
    match session.enqueue_accepted_command(request) {
        EnqueueCommandOutcome::Accepted | EnqueueCommandOutcome::Duplicate => {}
        EnqueueCommandOutcome::Full => {
            crate::chat::app_context::release(&chat_id, &request_id);
            return Err(EngineError::Failed(
                "the chat's command queue is full; nothing was queued".into(),
            ));
        }
        EnqueueCommandOutcome::Rejected => {
            crate::chat::app_context::release(&chat_id, &request_id);
            return Err(EngineError::Permission(
                "the chat refused the command; nothing was queued".into(),
            ));
        }
    }
    session.remember_accepted_request(&request_id);
    session.emit(refact_chat_api::ChatEvent::Ack {
        client_request_id: request_id.clone(),
        accepted: true,
        result: Some(json!({"queued": true})),
    });
    let notify = session.queue_notify.clone();
    let running = session.queue_processor_running.clone();
    drop(session);
    if !running.swap(true, std::sync::atomic::Ordering::SeqCst) {
        tokio::spawn(crate::chat::process_command_queue(
            ctx.app.clone(),
            session_arc.clone(),
            running,
        ));
    } else {
        notify.notify_one();
    }
    Ok(request_id)
}

/// Changes of this chat are refused in agent and child chats (the runner owns their session).
async fn require_own_chat(ctx: &EngineCtx) -> Result<(), EngineError> {
    let context = ctx.ccx.lock().await;
    if context.background_agent_id.is_some() || context.subchat_depth > 0 {
        return Err(EngineError::Permission(
            "this chat is run by an agent runner; its settings change only from the parent chat"
                .into(),
        ));
    }
    Ok(())
}

#[derive(Clone)]
struct ThisChatPlan {
    patch: Value,
}

async fn this_chat_snapshot(
    ctx: &EngineCtx,
) -> Result<(crate::chat::types::ThreadParams, bool, usize, Value), EngineError> {
    let session = this_session(ctx).await?;
    let s = session.lock().await;
    let tokens = crate::chat::trajectory_ops::approx_token_count(&s.messages);
    let state = json!({"thread":thread_state(&s.thread)});
    Ok((
        s.thread.clone(),
        s.active_command.model_override.is_some(),
        tokens,
        state,
    ))
}

fn params_preview(
    title: &str,
    scope: String,
    plan: ParamsPlan,
    state: Value,
    prefix: Option<&str>,
) -> Preview {
    if plan.changed.is_empty() {
        return Preview {
            title: title.into(),
            items: Vec::new(),
            scope: Some(scope),
            class: EngineClass::Change,
            warning: None,
            plan: Plan::nothing(),
        };
    }
    let warning = (!plan.warnings.is_empty()).then(|| {
        let text = plan.warnings.join(" ");
        match prefix {
            Some(prefix) => format!("{prefix}: {text}"),
            None => text,
        }
    });
    Preview {
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
            ThisChatPlan {
                patch: Value::Object(plan.patch),
            },
        ),
    }
}

fn this_chat_param_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
    title: &'static str,
    keys: &'static [&'static str],
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        require_own_chat(ctx).await?;
        if !keys.iter().any(|k| args.get(*k).is_some()) {
            return Err(EngineError::Type(format!(
                "no fields given; give at least one of {}",
                keys.join(", ")
            )));
        }
        let (thread, pinned, tokens, state) = this_chat_snapshot(ctx).await?;
        let plan = plan_params(ctx, &thread, pinned, tokens, args).await?;
        Ok(params_preview(
            title,
            THIS_CHAT_SCOPE.into(),
            plan,
            state,
            None,
        ))
    })
}

fn this_chat_param_apply<'a>(
    ctx: &'a EngineCtx,
    function: &'static str,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let (thread, _, _, state) = this_chat_snapshot(ctx).await?;
        let plan: ThisChatPlan = ctx.take_plan(function, &state)?;
        let session = this_session(ctx).await?;
        let request_id = queue_command(
            ctx,
            &session,
            refact_chat_api::ChatCommand::SetParams {
                patch: plan.patch.clone(),
            },
        )
        .await?;
        let mut changed = Map::new();
        if let Some(patch) = plan.patch.as_object() {
            for (key, new) in patch {
                if key == "reason" {
                    continue;
                }
                let old = thread_value(&thread, key);
                if &old != new {
                    changed.insert(key.clone(), json!({"old":old,"new":new}));
                }
            }
        }
        let summary = changed
            .iter()
            .map(|(k, v)| format!("{k} {} -> {}", v["old"], v["new"]))
            .collect::<Vec<_>>()
            .join(", ");
        let mut data = json!({"request_id":request_id,"changed":changed,"applies":APPLIES_LATER});
        if let Some(v) = changed.get("model") {
            data["old_model"] = v["old"].clone();
            data["new_model"] = v["new"].clone();
        }
        if let Some(v) = changed.get("mode") {
            data["old_mode"] = v["old"].clone();
            data["new_mode"] = v["new"].clone();
        }
        let mut text = format!(
            "{summary}; validated against this chat as it is now, takes effect after this turn"
        );
        if changed.contains_key("mode") {
            text.push_str(
                "; the new mode's tools and confirmation rules apply from the next turn, the system prompt stays as it is",
            );
        }
        Ok(EngineOutput::new(data, text))
    })
}

fn set_model_preview<'a>(
    c: &'a EngineCtx,
    a: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    this_chat_param_preview(c, a, "switch this chat's model", &["model"])
}
fn set_model<'a>(
    c: &'a EngineCtx,
    _a: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    this_chat_param_apply(c, "chat.set_model")
}
fn set_mode_preview<'a>(
    c: &'a EngineCtx,
    a: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    this_chat_param_preview(c, a, "switch this chat's mode", &["mode"])
}
fn set_mode<'a>(
    c: &'a EngineCtx,
    _a: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    this_chat_param_apply(c, "chat.set_mode")
}
fn set_reasoning_preview<'a>(
    c: &'a EngineCtx,
    a: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    this_chat_param_preview(
        c,
        a,
        "change this chat's reasoning settings",
        REASONING_ARGS,
    )
}
fn set_reasoning<'a>(
    c: &'a EngineCtx,
    _a: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    this_chat_param_apply(c, "chat.set_reasoning")
}
fn set_sampling_preview<'a>(
    c: &'a EngineCtx,
    a: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    this_chat_param_preview(c, a, "change this chat's sampling settings", SAMPLING_ARGS)
}
fn set_sampling<'a>(
    c: &'a EngineCtx,
    _a: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    this_chat_param_apply(c, "chat.set_sampling")
}
fn set_context_preview<'a>(
    c: &'a EngineCtx,
    a: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    this_chat_param_preview(c, a, "change this chat's context limits", CONTEXT_ARGS)
}
fn set_context<'a>(
    c: &'a EngineCtx,
    _a: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    this_chat_param_apply(c, "chat.set_context")
}
fn set_flags_preview<'a>(
    c: &'a EngineCtx,
    a: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    this_chat_param_preview(c, a, "change this chat's options", FLAG_ARGS)
}
fn set_flags<'a>(
    c: &'a EngineCtx,
    _a: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    this_chat_param_apply(c, "chat.set_flags")
}
fn set_recovery_preview<'a>(
    c: &'a EngineCtx,
    a: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let mut args = a.clone();
        if let Some(map) = args.as_object_mut() {
            if let Some(mode) = map.remove("mode") {
                map.insert("recovery_mode".into(), mode);
            }
        }
        this_chat_param_preview(
            c,
            &args,
            "change this chat's retry policy",
            &[
                "recovery_mode",
                "max_retries",
                "delay_ms",
                "total_timeout_ms",
                "reset",
            ],
        )
        .await
    })
}
fn set_recovery<'a>(
    c: &'a EngineCtx,
    _a: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    this_chat_param_apply(c, "chat.set_recovery")
}

#[derive(Clone)]
struct GoalBudgetPlan {
    budget: refact_chat_api::GoalBudget,
}

async fn goal_state(
    ctx: &EngineCtx,
) -> Result<
    (
        Option<refact_chat_api::GoalSnapshot>,
        Option<refact_chat_api::GoalStatus>,
        Value,
    ),
    EngineError,
> {
    let session = this_session(ctx).await?;
    let s = session.lock().await;
    let state = json!({"goal":s.goal,"status":s.goal_status});
    Ok((s.goal.clone(), s.goal_status, state))
}

fn set_goal_budget_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        require_own_chat(ctx).await?;
        if args.get("cooldown_ms").is_none() && args.get("no_progress_token_threshold").is_none() {
            return Err(EngineError::Type(
                "no fields given; give cooldown_ms or no_progress_token_threshold".into(),
            ));
        }
        let (goal, _, state) = goal_state(ctx).await?;
        let goal = goal.ok_or_else(|| {
            EngineError::Type("no goal to set budget for; call set_goal first".into())
        })?;
        {
            let session = this_session(ctx).await?;
            let s = session.lock().await;
            if !s.goal_active
                && goal.owner.as_ref().is_some_and(
                    |o| !matches!(o, refact_chat_api::GoalOwner::Chat { id } if *id == s.chat_id),
                )
            {
                return Err(EngineError::Permission(
                    crate::chat::queue::unowned_goal_error(&s, "change the budget of"),
                ));
            }
        }
        let old = goal.budget.clone();
        if old.supervised || old.clone().without_hard_limits() != old {
            return Err(EngineError::Permission(
                "this goal is supervised or has hard limits (turns, minutes, tokens, cost or no-progress turns); changing its budget would remove them, so scripts may not change it".into(),
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
        let plan = if items.is_empty() {
            Plan::nothing()
        } else {
            Plan::new(
                state,
                GoalBudgetPlan {
                    budget,
                },
            )
        };
        Ok(Preview {
            title: "change this chat's goal budget".into(),
            items,
            scope: Some(THIS_CHAT_SCOPE.into()),
            class: EngineClass::Change,
            warning: None,
            plan,
        })
    })
}

fn set_goal_budget<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let (goal, _, state) = goal_state(ctx).await?;
        let plan: GoalBudgetPlan = ctx.take_plan("chat.set_goal_budget", &state)?;
        let session = this_session(ctx).await?;
        let request_id = queue_command(
            ctx,
            &session,
            refact_chat_api::ChatCommand::SetGoalBudget {
                budget: plan.budget.clone(),
            },
        )
        .await?;
        Ok(EngineOutput::new(
            json!({"request_id":request_id,"old_budget":goal.map(|g| g.budget),"new_budget":plan.budget,"applies":APPLIES_LATER}),
            "goal budget changes after this turn",
        ))
    })
}

fn stop_goal_preview<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        require_own_chat(ctx).await?;
        let (goal, status, state) = goal_state(ctx).await?;
        if goal.is_none() {
            return Err(EngineError::Type("no goal to stop".into()));
        }
        if status == Some(refact_chat_api::GoalStatus::Stopped) {
            return Err(EngineError::Type("the goal is already stopped".into()));
        }
        {
            let session = this_session(ctx).await?;
            let s = session.lock().await;
            crate::chat::queue::validate_goal_control(&s, "stop").map_err(EngineError::Type)?;
        }
        Ok(Preview {
            title: "stop this chat's goal".into(),
            items: vec![super::mask::item(
                "goal status",
                Some(to_value(status)?),
                Some(json!("stopped")),
            )],
            scope: Some("this chat".into()),
            class: EngineClass::Change,
            warning: None,
            plan: Plan::new(state, ()),
        })
    })
}

fn stop_goal<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let (_, status, state) = goal_state(ctx).await?;
        ctx.take_plan::<()>("chat.stop_goal", &state)?;
        let session = this_session(ctx).await?;
        let request_id = queue_command(
            ctx,
            &session,
            refact_chat_api::ChatCommand::GoalControl {
                action: "stop".into(),
            },
        )
        .await?;
        crate::chat::cancel_recovery_for_goal_stop(&mut *session.lock().await);
        Ok(EngineOutput::new(
            json!({"request_id":request_id,"old_status":status,"new_status":"stopped","applies":APPLIES_LATER}),
            "goal pursuit stops after this turn",
        ))
    })
}

#[derive(Clone)]
struct CancelPlan {
    delivery: bool,
}

async fn queued_item(
    ctx: &EngineCtx,
    id: &str,
) -> Result<(refact_chat_api::QueuedItem, Value), EngineError> {
    let session = this_session(ctx).await?;
    let s = session.lock().await;
    let items = s.build_queued_items();
    let item = items
        .into_iter()
        .find(|i| i.client_request_id == id)
        .ok_or_else(|| {
            EngineError::Type(format!(
                "nothing with id {id} is queued in this chat; see chat.queue"
            ))
        })?;
    let state = json!({"id":item.client_request_id,"command_type":item.command_type,"preview":item.preview});
    Ok((item, state))
}

fn cancel_queued_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        require_own_chat(ctx).await?;
        let id = args["id"].as_str().unwrap_or_default();
        let (item, state) = queued_item(ctx, id).await?;
        let delivery = item.command_type == refact_chat_api::DELIVERY_COMMAND_TYPE;
        let user_message = item.command_type == "user_message";
        Ok(Preview {
            title: "cancel a queued item in this chat".into(),
            items: vec![super::mask::item(
                &item.command_type,
                Some(json!(scrub_text(&item.preview, &[]))),
                Some(json!("removed")),
            )],
            scope: Some("this chat".into()),
            class: if user_message {
                EngineClass::Destructive
            } else {
                EngineClass::Change
            },
            warning: user_message
                .then(|| "The queued message will be deleted and cannot be restored.".to_string()),
            plan: Plan::new(
                state,
                CancelPlan {
                    delivery,
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
        let id = args["id"].as_str().unwrap_or_default().to_string();
        let (item, state) = queued_item(ctx, &id).await?;
        let plan: CancelPlan = ctx.take_plan("chat.cancel_queued", &state)?;
        let removed = if plan.delivery {
            crate::chat::update_pending_delivery_in_chat(
                ctx.app.clone(),
                &ctx.chat_id,
                &id,
                None,
                true,
            )
            .await
            .map_err(EngineError::Failed)?;
            true
        } else {
            let session = this_session(ctx).await?;
            let mut s = session.lock().await;
            let _guard = crate::chat::NativeMutationGuard::enter_locked(&session, &mut s)
                .map_err(EngineError::Permission)?;
            crate::chat::cancel_queued_locked(&mut s, &ctx.chat_id, &id)
        };
        if !removed {
            return Err(EngineError::Failed(format!(
                "{id} was no longer queued; nothing was removed"
            )));
        }
        Ok(EngineOutput::new(
            json!({"removed":true,"kind":if plan.delivery {"delivery"} else {"command"}}),
            format!("removed queued {}", item.command_type),
        ))
    })
}

/// Approval flags: argument name, thread field, warning when the flag is turned on.
pub(super) const APPROVAL_ARGS: &[(&str, &str, &str)] = &[
    (
        "editing_tools",
        "auto_approve_editing_tools",
        "Edits to files will be applied without asking you.",
    ),
    (
        "dangerous_commands",
        "auto_approve_dangerous_commands",
        "Shell commands and other confirmations will run without asking you.",
    ),
    (
        "all_confirmations",
        "autonomous_no_confirm",
        "Every tool except agent control will run without asking you.",
    ),
];

pub(super) const ENGINE_UNAFFECTED: &str =
    "these flags never affect engine functions, which always ask you";

pub(super) fn auto_approve_schema(other_chat: bool) -> Value {
    let mut schema = json!({"type":"object","additionalProperties":false,"required":[],"properties":{
        "editing_tools":{"type":"boolean","description":"Apply edits of editing tools without asking."},
        "dangerous_commands":{"type":"boolean","description":"Run dangerous commands and other confirmations without asking."},
        "all_confirmations":{"type":"boolean","description":"Run every tool except agent control without asking."}
    }});
    if other_chat {
        schema["required"] = json!(["chat_id"]);
        schema["properties"]["chat_id"] = json!({"type":"string","minLength":1});
    }
    schema
}

/// Plans an approval-flag change: only the given flags that differ from the thread, with one
/// warning per flag turned on.
pub(super) fn plan_auto_approve(
    thread: &crate::chat::types::ThreadParams,
    args: &Value,
) -> Result<ParamsPlan, EngineError> {
    if !APPROVAL_ARGS
        .iter()
        .any(|(arg, _, _)| args.get(*arg).is_some())
    {
        return Err(EngineError::Type(
            "no flags given; give at least one of editing_tools, dangerous_commands, all_confirmations".into(),
        ));
    }
    let mut plan = ParamsPlan::default();
    for (arg, key, warning) in APPROVAL_ARGS {
        let Some(new) = args.get(*arg).and_then(Value::as_bool) else {
            continue;
        };
        let old = thread_value(thread, key);
        if old == json!(new) {
            continue;
        }
        plan.patch.insert((*key).into(), json!(new));
        plan.items
            .push(super::mask::item(arg, Some(old.clone()), Some(json!(new))));
        plan.changed
            .insert((*key).into(), json!({"old":old,"new":new}));
        if new {
            plan.warn(warning);
        }
    }
    Ok(plan)
}

fn set_auto_approve_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        require_own_chat(ctx).await?;
        let (thread, _, _, state) = this_chat_snapshot(ctx).await?;
        let plan = plan_auto_approve(&thread, args)?;
        Ok(params_preview(
            "change automatic approvals in this chat",
            THIS_CHAT_SCOPE.into(),
            plan,
            state,
            None,
        ))
    })
}

fn set_auto_approve<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let mut output = this_chat_param_apply(ctx, "chat.set_auto_approve").await?;
        output.text = format!("{}; {ENGINE_UNAFFECTED}", output.text);
        Ok(output)
    })
}
