use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::Mutex as AMutex;

use crate::call_validation::{ChatContent, ChatMessage};
use crate::chat::types::TaskMeta;
use crate::global_context::GlobalContext;
use crate::subchat::{
    resolve_subchat_config_with_explicit_params, run_subchat,
    run_subchat_once_with_explicit_params, ExplicitSubchatSpec, TraceParent,
};
use crate::tools::review_agents::contract::{
    parse_stage_output, parse_verdict_output, StageOutput, Verdict, VerdictOutput, REPAIR_PROMPT,
};
use crate::tools::review_agents::prompts::REVIEW_SYSTEM_PROMPT;
use crate::tools::review_agents::stages::{StageContract, StageSpec};
use crate::tools::review_types::{ReviewFinding, StageRun};
use crate::tools::subagent_phases::get_last_assistant_content;
use crate::worktrees::types::WorktreeMeta;

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn merge_metering(
    target: &mut serde_json::Map<String, Value>,
    source: serde_json::Map<String, Value>,
) {
    for (key, value) in source {
        match (target.get_mut(&key), value) {
            (Some(Value::Number(existing)), Value::Number(incoming)) => {
                let sum = existing.as_f64().unwrap_or(0.0) + incoming.as_f64().unwrap_or(0.0);
                *existing = serde_json::Number::from_f64(sum)
                    .unwrap_or_else(|| serde_json::Number::from(0));
            }
            (Some(slot), incoming) => {
                *slot = incoming;
            }
            (None, incoming) => {
                target.insert(key, incoming);
            }
        }
    }
}

#[derive(Clone)]
pub struct StageCtx {
    pub review_id: String,
    pub tool_call_id: String,
    pub subchat_tx: Arc<AMutex<UnboundedSender<Value>>>,
    pub abort_flag: Arc<AtomicBool>,
    pub depth: usize,
    pub task_meta: Option<TaskMeta>,
    pub worktree: Option<WorktreeMeta>,
    pub chat_id: String,
    pub root_chat_id: String,
}

pub struct StageJob {
    pub spec: Arc<StageSpec>,
    pub label: String,
    pub subchat: ExplicitSubchatSpec,
    pub max_steps: usize,
    pub prompt: String,
    pub trace_chat_id: String,
    pub budget: std::time::Duration,
    pub abort: Arc<AtomicBool>,
}

pub fn stage_trace_chat_id(review_id: &str, label: &str) -> String {
    let slug: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    format!("subchat-{review_id}-{slug}")
}

