use futures::future::BoxFuture;
use serde_json::{json, Value};

use super::common::{redact, to_value};
use super::{EngineClass, EngineCtx, EngineError, EngineFn, EngineOutput};

pub fn functions() -> Vec<EngineFn> {
    vec![
        EngineFn {
            name: "session.limits",
            summary: "The effective python_* limits of this project: run time, memory, nested and model calls and more.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{}}),
            output: || Some(json!({"type":"object","required":["limits"],"properties":{"limits":{"type":"object","additionalProperties":{}}}})),
            handler: limits,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "session.status",
            summary: "This chat's Python session: live, running, background, state size, last reset reason, disk use and the variables as saved after the previous run (in-script dir() is the live view).",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{"variables":{"type":"boolean","description":"Include the saved variable list; true by default."}}}),
            output: || Some(json!({"type":"object","required":["status","live","state_persisted","state_bytes","last_run_restored","disk","active_run_ids"],"properties":{"status":{"type":"string"},"live":{"type":"boolean"},"state_persisted":{"type":"boolean"},"state_bytes":{"type":"integer"},"saved_at_ms":{"type":"integer"},"last_run_id":{"type":"string"},"reset_reason":{"type":"string"},"last_run_restored":{"type":"boolean"},"variables":{"type":"array","items":{"type":"object","properties":{"name":{"type":"string"},"type_name":{"type":"string"},"preview":{"type":"string"}}}},"variables_truncated":{"type":"boolean"},"variables_total":{"type":"integer"},"disk":{"type":"object","properties":{"workspace_bytes":{"type":"integer"},"quota_bytes":{"type":"integer"}}},"active_run_ids":{"type":"array","items":{"type":"string"}}}})),
            handler: status,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "session.runs",
            summary: "This chat's recent Python runs, newest first, with status, call counts, model calls and timing.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{"limit":{"type":"integer","minimum":1,"maximum":100,"description":"Runs to return; 20 by default."}}}),
            output: || Some(json!({"type":"object","required":["runs","total","truncated"],"properties":{"runs":{"type":"array","items":{"type":"object","properties":{"run_id":{"type":"string"},"status":{"type":"string"},"started_at_ms":{"type":"integer"},"finished_at_ms":{"type":"integer"},"calls_total":{"type":"integer"},"background":{"type":"boolean"},"description":{"type":"string"},"code_preview":{"type":"string"},"llm_calls":{"type":"integer"},"displays_total":{"type":"integer"}}}},"total":{"type":"integer"},"truncated":{"type":"boolean"}}})),
            handler: runs,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "session.budget",
            summary: "What this run has left: nested calls, llm() calls and tokens, and time until the run deadline.",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{}}),
            output: || Some(json!({"type":"object","required":["run_id","nested_calls","parallel_max","llm","result_chars_max"],"properties":{"run_id":{"type":"string"},"nested_calls":{"type":"object","properties":{"used":{"type":"integer"},"max":{"type":"integer"}}},"parallel_max":{"type":"integer"},"llm":{"type":"object","properties":{"enabled":{"type":"boolean"},"calls_used":{"type":"integer"},"calls_max":{"type":"integer"},"tokens_used":{"type":"integer"},"tokens_max":{"type":"integer"},"models":{"type":"array","items":{"type":"string"}}}},"deadline_in_ms":{"type":["integer","null"]},"result_chars_max":{"type":"integer"}}})),
            handler: budget,
            preview: None,
            timeout: None,
        },
    ]
}

