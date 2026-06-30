use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::{Mutex as AMutex, Semaphore};
use tokio::task::JoinSet;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::global_context::GlobalContext;
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::subchat::{resolve_subchat_config_with_parent, run_subchat};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};
use crate::worktrees::types::WorktreeMeta;
use refact_chat_api::TaskMeta;

const TOOL_NAME: &str = "swarm_investigate";
const SUBCHAT_TOOL_NAME: &str = "subagent";
const DEFAULT_MAX_PARALLEL: usize = 3;
const HARD_MAX_PARALLEL: usize = 5;
const DEFAULT_PER_AGENT_BUDGET: usize = 15;
const MAX_PER_AGENT_BUDGET: usize = 50;
const MAX_TOTAL_RESULT_BYTES: usize = 30 * 1024;
const TRUNCATION_NOTE: &str = "\n\n… truncated\n";
const INVESTIGATION_SYSTEM_PROMPT: &str =
    "You are investigating one specific question. Be thorough but concise. Return a structured finding.";

const READ_ONLY_TOOLS: &[&str] = &[
    "cat",
    "tree",
    "search_pattern",
    "search_symbol_definition",
    "search_semantic",
    "knowledge",
];

pub struct ToolSwarmInvestigate {
    pub config_path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SwarmInput {
    topic: String,
    questions: Vec<String>,
    max_parallel: usize,
    per_agent_budget: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SwarmJob {
    index: usize,
    topic: String,
    question: String,
    per_agent_budget: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SwarmFinding {
    Success(String),
    Failed(String),
    TimedOut(Duration),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SwarmQuestionResult {
    index: usize,
    question: String,
    finding: SwarmFinding,
}

#[derive(Clone)]
struct RealSubchatRunnerContext {
    gcx: Arc<GlobalContext>,
    parent_subchat_tx: Arc<AMutex<tokio::sync::mpsc::UnboundedSender<Value>>>,
    parent_tool_call_id: String,
    parent_abort_flag: Arc<std::sync::atomic::AtomicBool>,
    parent_depth: usize,
    parent_task_meta: Option<TaskMeta>,
    parent_worktree: Option<WorktreeMeta>,
    parent_chat_id: String,
    parent_root_chat_id: String,
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "topic": {
                "type": "string",
                "description": "Overarching investigation topic shared by all subagents."
            },
            "questions": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Specific investigation questions. One read-only subchat is launched per question."
            },
            "max_parallel": {
                "type": "integer",
                "minimum": 1,
                "maximum": 5,
                "default": 3,
                "description": "Maximum number of investigation subchats to run concurrently. Hard-capped at 5."
            },
            "per_agent_budget": {
                "type": "integer",
                "minimum": 1,
                "maximum": 50,
                "default": 15,
                "description": "Maximum subchat steps per investigation agent."
            }
        },
        "required": ["topic", "questions"],
        "additionalProperties": false
    })
}

fn parse_required_string(args: &HashMap<String, Value>, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("argument `{key}` is missing or not a non-empty string"))
}

fn parse_questions(args: &HashMap<String, Value>) -> Result<Vec<String>, String> {
    let questions = args
        .get("questions")
        .and_then(Value::as_array)
        .ok_or_else(|| "argument `questions` must be an array of strings".to_string())?;
    if questions.is_empty() {
        return Err("argument `questions` must contain at least one question".to_string());
    }

    let mut parsed = Vec::with_capacity(questions.len());
    for (idx, question) in questions.iter().enumerate() {
        let question = question
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("argument `questions[{idx}]` must be a non-empty string"))?;
        parsed.push(question.to_string());
    }
    Ok(parsed)
}

fn parse_optional_usize(
    args: &HashMap<String, Value>,
    key: &str,
    default: usize,
) -> Result<usize, String> {
    let Some(value) = args.get(key) else {
        return Ok(default);
    };
    if value.is_null() {
        return Ok(default);
    }
    if let Some(n) = value.as_u64() {
        return usize::try_from(n).map_err(|_| format!("argument `{key}` is too large"));
    }
    if let Some(text) = value.as_str() {
        return text
            .trim()
            .parse::<usize>()
            .map_err(|_| format!("argument `{key}` must be a positive integer"));
    }
    Err(format!("argument `{key}` must be a positive integer"))
}