pub fn monitor_ctx(
    ctx: &StageCtx,
    activity: Arc<AtomicU64>,
) -> (StageCtx, tokio::task::JoinHandle<()>) {
    let (monitor_tx, mut monitor_rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
    let parent_tx = ctx.subchat_tx.clone();
    let forwarder = tokio::spawn(async move {
        while let Some(message) = monitor_rx.recv().await {
            activity.store(now_ms(), Ordering::Relaxed);
            let sender = parent_tx.lock().await;
            let _ = sender.send(message);
        }
    });
    let mut monitored = ctx.clone();
    monitored.subchat_tx = Arc::new(AMutex::new(monitor_tx));
    (monitored, forwarder)
}

pub struct StageProduct {
    pub run: StageRun,
    pub findings: Vec<ReviewFinding>,
    pub verdicts: Vec<Verdict>,
    pub metering: serde_json::Map<String, Value>,
    pub raw: Option<String>,
}

impl StageProduct {
    fn empty(run: StageRun) -> Self {
        Self {
            run,
            findings: vec![],
            verdicts: vec![],
            metering: serde_json::Map::new(),
            raw: None,
        }
    }
}

pub fn inherited_worktree(ctx: &StageCtx) -> Option<WorktreeMeta> {
    ctx.worktree.clone()
}

pub async fn run_stage(gcx: Arc<GlobalContext>, ctx: StageCtx, job: StageJob) -> StageProduct {
    let trace_chat_id = job.trace_chat_id.clone();
    let mut product = run_stage_inner(gcx, ctx, job).await;
    product.run = product.run.with_trace_chat_id(Some(trace_chat_id));
    product
}

async fn run_stage_inner(gcx: Arc<GlobalContext>, ctx: StageCtx, job: StageJob) -> StageProduct {
    let started = now_ms();
    let model = job.subchat.model.clone();
    let label = job.label.clone();
    let mut metering = serde_json::Map::new();
    let worktree = inherited_worktree(&ctx);

    let trace_chat_id = job.trace_chat_id.clone();
    let attribution = format!("review_{}", job.spec.id);
    let mut config = match resolve_subchat_config_with_explicit_params(
        gcx.clone(),
        &attribution,
        &job.subchat,
        true,
        Some(trace_chat_id.clone()),
        Some(job.spec.display_title()),
        Some(ctx.chat_id.clone()),
        Some("review_stage".to_string()),
        Some(ctx.root_chat_id.clone()),
        Some(job.spec.tools()),
        job.max_steps,
        false,
        "agent".to_string(),
        ctx.task_meta.clone(),
        worktree,
        Some(ctx.tool_call_id.clone()),
        Some(ctx.subchat_tx.clone()),
        Some(job.abort.clone()),
        ctx.depth + 1,
    )
    .await
    {
        Ok(config) => config,
        Err(error) => {
            return StageProduct::empty(StageRun::failed(
                &label,
                Some(model),
                now_ms().saturating_sub(started),
                &error,
            ));
        }
    };
    config.soft_abort = true;

    let messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: ChatContent::SimpleText(REVIEW_SYSTEM_PROMPT.to_string()),
            ..Default::default()
        },
        ChatMessage::new("user".to_string(), job.prompt.clone()),
    ];

    let result = match run_subchat(gcx.clone(), messages, config).await {
        Ok(result) => result,
        Err(error) => {
            return StageProduct::empty(StageRun::failed(
                &label,
                Some(model),
                now_ms().saturating_sub(started),
                &error,
            ));
        }
    };
    merge_metering(&mut metering, result.metering.clone());
    let text = get_last_assistant_content(&result.messages);

    let parsed = match job.spec.contract {
        StageContract::Findings => parse_stage_output(&text)
            .map(Parsed::Findings)
            .map_err(|error| error.to_string()),
        StageContract::Verdicts => parse_verdict_output(&text)
            .map(Parsed::Verdicts)
            .map_err(|error| error.to_string()),
    };

    let parsed = match parsed {
        Ok(parsed) => Ok(parsed),
        Err(first_error) if job.abort.load(Ordering::SeqCst) => {
            Err(format!("output_contract: {first_error}"))
        }
        Err(first_error) => {
            tracing::info!("review: {label} broke the output contract ({first_error}), repairing");
            let mut retry_messages = result.messages.clone();
            retry_messages.push(ChatMessage::new(
                "user".to_string(),
                format!("{REPAIR_PROMPT} {first_error}"),
            ));
            match run_subchat_once_with_explicit_params(
                gcx.clone(),
                &attribution,
                &job.subchat,
                retry_messages,
                ctx.tool_call_id.clone(),
                ctx.subchat_tx.clone(),
                job.abort.clone(),
                ctx.depth,
                ctx.task_meta.clone(),
                ctx.worktree.clone(),
                TraceParent::rooted(&ctx.chat_id, &ctx.root_chat_id),
            )
            .await
            {
                Ok(retry) => {
                    merge_metering(&mut metering, retry.metering.clone());
                    let retry_text = get_last_assistant_content(&retry.messages);
                    let repaired = match job.spec.contract {
                        StageContract::Findings => parse_stage_output(&retry_text)
                            .map(Parsed::Findings)
                            .map_err(|error| error.to_string()),
                        StageContract::Verdicts => parse_verdict_output(&retry_text)
                            .map(Parsed::Verdicts)
                            .map_err(|error| error.to_string()),
                    };
                    repaired.map_err(|error| format!("output_contract: {error}"))
                }
                Err(retry_error) => Err(format!("output_contract: repair failed: {retry_error}")),
            }
        }
    };

    let duration = now_ms().saturating_sub(started);
    match parsed {
        Ok(Parsed::Findings(output)) => {
            let mut product = findings_product(&label, &job, output, model, duration);
            merge_metering(&mut product.metering, metering);
            product.raw = Some(text);
            product
        }
        Ok(Parsed::Verdicts(output)) => {
            let mut product = verdicts_product(&label, output, model, duration);
            merge_metering(&mut product.metering, metering);
            product.raw = Some(text);
            product
        }
        Err(reason) => {
            let mut product =
                StageProduct::empty(StageRun::failed(&label, Some(model), duration, &reason));
            product.metering = metering;
            product.raw = Some(text);
            product
        }
    }
}