fn budget<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    use std::sync::atomic::Ordering;
    Box::pin(async move {
        let llm = match &ctx.llm {
            Some(budget) => {
                let used = budget.input_tokens.load(Ordering::Acquire)
                    + budget.output_tokens.load(Ordering::Acquire);
                let models: Vec<String> = budget
                    .models
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .iter()
                    .cloned()
                    .collect();
                json!({
                    "enabled": true,
                    "calls_used": budget.started.load(Ordering::Acquire),
                    "calls_max": budget.max_calls,
                    "tokens_used": used,
                    "tokens_max": budget.max_tokens,
                    "models": models,
                })
            }
            None => json!({"enabled": false}),
        };
        let deadline_in_ms = ctx
            .app
            .python_tool
            .run_deadline(&ctx.chat_id, &ctx.outer_tool_call_id)
            .map(|deadline| {
                deadline
                    .saturating_duration_since(tokio::time::Instant::now())
                    .as_millis() as u64
            });
        let text = format!(
            "{}/{} nested calls, {} llm calls, {}",
            ctx.nested.used,
            ctx.nested.limit,
            if llm["enabled"] == true {
                format!("{}/{}", llm["calls_used"], llm["calls_max"])
            } else {
                "no".into()
            },
            deadline_in_ms.map_or("no deadline".into(), |ms| format!("{} s left", ms / 1000))
        );
        Ok(EngineOutput::new(
            json!({
                "run_id": ctx.run_id,
                "nested_calls": {"used": ctx.nested.used, "max": ctx.nested.limit},
                "parallel_max": ctx.nested.parallel_limit,
                "llm": llm,
                "deadline_in_ms": deadline_in_ms,
                "result_chars_max": ctx.settings.python_max_result_chars,
            }),
            text,
        ))
    })
}

fn limits<'a>(
    ctx: &'a EngineCtx,
    _args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let rendered =
            serde_json::to_value(&ctx.settings).map_err(|e| EngineError::Failed(e.to_string()))?;
        let limits: serde_json::Map<String, Value> = rendered
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(key, _)| key.starts_with("python_"))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        let text = format!(
            "{} python limits; nested calls {}, model calls {}, run {} ms",
            limits.len(),
            ctx.settings.python_max_nested_calls,
            ctx.settings.python_max_llm_calls,
            ctx.settings.python_max_run_ms
        );
        Ok(EngineOutput::new(json!({ "limits": limits }), text))
    })
}

async fn view(
    ctx: &EngineCtx,
) -> Result<crate::python_tool::workspace_view::WorkspaceView, EngineError> {
    crate::python_tool::workspace_view::workspace_view(&ctx.app, &ctx.chat_id)
        .await
        .map_err(EngineError::Failed)
}

fn status<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let view = view(ctx).await?;
        let mut session = to_value(&view.session)?;
        let variables = session["variables"].as_array().map_or(0, Vec::len);
        if args.get("variables").and_then(Value::as_bool) == Some(false) {
            if let Some(map) = session.as_object_mut() {
                map.remove("variables");
            }
        } else if let Some(rows) = session["variables"].as_array_mut() {
            for row in rows {
                if let Some(preview) = row["preview"].as_str() {
                    row["preview"] = json!(redact(preview));
                }
            }
        }
        session["disk"] = to_value(&view.disk)?;
        session["active_run_ids"] = json!(ctx.app.python_tool.active_run_ids(&ctx.chat_id));
        let text = format!(
            "session {}, {} variables, {:.1} MB of {:.1} MB used",
            view.session.status,
            variables,
            view.disk.workspace_bytes as f64 / 1e6,
            view.disk.quota_bytes as f64 / 1e6
        );
        Ok(EngineOutput::new(session, text))
    })
}

fn runs<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
        let view = view(ctx).await?;
        let total = view.runs.len();
        let mut runs = to_value(&view.runs)?;
        let rows = runs.as_array_mut().map(std::mem::take).unwrap_or_default();
        let rows: Vec<Value> = rows
            .into_iter()
            .take(limit)
            .map(|mut row| {
                if let Some(code) = row["code_preview"].as_str() {
                    row["code_preview"] = json!(redact(code));
                }
                row
            })
            .collect();
        let truncated = total > limit || view.runs_truncated;
        let text = format!("{} runs", rows.len());
        Ok(EngineOutput::new(
            json!({"runs":rows,"total":total,"truncated":truncated}),
            text,
        ))
    })
}