fn parse_swarm_input(args: &HashMap<String, Value>) -> Result<SwarmInput, String> {
    let topic = parse_required_string(args, "topic")?;
    let questions = parse_questions(args)?;
    let max_parallel = parse_optional_usize(args, "max_parallel", DEFAULT_MAX_PARALLEL)?
        .max(1)
        .min(HARD_MAX_PARALLEL);
    let per_agent_budget =
        parse_optional_usize(args, "per_agent_budget", DEFAULT_PER_AGENT_BUDGET)?
            .max(1)
            .min(MAX_PER_AGENT_BUDGET);

    Ok(SwarmInput {
        topic,
        questions,
        max_parallel,
        per_agent_budget,
    })
}

fn build_investigation_messages(topic: &str, question: &str) -> Vec<ChatMessage> {
    vec![
        ChatMessage::new("system".to_string(), INVESTIGATION_SYSTEM_PROMPT.to_string()),
        ChatMessage::new(
            "user".to_string(),
            format!(
                "Topic: {topic}\n\nQuestion: {question}\n\nProvide findings as: ## Files / ## Key findings / ## Open questions"
            ),
        ),
    ]
}

fn read_only_tools() -> Vec<String> {
    READ_ONLY_TOOLS
        .iter()
        .map(|tool| tool.to_string())
        .collect()
}