enum Parsed {
    Findings(StageOutput),
    Verdicts(VerdictOutput),
}

pub(crate) fn findings_product(
    label: &str,
    job: &StageJob,
    output: StageOutput,
    model: String,
    duration_ms: u64,
) -> StageProduct {
    let findings: Vec<ReviewFinding> = output
        .findings
        .into_iter()
        .map(|raw| ReviewFinding {
            id: String::new(),
            stage: job.spec.id.clone(),
            model: Some(model.clone()),
            title: raw.title,
            severity: raw.severity,
            file: raw.file,
            line_start: raw.line_start,
            line_end: raw.line_end,
            claim: raw.claim,
            evidence: raw.evidence,
            evidence_present: false,
            reproduction: raw.reproduction,
            fix: raw.fix,
            introduced_by_diff: false,
            out_of_scope: false,
            reported_by: vec![job.spec.id.clone()],
            locations: vec![],
            disputed: None,
        })
        .collect();

    let mut run = StageRun::ok(label, Some(model), duration_ms);
    run.findings = findings.len();
    run.coverage = output.coverage;
    run.summary = summary_with_rejections(output.summary, &output.rejected);
    StageProduct {
        run,
        findings,
        verdicts: vec![],
        metering: serde_json::Map::new(),
        raw: None,
    }
}

fn verdicts_product(
    label: &str,
    output: VerdictOutput,
    model: String,
    duration_ms: u64,
) -> StageProduct {
    let mut run = StageRun::ok(label, Some(model), duration_ms);
    run.findings = output.verdicts.iter().filter(|v| !v.supported).count();
    run.coverage = output.coverage;
    run.summary = summary_with_rejections(output.summary, &output.rejected);
    StageProduct {
        run,
        findings: vec![],
        verdicts: output.verdicts,
        metering: serde_json::Map::new(),
        raw: None,
    }
}

