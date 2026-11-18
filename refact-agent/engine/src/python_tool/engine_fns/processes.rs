use futures::future::BoxFuture;
use serde_json::{json, Value};

use super::common::{limit, limit_schema, redact, require_root_chat, PathFilter};
use super::mask::item;
use super::{previewed, EngineClass, EngineCtx, EngineError, EngineFn, EngineOutput, Preview};
use crate::tool_jobs::JobStatus;
use refact_exec::{ExecProcessFilter, ExecStatus};

const HIDDEN: &str = "<hidden: other chat>";

/// Tools whose jobs only read; cancelling any other job midway can leave partial effects.
const READ_ONLY_TOOLS: &[&str] = &[
    "cat",
    "tree",
    "glob",
    "search_pattern",
    "search_symbol_definition",
    "search_trajectories",
    "knowledge",
    "web_search",
    "get_goal",
    "get_trajectory_context",
    "work_overview",
    "task_overview",
    "check_agents",
    "agent_diff",
    "causal_trace",
    "metrics",
    "dead_code",
    "code_map",
    "code_why",
    "code_health",
    "code_duplication",
    "codegraph_overview",
    "git_risk",
    "security_scan",
    "pr_blast",
    "design_system",
];

pub(super) fn effectful(tool: &str) -> bool {
    !READ_ONLY_TOOLS.contains(&tool)
}

pub fn functions() -> Vec<EngineFn> {
    vec![
        EngineFn {
            name: "processes.list",
            summary: "Exec processes of every chat in this engine, across all workspaces, with owner chat, status, a redacted command preview and exit code. Never returns output; other chats' commands naming a path this chat's model may not see are hidden.",
            class: EngineClass::RedactedRead,
            input: || json!({"type":"object","additionalProperties":false,"properties":{"status":{"enum":["running","completed","all"],"description":"running by default."},"chat_id":{"type":"string","minLength":1,"description":"Only processes owned by this chat."},"limit":limit_schema()}}),
            output: || Some(json!({"type":"object","required":["processes","total","truncated"],"properties":{"processes":{"type":"array","items":{"type":"object","properties":{"process_id":{"type":"string"},"status":{"type":"string"},"command_preview":{"type":"string"},"mode":{"type":"string"},"service_name":{"type":"string"},"owner_chat_id":{"type":"string"},"own":{"type":"boolean"},"tty":{"type":"boolean"},"created_at_ms":{"type":"integer"},"ended_at_ms":{"type":"integer"},"exit_code":{"type":"integer"},"workspace":{"type":"string"}}}},"total":{"type":"integer"},"truncated":{"type":"boolean"}}})),
            handler: list,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "processes.jobs",
            summary: "Tool jobs (long-running tool calls) of this chat or of every chat, with status and tool name; never their results (use the tool_job tool in the owning chat).",
            class: EngineClass::Read,
            input: || json!({"type":"object","additionalProperties":false,"properties":{"scope":{"enum":["chat","all"],"description":"chat (default) or all."},"status":{"type":"array","minItems":1,"uniqueItems":true,"items":{"enum":["accepted","running","cancel_requested","succeeded","failed","cancelled","unknown"]}},"limit":limit_schema()}}),
            output: || Some(json!({"type":"object","required":["jobs","total","truncated"],"properties":{"jobs":{"type":"array","items":{"type":"object","properties":{"job_id":{"type":"string"},"chat_id":{"type":"string"},"tool_name":{"type":"string"},"tool_call_id":{"type":"string"},"model":{"type":"string"},"turn_id":{"type":"string"},"status":{"type":"string"},"revision":{"type":"integer"},"result_available":{"type":"boolean"},"result_released":{"type":"boolean"}}}},"total":{"type":"integer"},"truncated":{"type":"boolean"}}})),
            handler: jobs,
            preview: None,
            timeout: None,
        },
        EngineFn {
            name: "processes.cancel_job",
            summary: "Request cancellation of a running tool job of another chat, after the user approves. Cooperative: the job may still end failed or unknown; editing and shell jobs make this destructive.",
            class: EngineClass::Change,
            input: || json!({"type":"object","additionalProperties":false,"required":["chat_id","job_id"],"properties":{"chat_id":{"type":"string","minLength":1},"job_id":{"type":"string","minLength":1}}}),
            output: || Some(json!({"type":"object","required":["job_id","status"],"properties":{"job_id":{"type":"string"},"status":{"type":"string"}}})),
            handler: cancel_job,
            preview: Some(cancel_job_preview),
            timeout: None,
        },
        EngineFn {
            name: "processes.kill",
            summary: "Kill an exec process owned by another chat or by no chat, after the user approves. Not for this chat's own processes (use process_kill) or managed services.",
            class: EngineClass::Destructive,
            input: || json!({"type":"object","additionalProperties":false,"required":["process_id"],"properties":{"process_id":{"type":"string","minLength":1}}}),
            output: || Some(json!({"type":"object","required":["process_id","status"],"properties":{"process_id":{"type":"string"},"status":{"type":"string"},"exit_code":{"type":"integer"}}})),
            handler: kill,
            preview: Some(kill_preview),
            timeout: None,
        },
    ]
}