fn safe_prefix(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn truncate_with_note(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    if max_bytes == 0 {
        return String::new();
    }
    if max_bytes <= TRUNCATION_NOTE.len() {
        return safe_prefix(TRUNCATION_NOTE, max_bytes).to_string();
    }
    format!(
        "{}{}",
        safe_prefix(text, max_bytes - TRUNCATION_NOTE.len()),
        TRUNCATION_NOTE
    )
}

fn finding_markdown(finding: &SwarmFinding) -> String {
    match finding {
        SwarmFinding::Success(text) => text.clone(),
        SwarmFinding::Failed(error) => format!("⚠️ Investigation failed: {error}"),
        SwarmFinding::TimedOut(duration) => {
            format!("⚠️ Investigation timed out after {}s.", duration.as_secs())
        }
    }
}

fn aggregate_swarm_report(topic: &str, results: &[SwarmQuestionResult]) -> String {
    let mut sorted = results.to_vec();
    sorted.sort_by_key(|result| result.index);

    let header = format!("# Swarm Investigation: {topic}\n\n");
    if sorted.is_empty() {
        return header;
    }

    let headings_len: usize = sorted
        .iter()
        .map(|result| format!("## Q{}: {}\n", result.index + 1, result.question).len() + 3)
        .sum();
    let available_for_findings = MAX_TOTAL_RESULT_BYTES.saturating_sub(header.len() + headings_len);
    let per_finding_limit = available_for_findings / sorted.len().max(1);

    let mut report = header;
    for result in sorted {
        let finding = truncate_with_note(&finding_markdown(&result.finding), per_finding_limit);
        report.push_str(&format!(
            "## Q{}: {}\n{}\n\n",
            result.index + 1,
            result.question,
            finding
        ));
    }

    truncate_with_note(&report, MAX_TOTAL_RESULT_BYTES)
}

async fn run_swarm_questions<F, Fut>(
    input: SwarmInput,
    timeout: Option<Duration>,
    runner: F,
) -> Vec<SwarmQuestionResult>
where
    F: Fn(SwarmJob) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<String, String>> + Send + 'static,
{
    let semaphore = Arc::new(Semaphore::new(input.max_parallel));
    let mut jobs = JoinSet::new();

    for (index, question) in input.questions.iter().enumerate() {
        let semaphore = semaphore.clone();
        let runner = runner.clone();
        let job = SwarmJob {
            index,
            topic: input.topic.clone(),
            question: question.clone(),
            per_agent_budget: input.per_agent_budget,
        };

        jobs.spawn(async move {
            let Ok(_permit) = semaphore.acquire_owned().await else {
                return SwarmQuestionResult {
                    index: job.index,
                    question: job.question,
                    finding: SwarmFinding::Failed(
                        "parallel execution semaphore closed".to_string(),
                    ),
                };
            };
            let finding = if let Some(timeout) = timeout {
                match tokio::time::timeout(timeout, runner(job.clone())).await {
                    Ok(Ok(text)) => SwarmFinding::Success(text),
                    Ok(Err(error)) => SwarmFinding::Failed(error),
                    Err(_) => SwarmFinding::TimedOut(timeout),
                }
            } else {
                match runner(job.clone()).await {
                    Ok(text) => SwarmFinding::Success(text),
                    Err(error) => SwarmFinding::Failed(error),
                }
            };
            SwarmQuestionResult {
                index: job.index,
                question: job.question,
                finding,
            }
        });
    }

    let mut results = Vec::new();
    while let Some(result) = jobs.join_next().await {
        match result {
            Ok(result) => results.push(result),
            Err(error) => results.push(SwarmQuestionResult {
                index: usize::MAX,
                question: "unknown".to_string(),
                finding: SwarmFinding::Failed(format!(
                    "investigation task failed to join: {error}"
                )),
            }),
        }
    }
    results
}

async fn run_real_investigation_subchat(
    ctx: RealSubchatRunnerContext,
    job: SwarmJob,
) -> Result<String, String> {
    let config = resolve_subchat_config_with_parent(
        ctx.gcx.clone(),
        SUBCHAT_TOOL_NAME,
        false,
        None,
        Some(format!("Swarm Investigation Q{}", job.index + 1)),
        Some(ctx.parent_chat_id),
        Some(TOOL_NAME.to_string()),
        Some(ctx.parent_root_chat_id),
        Some(read_only_tools()),
        job.per_agent_budget,
        false,
        None,
        "agent".to_string(),
        ctx.parent_task_meta,
        ctx.parent_worktree,
        Some(ctx.parent_tool_call_id),
        Some(ctx.parent_subchat_tx),
        Some(ctx.parent_abort_flag),
        ctx.parent_depth + 1,
    )
    .await?;

    let messages = build_investigation_messages(&job.topic, &job.question);
    let result = run_subchat(ctx.gcx, messages, config).await?;
    let reply = result
        .messages
        .iter()
        .rev()
        .find(|message| message.role == "assistant")
        .ok_or_else(|| "No response from investigation subchat".to_string())?;

    Ok(reply.content.content_text_only())
}

#[async_trait]
impl Tool for ToolSwarmInvestigate {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: TOOL_NAME.to_string(),
            display_name: "Swarm Investigation".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Spawn parallel read-only investigation subchats for multiple questions and aggregate their findings into one structured report.".to_string(),
            input_schema: input_schema(),
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
        let input = parse_swarm_input(args)?;
        let ctx = {
            let ccx_lock = ccx.lock().await;
            RealSubchatRunnerContext {
                gcx: ccx_lock.app.gcx.clone(),
                parent_subchat_tx: ccx_lock.subchat_tx.clone(),
                parent_tool_call_id: tool_call_id.clone(),
                parent_abort_flag: ccx_lock.abort_flag.clone(),
                parent_depth: ccx_lock.subchat_depth,
                parent_task_meta: ccx_lock.task_meta.clone(),
                parent_worktree: ccx_lock.execution_scope_worktree(),
                parent_chat_id: ccx_lock.chat_id.clone(),
                parent_root_chat_id: ccx_lock.root_chat_id.clone(),
            }
        };

        let runner = move |job: SwarmJob| {
            let ctx = ctx.clone();
            async move { run_real_investigation_subchat(ctx, job).await }
        };
        let results = run_swarm_questions(input.clone(), None, runner).await;
        let report = aggregate_swarm_report(&input.topic, &results);

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(report),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                usage: None,
                preserve: Some(true),
                output_filter: Some(OutputFilter::no_limits()),
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn args(pairs: Vec<(&str, Value)>) -> HashMap<String, Value> {
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect()
    }

    fn input_with_questions(count: usize, max_parallel: usize) -> SwarmInput {
        SwarmInput {
            topic: "topic".to_string(),
            questions: (0..count).map(|idx| format!("question {idx}")).collect(),
            max_parallel,
            per_agent_budget: DEFAULT_PER_AGENT_BUDGET,
        }
    }

    #[test]
    fn swarm_investigate_tool_description_correct() {
        let tool = ToolSwarmInvestigate {
            config_path: "builtin_tools.yaml".to_string(),
        };
        let desc = tool.tool_description();

        assert_eq!(desc.name, "swarm_investigate");
        assert_eq!(desc.display_name, "Swarm Investigation");
        assert!(desc.allow_parallel);
        assert!(desc
            .description
            .contains("parallel read-only investigation"));
        assert_eq!(
            desc.input_schema["properties"]["questions"]["type"],
            json!("array")
        );
        assert_eq!(
            desc.input_schema["properties"]["max_parallel"]["maximum"],
            json!(5)
        );
        assert_eq!(desc.input_schema["required"], json!(["topic", "questions"]));
    }

    #[test]
    fn swarm_investigate_input_validation_caps_max_parallel() {
        let input = parse_swarm_input(&args(vec![
            ("topic", json!("topic")),
            ("questions", json!(["q1", "q2"])),
            ("max_parallel", json!(99)),
            ("per_agent_budget", json!(500)),
        ]))
        .unwrap();

        assert_eq!(input.max_parallel, HARD_MAX_PARALLEL);
        assert_eq!(input.per_agent_budget, MAX_PER_AGENT_BUDGET);
    }

    #[test]
    fn swarm_investigate_input_validation_rejects_bad_questions() {
        let err = parse_swarm_input(&args(vec![
            ("topic", json!("topic")),
            ("questions", json!([])),
        ]))
        .unwrap_err();

        assert!(err.contains("at least one"));
    }

    #[tokio::test]
    async fn swarm_investigate_mock_subchat_three_parallel_successes() {
        let input = input_with_questions(3, 3);
        let active = Arc::new(AtomicUsize::new(0));
        let max_seen = Arc::new(AtomicUsize::new(0));
        let results = run_swarm_questions(input.clone(), Some(Duration::from_secs(1)), {
            let active = active.clone();
            let max_seen = max_seen.clone();
            move |job| {
                let active = active.clone();
                let max_seen = max_seen.clone();
                async move {
                    let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                    max_seen.fetch_max(now, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(40)).await;
                    active.fetch_sub(1, Ordering::SeqCst);
                    Ok(format!("finding {}", job.index + 1))
                }
            }
        })
        .await;

        assert_eq!(results.len(), 3);
        assert_eq!(max_seen.load(Ordering::SeqCst), 3);
        assert!(results
            .iter()
            .all(|result| matches!(result.finding, SwarmFinding::Success(_))));
        let report = aggregate_swarm_report(&input.topic, &results);
        assert!(report.contains("# Swarm Investigation: topic"));
        assert!(report.contains("## Q1: question 0"));
        assert!(report.contains("finding 3"));
    }

    #[tokio::test]
    async fn swarm_investigate_mock_subchat_failure_is_isolated() {
        let input = input_with_questions(3, 3);
        let results = run_swarm_questions(
            input.clone(),
            Some(Duration::from_secs(1)),
            move |job| async move {
                if job.index == 1 {
                    Err("boom".to_string())
                } else {
                    Ok(format!("finding {}", job.index + 1))
                }
            },
        )
        .await;

        assert_eq!(results.len(), 3);
        assert!(matches!(results[1].finding, SwarmFinding::Failed(_)));
        assert!(matches!(results[0].finding, SwarmFinding::Success(_)));
        assert!(matches!(results[2].finding, SwarmFinding::Success(_)));
        let report = aggregate_swarm_report(&input.topic, &results);
        assert!(report.contains("finding 1"));
        assert!(report.contains("Investigation failed: boom"));
        assert!(report.contains("finding 3"));
    }

    #[tokio::test]
    async fn swarm_investigate_mock_subchat_timeout_per_question() {
        let input = input_with_questions(1, 1);
        let results = run_swarm_questions(
            input.clone(),
            Some(Duration::from_millis(10)),
            move |_job| async move {
                tokio::time::sleep(Duration::from_millis(100)).await;
                Ok("late".to_string())
            },
        )
        .await;

        assert_eq!(results.len(), 1);
        assert!(matches!(results[0].finding, SwarmFinding::TimedOut(_)));
        let report = aggregate_swarm_report(&input.topic, &results);
        assert!(report.contains("timed out"));
        assert!(!report.contains("late"));
    }

    #[test]
    fn swarm_investigate_aggregate_caps_total_result() {
        let results = vec![SwarmQuestionResult {
            index: 0,
            question: "q".to_string(),
            finding: SwarmFinding::Success("x".repeat(MAX_TOTAL_RESULT_BYTES * 2)),
        }];

        let report = aggregate_swarm_report("topic", &results);

        assert!(report.len() <= MAX_TOTAL_RESULT_BYTES);
        assert!(report.contains("truncated"));
    }
}