fn summary_with_rejections(summary: String, rejected: &[String]) -> Option<String> {
    let summary = summary.trim().to_string();
    match (summary.is_empty(), rejected.is_empty()) {
        (true, true) => None,
        (false, true) => Some(summary),
        (true, false) => Some(format!("{} malformed entries skipped", rejected.len())),
        (false, false) => Some(format!(
            "{summary} ({} malformed entries skipped)",
            rejected.len()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::review_agents::contract::parse_stage_output;
    use crate::tools::review_agents::stages::embedded_catalog;

    fn job(stage_id: &str) -> StageJob {
        StageJob {
            spec: Arc::new(
                embedded_catalog()
                    .into_iter()
                    .find(|spec| spec.id == stage_id)
                    .unwrap(),
            ),
            label: stage_id.to_string(),
            subchat: ExplicitSubchatSpec {
                params: crate::call_validation::SubchatParameters {
                    subchat_model_type: crate::call_validation::ChatModelType::Default,
                    subchat_model: "test-model".to_string(),
                    subchat_n_ctx: 100000,
                    subchat_max_new_tokens: 8000,
                    subchat_temperature: None,
                    subchat_tokens_for_rag: 0,
                    subchat_reasoning_effort: None,
                    subchat_cache_control: crate::llm::params::CacheControl::Off,
                },
                model: "test-model".to_string(),
                autonomous_no_confirm: true,
            },
            max_steps: 10,
            prompt: String::new(),
            trace_chat_id: stage_trace_chat_id("rv-1", stage_id),
            budget: std::time::Duration::from_secs(60),
            abort: Arc::new(AtomicBool::new(false)),
        }
    }

    fn ctx_with_worktree(worktree: Option<WorktreeMeta>) -> StageCtx {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
        StageCtx {
            review_id: "rv-1".to_string(),
            tool_call_id: "tc-1".to_string(),
            subchat_tx: Arc::new(AMutex::new(tx)),
            abort_flag: Arc::new(AtomicBool::new(false)),
            depth: 0,
            task_meta: None,
            worktree,
            chat_id: "chat".to_string(),
            root_chat_id: "chat".to_string(),
        }
    }

    #[test]
    fn review_runner_every_stage_runs_in_the_caller_worktree() {
        let meta = WorktreeMeta {
            id: "wt-1".to_string(),
            kind: "chat".to_string(),
            root: std::path::PathBuf::from("/tmp/wt-1"),
            source_workspace_root: std::path::PathBuf::from("/tmp/workspace"),
            repo_root: std::path::PathBuf::from("/tmp/workspace"),
            branch: Some("refact/chat/abc".to_string()),
            base_branch: None,
            base_commit: None,
            task_id: None,
            card_id: None,
            agent_id: None,
            enforce: false,
        };

        let inherited = inherited_worktree(&ctx_with_worktree(Some(meta.clone())));

        assert_eq!(inherited.as_ref().map(|w| w.id.as_str()), Some("wt-1"));
        assert_eq!(
            inherited.and_then(|w| w.branch),
            Some("refact/chat/abc".to_string())
        );
        assert!(inherited_worktree(&ctx_with_worktree(None)).is_none());
    }

    #[test]
    fn review_runner_maps_contract_findings_onto_review_findings() {
        let output = parse_stage_output(
            r#"{"findings":[{"title":"t","severity":"high","file":"src/lib.rs","line_start":4,"line_end":6,"claim":"c","evidence":"e","reproduction":"cargo test","fix":"f"}],"summary":"ok","coverage":{"files_read":["src/lib.rs"],"commands_run":[{"cmd":"cargo test","exit":1}]}}"#,
        )
        .unwrap();

        let product = findings_product("diff", &job("diff"), output, "m".to_string(), 42);

        assert_eq!(product.run.findings, 1);
        assert_eq!(product.run.duration_ms, 42);
        assert_eq!(product.run.coverage.commands_run[0].exit, 1);
        assert_eq!(product.findings[0].stage, "diff");
        assert_eq!(product.findings[0].reported_by, ["diff"]);
        assert!(!product.findings[0].evidence_present);
        assert!(!product.findings[0].introduced_by_diff);
        assert_eq!(product.run.summary.as_deref(), Some("ok"));
    }

    #[test]
    fn review_runner_records_malformed_entries_in_the_stage_summary() {
        assert_eq!(
            summary_with_rejections("checked".to_string(), &["bad".to_string()]),
            Some("checked (1 malformed entries skipped)".to_string())
        );
        assert_eq!(summary_with_rejections(String::new(), &[]), None);
    }

    #[test]
    fn review_runner_trace_chat_id_is_deterministic_and_slug_safe() {
        assert_eq!(
            stage_trace_chat_id("rv-b08fc869", "diff"),
            "subchat-rv-b08fc869-diff"
        );
        assert_eq!(
            stage_trace_chat_id("rv-1", "impact@thinking"),
            "subchat-rv-1-impact-thinking"
        );
    }

    #[tokio::test]
    async fn review_runner_monitor_ctx_forwards_messages_and_stamps_activity() {
        let (parent_tx, mut parent_rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
        let mut ctx = ctx_with_worktree(None);
        ctx.subchat_tx = Arc::new(AMutex::new(parent_tx));
        let activity = Arc::new(AtomicU64::new(1));

        let (monitored, forwarder) = monitor_ctx(&ctx, activity.clone());
        monitored
            .subchat_tx
            .lock()
            .await
            .send(serde_json::json!({"subchat_id": "progress"}))
            .unwrap();
        let received = tokio::time::timeout(std::time::Duration::from_secs(2), parent_rx.recv())
            .await
            .unwrap()
            .unwrap();

        assert_eq!(received["subchat_id"], "progress");
        assert!(activity.load(Ordering::Relaxed) > 1);
        forwarder.abort();
    }

    #[test]
    fn review_runner_metering_sums_numbers_and_replaces_other_values() {
        let mut target = serde_json::Map::new();
        target.insert("tokens".to_string(), serde_json::json!(10));
        target.insert("model".to_string(), serde_json::json!("a"));
        let mut source = serde_json::Map::new();
        source.insert("tokens".to_string(), serde_json::json!(5));
        source.insert("model".to_string(), serde_json::json!("b"));
        source.insert("new".to_string(), serde_json::json!(1));

        merge_metering(&mut target, source);

        assert_eq!(target["tokens"], serde_json::json!(15.0));
        assert_eq!(target["model"], serde_json::json!("b"));
        assert_eq!(target["new"], serde_json::json!(1));
    }
}