const KILL_WARNING: &str = "The process is stopped immediately; any work it was doing is lost and its owning chat sees it fail.";

struct KillPlan {
    snapshot: refact_exec::ExecProcessSnapshot,
    preview: String,
}

async fn plan_kill(ctx: &EngineCtx, args: &Value) -> Result<KillPlan, EngineError> {
    require_root_chat(ctx, "processes.kill").await?;
    let id = refact_exec::ExecProcessId(args["process_id"].as_str().unwrap_or_default().into());
    let snapshot = ctx
        .app
        .runtime
        .exec_registry
        .get(&id)
        .await
        .ok_or_else(|| EngineError::Type(format!("no process {id}")))?;
    let owner = &snapshot.meta.owner;
    let (_, own_chats) = super::agents::own_tree(ctx).await;
    if owner
        .chat_id
        .as_ref()
        .is_some_and(|chat| own_chats.contains(chat))
    {
        return Err(EngineError::Type(
            "this process belongs to this chat or one of its agents; use the process_kill tool"
                .into(),
        ));
    }
    if owner.service_name.is_some() {
        return Err(EngineError::Permission(
            "this process is a managed service; its integration owns and restarts it".into(),
        ));
    }
    if snapshot.status.is_terminal() {
        return Err(EngineError::Failed(format!(
            "process {id} already ended ({})",
            status_label(&snapshot.status)
        )));
    }
    let folders = ctx
        .app
        .gcx
        .documents_state
        .workspace_folders
        .lock()
        .unwrap()
        .clone();
    if !owner
        .workspace
        .as_ref()
        .is_some_and(|workspace| folders.iter().any(|folder| workspace.starts_with(folder)))
    {
        return Err(EngineError::Permission(format!(
            "process {id} does not run in one of this engine's workspace folders"
        )));
    }
    let filter = PathFilter::new(ctx).await?;
    let preview = if filter.allows_text(&snapshot.meta.short_description) {
        redact(&super::mask::scrub_text(&snapshot.meta.short_description))
    } else {
        HIDDEN.to_string()
    };
    Ok(KillPlan {
        snapshot,
        preview,
    })
}

fn kill_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let plan = plan_kill(ctx, args).await?;
        let age_s = crate::agents::approval::now_ms()
            .saturating_sub(plan.snapshot.meta.created_at_ms)
            / 1000;
        let owner = plan
            .snapshot
            .meta
            .owner
            .chat_id
            .as_deref()
            .map_or("none".to_string(), |chat| format!("chat {chat}"));
        Ok(Preview {
            title: "kill another chat's process".into(),
            items: vec![
                item(
                    "process",
                    Some(json!(format!("{} (running {age_s} s)", plan.preview))),
                    Some(json!("killed")),
                ),
                item("owner", None, Some(json!(owner))),
            ],
            scope: Some("engine".into()),
            class: EngineClass::Destructive,
            warning: Some(KILL_WARNING.into()),
            plan: previewed::plan(json!(status_label(&plan.snapshot.status))),
        })
    })
}

fn kill<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let plan = plan_kill(ctx, args).await?;
        previewed::take(
            ctx,
            "processes.kill",
            &json!(status_label(&plan.snapshot.status)),
        )?;
        let snapshot = ctx
            .app
            .runtime
            .exec_registry
            .kill(&plan.snapshot.meta.process_id)
            .await
            .map_err(EngineError::Failed)?;
        let mut data = json!({
            "process_id": snapshot.meta.process_id.as_str(),
            "status": status_label(&snapshot.status),
        });
        if let Some(code) = exit_code(&snapshot.status) {
            data["exit_code"] = json!(code);
        }
        Ok(EngineOutput::new(data, format!("killed {}", plan.preview)))
    })
}

fn status_label(status: &ExecStatus) -> &'static str {
    match status {
        ExecStatus::Starting => "starting",
        ExecStatus::Running => "running",
        ExecStatus::Exited {
            ..
        } => "exited",
        ExecStatus::SandboxLauncherFailed {
            ..
        } => "sandbox_launcher_failed",
        ExecStatus::Failed {
            ..
        } => "failed",
        ExecStatus::Killed => "killed",
        ExecStatus::TimedOut => "timed_out",
    }
}

fn exit_code(status: &ExecStatus) -> Option<i32> {
    match status {
        ExecStatus::Exited {
            exit_code,
        } => *exit_code,
        ExecStatus::SandboxLauncherFailed {
            exit_code,
        } => Some(*exit_code),
        _ => None,
    }
}

fn list<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        require_root_chat(ctx, "processes.list").await?;
        let wanted = args
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("running");
        let owner = args.get("chat_id").and_then(Value::as_str);
        let filter = PathFilter::new(ctx).await?;
        let mut snapshots = ctx
            .app
            .runtime
            .exec_registry
            .list(ExecProcessFilter::default())
            .await;
        snapshots.retain(|s| match wanted {
            "running" => matches!(s.status, ExecStatus::Starting | ExecStatus::Running),
            "completed" => s.status.is_terminal(),
            _ => true,
        });
        snapshots.retain(|s| owner.is_none_or(|o| s.meta.owner.chat_id.as_deref() == Some(o)));
        snapshots.sort_by(|a, b| b.meta.created_at_ms.cmp(&a.meta.created_at_ms));
        let total = snapshots.len();
        let limit = limit(args);
        let mut running = 0;
        let mut owned = 0;
        let mut rows = Vec::new();
        for s in &snapshots {
            let own = s.meta.owner.chat_id.as_deref() == Some(ctx.chat_id.as_str());
            running += usize::from(!s.status.is_terminal());
            owned += usize::from(own);
            if rows.len() >= limit {
                continue;
            }
            let preview = if !own && !filter.allows_text(&s.meta.short_description) {
                HIDDEN.to_string()
            } else {
                redact(&crate::integrations::mcp::redaction::scrub_text(
                    &s.meta.short_description,
                    &[],
                ))
            };
            let mut row = json!({
                "process_id": s.meta.process_id.as_str(),
                "status": status_label(&s.status),
                "command_preview": preview,
                "mode": s.meta.mode.to_string(),
                "own": own,
                "tty": s.meta.tty,
                "created_at_ms": s.meta.created_at_ms,
            });
            if let Some(chat) = &s.meta.owner.chat_id {
                row["owner_chat_id"] = json!(chat);
            }
            if let Some(service) = &s.meta.owner.service_name {
                row["service_name"] = json!(service);
            }
            if let Some(ended) = s.meta.ended_at_ms {
                row["ended_at_ms"] = json!(ended);
            }
            if let Some(code) = exit_code(&s.status) {
                row["exit_code"] = json!(code);
            }
            if let Some(workspace) = &s.meta.owner.workspace {
                row["workspace"] = json!(workspace.to_string_lossy());
            }
            rows.push(row);
        }
        let text = format!("{total} processes ({running} running, {owned} owned by this chat)");
        Ok(EngineOutput::new(
            json!({"processes":rows,"total":total,"truncated":total > limit}),
            text,
        ))
    })
}

fn status_name(status: JobStatus) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default()
}

fn jobs<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let all = args.get("scope").and_then(Value::as_str) == Some("all");
        if all {
            require_root_chat(ctx, "processes.jobs scope=all").await?;
        }
        let statuses: Option<Vec<String>> = args.get("status").and_then(Value::as_array).map(|s| {
            s.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        });
        let mut jobs = ctx
            .app
            .runtime
            .tool_jobs
            .list((!all).then_some(ctx.chat_id.as_str()));
        jobs.retain(|(job, _)| {
            statuses
                .as_ref()
                .is_none_or(|s| s.contains(&status_name(job.status)))
        });
        jobs.sort_by(|a, b| a.0.receipt.job_id.cmp(&b.0.receipt.job_id));
        let total = jobs.len();
        let running = jobs.iter().filter(|(j, _)| !j.status.is_terminal()).count();
        let limit = limit(args);
        let rows: Vec<Value> = jobs
            .into_iter()
            .take(limit)
            .map(|(job, kept)| {
                let b = &job.receipt.binding;
                json!({
                    "job_id": job.receipt.job_id,
                    "chat_id": b.chat_id,
                    "tool_name": b.tool_name,
                    "tool_call_id": b.tool_call_id,
                    "model": b.model,
                    "turn_id": b.turn_id,
                    "status": status_name(job.status),
                    "revision": job.revision,
                    "result_available": kept,
                    "result_released": job.result_released,
                })
            })
            .collect();
        Ok(EngineOutput::new(
            json!({"jobs":rows,"total":total,"truncated":total > limit}),
            format!("{total} jobs ({running} running)"),
        ))
    })
}

struct JobPlan {
    tool_name: String,
    status: JobStatus,
}

async fn plan_cancel(ctx: &EngineCtx, args: &Value) -> Result<JobPlan, EngineError> {
    require_root_chat(ctx, "processes.cancel_job").await?;
    let chat_id = args["chat_id"].as_str().unwrap_or_default();
    let job_id = args["job_id"].as_str().unwrap_or_default();
    if super::agents::own_tree(ctx).await.1.contains(chat_id) {
        return Err(EngineError::Type(
            "this job belongs to this chat or one of its agents; use the tool_job tool to cancel it".into(),
        ));
    }
    let job = ctx
        .app
        .runtime
        .tool_jobs
        .list(Some(chat_id))
        .into_iter()
        .map(|(job, _)| job)
        .find(|job| job.receipt.job_id == job_id)
        .ok_or_else(|| EngineError::Type(format!("no job {job_id} in chat {chat_id}")))?;
    if job.status.is_terminal() {
        return Err(EngineError::Failed(format!(
            "job {job_id} already ended ({})",
            status_name(job.status)
        )));
    }
    Ok(JobPlan {
        tool_name: job.receipt.binding.tool_name.clone(),
        status: job.status,
    })
}

fn cancel_job_preview<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<Preview, EngineError>> {
    Box::pin(async move {
        let plan = plan_cancel(ctx, args).await?;
        let effectful = effectful(&plan.tool_name);
        let approved = previewed::plan(json!(status_name(plan.status)));
        Ok(Preview {
            plan: approved,
            title: "cancel another chat's tool job".into(),
            items: vec![item(
                "job",
                Some(json!(format!("{} {}", plan.tool_name, status_name(plan.status)))),
                Some(json!("cancel requested")),
            )],
            scope: Some(format!("chat {}", args["chat_id"].as_str().unwrap_or_default())),
            class: if effectful {
                EngineClass::Destructive
            } else {
                EngineClass::Change
            },
            warning: effectful.then(|| {
                "The job may already have changed files or run commands; cancelling stops it midway and leaves partial effects.".to_string()
            }),
        })
    })
}

fn cancel_job<'a>(
    ctx: &'a EngineCtx,
    args: &'a Value,
) -> BoxFuture<'a, Result<EngineOutput, EngineError>> {
    Box::pin(async move {
        let plan = plan_cancel(ctx, args).await?;
        previewed::take(
            ctx,
            "processes.cancel_job",
            &json!(status_name(plan.status)),
        )?;
        let chat_id = args["chat_id"].as_str().unwrap_or_default();
        let job_id = args["job_id"].as_str().unwrap_or_default();
        let status = ctx
            .app
            .runtime
            .tool_jobs
            .cancel(chat_id, job_id)
            .ok_or_else(|| EngineError::Failed(format!("job {job_id} is gone")))?;
        Ok(EngineOutput::new(
            json!({"job_id":job_id,"status":status_name(status)}),
            format!("cancel requested for {} job", plan.tool_name),
        ))
    })
}
