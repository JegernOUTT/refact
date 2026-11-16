use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use refact_python_kernel::protocol::HostResult;
use serde_json::{json, Value};

use crate::call_validation::{ChatContent, ChatMessage, ReasoningEffort};

pub const LLM_TIMEOUT: Duration = Duration::from_secs(180);

#[cfg(test)]
static TEST_TIMEOUTS: std::sync::LazyLock<Mutex<std::collections::HashMap<String, Duration>>> =
    std::sync::LazyLock::new(Default::default);

/// Overrides the whole-call timeout of `llm()` for one chat in tests.
#[cfg(test)]
pub(crate) fn set_test_timeout(chat_id: &str, timeout: Option<Duration>) {
    let mut timeouts = TEST_TIMEOUTS.lock().unwrap_or_else(|e| e.into_inner());
    match timeout {
        Some(timeout) => timeouts.insert(chat_id.to_string(), timeout),
        None => timeouts.remove(chat_id),
    };
}

fn call_timeout(_chat_id: &str) -> Duration {
    #[cfg(test)]
    if let Some(timeout) = TEST_TIMEOUTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(_chat_id)
    {
        return *timeout;
    }
    LLM_TIMEOUT
}

pub const MAX_SCHEMA_BYTES: usize = 16 * 1024;
pub const MAX_PROMPT_CHARS: usize = 400_000;
const ARGS: &[&str] = &["prompt", "system", "schema", "model", "max_tokens"];
const MODELS: &[&str] = &["light", "default", "thinking", "chat"];

/// Per-run limits and totals of the `llm()` helper.
#[derive(Default)]
pub struct LlmBudget {
    pub max_calls: usize,
    pub max_tokens: u64,
    pub calls: AtomicUsize,
    pub started: AtomicUsize,
    pub input_tokens: AtomicU64,
    pub output_tokens: AtomicU64,
    pub models: Mutex<BTreeSet<String>>,
}

impl LlmBudget {
    pub fn new(max_calls: usize, max_tokens: u64) -> Self {
        Self {
            max_calls,
            max_tokens,
            ..Default::default()
        }
    }
    /// `extra.python.llm`, present once a call was made.
    pub fn totals(&self) -> Option<Value> {
        let calls = self.calls.load(Ordering::Acquire);
        (calls > 0).then(|| {
            json!({
                "calls": calls,
                "input_tokens": self.input_tokens.load(Ordering::Acquire),
                "output_tokens": self.output_tokens.load(Ordering::Acquire),
                "models": self.models.lock().unwrap_or_else(|e| e.into_inner()).iter().collect::<Vec<_>>(),
            })
        })
    }
}

/// Privacy records the outer message would carry if the run ended now.
#[derive(Default)]
pub struct RunPrivacy {
    pub taint: Vec<Value>,
    pub reads: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LlmArgs {
    pub prompt: String,
    pub system: Option<String>,
    pub schema: Option<Value>,
    pub model: Option<String>,
    pub max_tokens: Option<usize>,
}

pub fn parse_args(kwargs: &Value, truncated: bool) -> Result<LlmArgs, String> {
    if truncated {
        return Err("llm() arguments too large".into());
    }
    let args = kwargs
        .as_object()
        .ok_or("llm() arguments must be keywords")?;
    if let Some(unknown) = args.keys().find(|k| !ARGS.contains(&k.as_str())) {
        return Err(format!(
            "llm() got an unexpected keyword argument '{unknown}'"
        ));
    }
    let text = |name: &str| -> Result<Option<String>, String> {
        match args.get(name) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(_) => Err(format!("llm() argument '{name}' must be a str")),
        }
    };
    let prompt = text("prompt")?.ok_or("llm() missing required argument: 'prompt'")?;
    if prompt.trim().is_empty() {
        return Err("llm() prompt must not be empty".into());
    }
    let system = text("system")?;
    let total = prompt.chars().count() + system.as_deref().map_or(0, |s| s.chars().count());
    if total > MAX_PROMPT_CHARS {
        return Err(format!(
            "llm() prompt and system exceed {MAX_PROMPT_CHARS} characters ({total})"
        ));
    }
    let schema = match args.get("schema") {
        None | Some(Value::Null) => None,
        Some(value @ Value::Object(_)) => {
            if contains_repr(value) {
                return Err("llm() schema must be a JSON-serializable dict".into());
            }
            let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?.len();
            if bytes > MAX_SCHEMA_BYTES {
                return Err(format!(
                    "llm() schema is {bytes} bytes; at most {MAX_SCHEMA_BYTES} are allowed"
                ));
            }
            Some(value.clone())
        }
        Some(_) => return Err("llm() schema must be a dict".into()),
    };
    let model = text("model")?;
    if let Some(model) = &model {
        if !MODELS.contains(&model.as_str()) {
            return Err(format!(
                "llm() model must be one of {} or None, got '{model}'",
                MODELS.join(", ")
            ));
        }
    }
    let max_tokens = match args.get("max_tokens") {
        None | Some(Value::Null) => None,
        Some(Value::Number(n)) if n.as_u64().is_some_and(|n| n > 0) => {
            Some(n.as_u64().unwrap() as usize)
        }
        Some(_) => return Err("llm() max_tokens must be a positive int".into()),
    };
    Ok(LlmArgs {
        prompt,
        system,
        schema,
        model,
        max_tokens,
    })
}

fn contains_repr(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.contains_key("__repr__") || map.values().any(contains_repr),
        Value::Array(items) => items.iter().any(contains_repr),
        _ => false,
    }
}

/// Removes one surrounding Markdown code fence.
pub fn strip_fence(text: &str) -> &str {
    let trimmed = text.trim();
    let Some(rest) = trimmed.strip_prefix("```") else {
        return trimmed;
    };
    let Some(body) = rest.strip_suffix("```") else {
        return trimmed;
    };
    match body.split_once('\n') {
        Some((lang, inner)) if !lang.trim().contains(' ') => inner.trim(),
        _ => body.trim(),
    }
}

/// Parses and validates a schema answer; `Err` lists the violations.
pub fn check_answer(schema: &Value, answer: &str) -> Result<Value, String> {
    let value: Value = serde_json::from_str(strip_fence(answer))
        .map_err(|e| format!("answer is not valid JSON: {e}"))?;
    refact_tool_api::schema_validate::validate(schema, &value).map_err(|errors| {
        errors
            .iter()
            .map(|e| {
                if e.path.is_empty() {
                    e.message.clone()
                } else {
                    format!("{}: {}", e.path, e.message)
                }
            })
            .collect::<Vec<_>>()
            .join("; ")
    })?;
    Ok(value)
}

pub fn schema_instruction(schema: &Value) -> String {
    format!(
        "Answer with exactly one JSON value that matches the JSON Schema below. Output only that JSON value, with no other text.\nJSON Schema:\n{}",
        serde_json::to_string(schema).unwrap_or_default()
    )
}

/// Lowest reasoning effort the model accepts, if it reasons at all.
pub fn lowest_effort(options: Option<&Vec<String>>) -> Option<ReasoningEffort> {
    let options = options.filter(|o| !o.is_empty())?;
    [
        ("none", ReasoningEffort::NoReasoning),
        ("noreasoning", ReasoningEffort::NoReasoning),
        ("minimal", ReasoningEffort::Minimal),
        ("low", ReasoningEffort::Low),
        ("medium", ReasoningEffort::Medium),
    ]
    .into_iter()
    .find(|(name, _)| options.iter().any(|o| o.eq_ignore_ascii_case(name)))
    .map(|(_, effort)| effort)
}

pub fn type_error(message: impl Into<String>) -> HostResult {
    HostResult::Err {
        exc_type: "TypeError".into(),
        message: message.into(),
    }
}

pub fn user_message(text: &str, privacy: &ChatMessage) -> ChatMessage {
    let mut message = ChatMessage {
        role: "user".into(),
        content: ChatContent::SimpleText(text.into()),
        ..Default::default()
    };
    if let Some(records) = privacy.extra.get("privacy") {
        message.extra.insert("privacy".into(), records.clone());
    }
    message
}

pub fn abort_flag() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

/// Output tokens requested when the script gives no `max_tokens`.
pub const DEFAULT_MAX_TOKENS: usize = 8192;

/// `max_tokens`, or the default, capped at the model's output limit.
pub fn output_tokens(requested: Option<usize>, model: &crate::caps::ChatModelRecord) -> usize {
    let wanted = requested.unwrap_or(DEFAULT_MAX_TOKENS);
    match model.max_output_tokens.filter(|limit| *limit > 0) {
        Some(limit) => wanted.min(limit),
        None => wanted,
    }
}

/// Usage the provider reported for the attempt in flight.
pub type UsageSink = Arc<Mutex<crate::call_validation::ChatUsage>>;

/// Takes the in-flight usage as (input, output) tokens and clears it.
fn take_partial(sink: &UsageSink) -> (u64, u64) {
    let usage = std::mem::take(&mut *sink.lock().unwrap());
    (
        (usage.prompt_tokens
            + usage.cache_read_tokens.unwrap_or(0)
            + usage.cache_creation_tokens.unwrap_or(0)) as u64,
        usage.completion_tokens as u64,
    )
}

struct AbortOnDrop(Arc<AtomicBool>);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arguments_are_checked_like_python() {
        let ok = parse_args(
            &json!({"prompt":"q","schema":{"type":"object"},"model":"light","max_tokens":5}),
            false,
        )
        .unwrap();
        assert_eq!(ok.model.as_deref(), Some("light"));
        for bad in [
            json!({}),
            json!({"prompt":"  "}),
            json!({"prompt":1}),
            json!({"prompt":"q","temperature":1}),
            json!({"prompt":"q","model":"gpt"}),
            json!({"prompt":"q","schema":[1]}),
            json!({"prompt":"q","schema":{"x":{"__repr__":"<object>"}}}),
            json!({"prompt":"q","schema":{"d":"x".repeat(MAX_SCHEMA_BYTES)}}),
            json!({"prompt":"q","max_tokens":0}),
            json!({"prompt":"x".repeat(MAX_PROMPT_CHARS),"system":"y"}),
        ] {
            assert!(parse_args(&bad, false).is_err(), "{bad}");
        }
        assert!(parse_args(&json!({"prompt":"q"}), true).is_err());
    }
    #[test]
    fn fenced_answers_are_parsed_and_validated() {
        let schema =
            json!({"type":"object","required":["n"],"properties":{"n":{"type":"integer"}}});
        assert_eq!(
            check_answer(&schema, "```json\n{\"n\": 3}\n```").unwrap(),
            json!({"n":3})
        );
        assert!(check_answer(&schema, "{\"n\": \"x\"}").is_err());
        assert!(check_answer(&schema, "not json").is_err());
        assert_eq!(
            lowest_effort(Some(&vec!["low".into(), "minimal".into(), "high".into()])),
            Some(ReasoningEffort::Minimal)
        );
        assert_eq!(lowest_effort(None), None);
    }
}

struct Answer {
    text: String,
    input_tokens: u64,
    output_tokens: u64,
}

enum Failure {
    Raise(&'static str, String, bool),
    Soft(String),
}

impl super::dispatcher::NestedDispatcher {
    pub async fn dispatch_llm(
        &self,
        seq: u32,
        kwargs: Value,
        truncated: bool,
        site: (Option<u32>, Option<u32>),
    ) -> super::dispatcher::NestedOutcome {
        use refact_chat_api::{PythonCallApproval, PythonCallRecord, PythonCallStatus};
        let c = &self.config;
        let start = std::time::Instant::now();
        let preview_args = json!({
            "prompt": kwargs.get("prompt").cloned().unwrap_or(Value::Null),
            "model": kwargs.get("model").cloned().unwrap_or(Value::Null),
            "schema": kwargs.get("schema").is_some_and(|s| !s.is_null()),
        });
        let mut record = PythonCallRecord {
            seq,
            call_id: format!("py_{}_{}", c.run_id, seq),
            tool: "llm".into(),
            args_preview: super::approvals::args_preview(&preview_args.to_string(), 1000, 200),
            status: PythonCallStatus::Running,
            summary: String::new(),
            started_at_ms: crate::agents::approval::now_ms(),
            duration_ms: None,
            approval: PythonCallApproval::NotNeeded,
            diffs: Vec::new(),
            line: site.0,
            col: site.1,
            model: None,
            tokens_in: None,
            tokens_out: None,
        };
        c.sink.call_started(&record);
        let mut log = json!({"seq":seq,"call_id":record.call_id,"tool":"llm"});
        let inflight = UsageSink::default();
        let outcome = tokio::select! {
            biased;
            _ = c.cancel.cancelled() => Err(Failure::Raise("RuntimeError", "python run cancelled".into(), true)),
            result = self.llm_inner(kwargs, truncated, &mut record, &mut log, &inflight) => result,
        };
        let (input, output) = take_partial(&inflight);
        if input + output > 0 {
            if let Some(budget) = &c.llm {
                budget.input_tokens.fetch_add(input, Ordering::AcqRel);
                budget.output_tokens.fetch_add(output, Ordering::AcqRel);
            }
            record.tokens_in = Some(record.tokens_in.unwrap_or(0) + input);
            record.tokens_out = Some(record.tokens_out.unwrap_or(0) + output);
            log["usage"] =
                json!({"input_tokens":record.tokens_in,"output_tokens":record.tokens_out});
        }
        let (result, stop_run) = match outcome {
            Ok(value) => {
                record.status = if value["ok"] == true {
                    PythonCallStatus::Ok
                } else {
                    PythonCallStatus::Failed
                };
                (HostResult::Ok(value), false)
            }
            Err(Failure::Soft(_)) => unreachable!(),
            Err(Failure::Raise(exc_type, message, stopped)) => {
                record.status = if stopped {
                    PythonCallStatus::Cancelled
                } else if exc_type == "PermissionError" {
                    PythonCallStatus::Denied
                } else {
                    PythonCallStatus::Failed
                };
                record.summary = super::envelope::preview(&message);
                log["error"] = json!({"exception":exc_type,"message":message});
                (
                    HostResult::Err {
                        exc_type: exc_type.into(),
                        message,
                    },
                    stopped,
                )
            }
        };
        record.duration_ms = Some(start.elapsed().as_millis().min(u64::MAX as u128) as u64);
        log["record"] = json!(record);
        let dir = c.run_dir.join("calls");
        if let Err(error) = async {
            tokio::fs::create_dir_all(&dir).await?;
            tokio::fs::write(
                dir.join(format!("{seq}_llm.json")),
                serde_json::to_vec(&log)?,
            )
            .await
        }
        .await
        {
            tracing::warn!(%error, "cannot persist python llm call");
        }
        c.sink.call_finished(&record);
        super::dispatcher::NestedOutcome {
            result,
            record,
            privacy: Vec::new(),
            images: Vec::new(),
            stop_run,
        }
    }

    async fn llm_inner(
        &self,
        kwargs: Value,
        truncated: bool,
        record: &mut refact_chat_api::PythonCallRecord,
        log: &mut Value,
        inflight: &UsageSink,
    ) -> Result<Value, Failure> {
        let c = &self.config;
        let budget = c.llm.clone().ok_or_else(|| {
            Failure::Raise(
                "RuntimeError",
                "llm() is switched off for this project (python_max_llm_calls is 0)".into(),
                false,
            )
        })?;
        let args =
            parse_args(&kwargs, truncated).map_err(|e| Failure::Raise("TypeError", e, false))?;
        log["prompt"] = json!(args.prompt);
        log["system"] = json!(args.system);
        log["schema"] = json!(args.schema);
        log["requested_model"] = json!(args.model);
        let used = budget.input_tokens.load(Ordering::Acquire)
            + budget.output_tokens.load(Ordering::Acquire);
        if used >= budget.max_tokens {
            return Err(Failure::Raise(
                "RuntimeError",
                format!(
                    "llm token budget exhausted ({used} of {} tokens used in this run)",
                    budget.max_tokens
                ),
                false,
            ));
        }
        if budget.started.load(Ordering::Acquire) >= budget.max_calls {
            return Err(Failure::Raise(
                "RuntimeError",
                format!(
                    "llm call budget exhausted ({} calls per run)",
                    budget.max_calls
                ),
                false,
            ));
        }
        let (model, carrier) = self.llm_model_and_clearance(args.model.as_deref()).await?;
        if budget
            .started
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < budget.max_calls).then_some(n + 1)
            })
            .is_err()
        {
            return Err(Failure::Raise(
                "RuntimeError",
                format!(
                    "llm call budget exhausted ({} calls per run)",
                    budget.max_calls
                ),
                false,
            ));
        }
        let model_id = model.base.id.clone();
        record.model = Some(model_id.clone());
        log["model"] = json!(model_id);
        let _permit = tokio::select! {
            biased;
            _ = c.cancel.cancelled() => return Err(Failure::Raise("RuntimeError", "python run cancelled".into(), true)),
            permit = self.parallel.acquire() => permit.map_err(|_| Failure::Raise("RuntimeError", "dispatcher closed".into(), true))?,
        };
        budget.calls.fetch_add(1, Ordering::AcqRel);
        budget.models.lock().unwrap().insert(model_id.clone());
        let mut system = args.system.clone();
        if let Some(schema) = &args.schema {
            let instruction = schema_instruction(schema);
            system = Some(match system {
                Some(text) if !text.trim().is_empty() => format!("{text}\n\n{instruction}"),
                _ => instruction,
            });
        }
        let effort = (args.model.is_none() || args.model.as_deref() == Some("light"))
            .then(|| lowest_effort(model.reasoning_effort_options.as_ref()))
            .flatten();
        let mut prompt = args.prompt.clone();
        let timeout = call_timeout(&c.chat_id);
        let deadline = tokio::time::Instant::now() + timeout;
        let mut tokens = (0u64, 0u64);
        let mut answers = Vec::new();
        let mut outcome: Result<(String, Option<Value>), String> = Err(String::new());
        for attempt in 0..2 {
            let answer = match self
                .llm_request(
                    record.seq,
                    &model,
                    system.as_deref(),
                    &prompt,
                    &carrier,
                    output_tokens(args.max_tokens, &model),
                    effort.clone(),
                    inflight,
                    (deadline, timeout),
                )
                .await
            {
                Ok(answer) => answer,
                Err(Failure::Soft(error)) => {
                    let (input, output) = take_partial(inflight);
                    tokens.0 += input;
                    tokens.1 += output;
                    record.tokens_in = Some(tokens.0);
                    record.tokens_out = Some(tokens.1);
                    budget.input_tokens.fetch_add(input, Ordering::AcqRel);
                    budget.output_tokens.fetch_add(output, Ordering::AcqRel);
                    outcome = Err(error);
                    break;
                }
                Err(raise) => {
                    record.tokens_in = Some(tokens.0);
                    record.tokens_out = Some(tokens.1);
                    return Err(raise);
                }
            };
            take_partial(inflight);
            tokens.0 += answer.input_tokens;
            tokens.1 += answer.output_tokens;
            // A Stop that drops this future later still finds this attempt's tokens in the record.
            record.tokens_in = Some(tokens.0);
            record.tokens_out = Some(tokens.1);
            budget
                .input_tokens
                .fetch_add(answer.input_tokens, Ordering::AcqRel);
            budget
                .output_tokens
                .fetch_add(answer.output_tokens, Ordering::AcqRel);
            answers.push(answer.text.clone());
            let Some(schema) = &args.schema else {
                outcome = Ok((answer.text, None));
                break;
            };
            // Validation can be costly for hostile schemas; keep Stop able to end the run.
            let checked = {
                let (schema, text) = (schema.clone(), answer.text.clone());
                let task = tokio::task::spawn_blocking(move || check_answer(&schema, &text));
                tokio::select! {
                    biased;
                    _ = c.cancel.cancelled() => {
                        record.tokens_in = Some(tokens.0);
                        record.tokens_out = Some(tokens.1);
                        return Err(Failure::Raise("RuntimeError", "python run cancelled".into(), true));
                    }
                    result = task => result.unwrap_or_else(|e| Err(format!("validation failed: {e}"))),
                }
            };
            match checked {
                Ok(data) => {
                    outcome = Ok((answer.text, Some(data)));
                    break;
                }
                Err(violations) if attempt == 0 => {
                    let used = budget.input_tokens.load(Ordering::Acquire)
                        + budget.output_tokens.load(Ordering::Acquire);
                    if used >= budget.max_tokens {
                        outcome = Err(format!(
                            "response did not match schema: {violations} (no retry: llm token budget exhausted)"
                        ));
                        break;
                    }
                    prompt = format!(
                        "{}\n\nYour previous answer did not match the JSON Schema:\n{}\nViolations: {violations}\nAnswer again with exactly one JSON value that matches the schema.",
                        args.prompt, answer.text
                    );
                }
                Err(violations) => {
                    outcome = Err(format!("response did not match schema: {violations}"));
                    break;
                }
            }
        }
        record.tokens_in = Some(tokens.0);
        record.tokens_out = Some(tokens.1);
        let usage = json!({"input_tokens":tokens.0,"output_tokens":tokens.1});
        log["answers"] = json!(answers);
        log["usage"] = usage.clone();
        let text = answers.last().cloned().unwrap_or_default();
        let value = match outcome {
            Ok((text, data)) => {
                record.summary = format!("{model_id} · {} in / {} out tokens", tokens.0, tokens.1);
                json!({"ok":true,"text":text,"data":data,"model":model_id,"usage":usage,"error":null})
            }
            Err(error) => {
                record.summary = super::envelope::preview(&format!("{model_id}: {error}"));
                log["error"] = json!(error);
                json!({"ok":false,"text":text,"data":null,"model":model_id,"usage":usage,"error":error})
            }
        };
        log["answer"] = json!(text);
        Ok(value)
    }

    /// Resolves the model and clears the run's privacy records for its provider.
    async fn llm_model_and_clearance(
        &self,
        requested: Option<&str>,
    ) -> Result<(Arc<crate::caps::ChatModelRecord>, ChatMessage), Failure> {
        let c = &self.config;
        let caps =
            crate::global_context::try_load_caps_quickly_if_not_present(c.app.gcx.clone(), 0)
                .await
                .map_err(|e| {
                    Failure::Raise(
                        "RuntimeError",
                        format!("llm(): model capabilities unavailable: {}", e.message),
                        false,
                    )
                })?;
        let chat_model = c.model_id.clone().unwrap_or_default();
        let slot = |name: &str| -> String {
            let id = match name {
                "light" => &caps.defaults.chat_light_model,
                "default" => &caps.defaults.chat_default_model,
                "thinking" => &caps.defaults.chat_thinking_model,
                _ => return chat_model.clone(),
            };
            if id.trim().is_empty() {
                chat_model.clone()
            } else {
                id.clone()
            }
        };
        let mut candidates = vec![slot(requested.unwrap_or("light"))];
        if requested.is_none() && candidates[0] != chat_model {
            candidates.push(chat_model.clone());
        }
        let carrier = self.llm_privacy_carrier().await?;
        let mut refusal = None;
        for id in candidates {
            let model = crate::caps::resolve_chat_model(caps.clone(), &id).map_err(|e| {
                Failure::Raise(
                    "RuntimeError",
                    format!("llm(): model `{id}` is not available: {e}"),
                    false,
                )
            })?;
            match crate::privacy::destinations::clear_for_model(
                &c.app.gcx,
                CarrierAudit(std::slice::from_ref(&carrier)),
                &model.base,
            ) {
                Ok(_) => return Ok((model, carrier)),
                Err(denied) => refusal = Some(denied.model_facing()),
            }
        }
        Err(Failure::Raise(
            "PermissionError",
            format!("llm(): {}", refusal.unwrap_or_default()),
            false,
        ))
    }

    async fn llm_privacy_carrier(&self) -> Result<ChatMessage, Failure> {
        let c = &self.config;
        let (taint, reads) = {
            let run = c.run_privacy.lock().unwrap();
            (run.taint.clone(), run.reads.clone())
        };
        let refuse = |e: String| {
            Failure::Raise(
                "PermissionError",
                format!("llm(): privacy records of this run could not be verified: {e}"),
                false,
            )
        };
        let mut carrier = ChatMessage::default();
        let (history, derived) = {
            let context = c.ccx.lock().await;
            (
                context.messages.clone(),
                context.derived_privacy_zones.clone(),
            )
        };
        // The script is written by the chat model, so it can carry anything the chat has seen.
        crate::privacy::records::carry_records_into(&mut carrier, &history)
            .map_err(|e| refuse(e.to_string()))?;
        super::run_output::merge_taint(&mut carrier, &taint).map_err(refuse)?;
        let direct = crate::privacy::records::observed_file_records_with_derived(
            &c.app.gcx,
            reads.iter().map(std::path::PathBuf::from),
            &derived,
        )
        .map_err(refuse)?;
        crate::privacy::records::merge_records(&mut carrier, direct);
        Ok(carrier)
    }

    async fn llm_request(
        &self,
        seq: u32,
        model: &crate::caps::ChatModelRecord,
        system: Option<&str>,
        prompt: &str,
        carrier: &ChatMessage,
        max_tokens: usize,
        effort: Option<ReasoningEffort>,
        inflight: &UsageSink,
        (deadline, timeout): (tokio::time::Instant, Duration),
    ) -> Result<Answer, Failure> {
        let c = &self.config;
        let abort = abort_flag();
        // Every exit, including this future being dropped on Stop, aborts the provider request.
        let _abort_on_exit = AbortOnDrop(abort.clone());
        let (root, task_meta) = {
            let context = c.ccx.lock().await;
            (
                Some(context.root_chat_id.clone())
                    .filter(|id| !id.trim().is_empty())
                    .unwrap_or_else(|| c.chat_id.clone()),
                context.task_meta.clone(),
            )
        };
        let stream_id = llm_stream_id(&c.chat_id, &c.run_id, seq);
        let n_ctx = if model.base.n_ctx > 0 {
            model.base.n_ctx
        } else {
            128_000
        };
        let ccx = Arc::new(tokio::sync::Mutex::new(
            crate::at_commands::at_commands::AtCommandsContext::new_with_abort(
                c.app.clone(),
                n_ctx,
                1,
                false,
                vec![],
                stream_id.clone(),
                Some(root),
                model.base.id.clone(),
                task_meta,
                None,
                Some(abort.clone()),
            )
            .await,
        ));
        let mut messages = Vec::new();
        if let Some(system) = system.filter(|s| !s.trim().is_empty()) {
            messages.push(ChatMessage::new("system".into(), system.into()));
        }
        messages.push(user_message(prompt, carrier));
        let call = crate::subchat::subchat_stream(
            ccx,
            &model.base.id,
            "agent",
            messages,
            vec![],
            false,
            None,
            max_tokens,
            effort,
            crate::llm::params::CacheControl::Off,
            false,
            None,
            true,
        );
        let call = crate::subchat::with_stream_usage(inflight.clone(), call);
        let result = tokio::select! {
            biased;
            _ = c.cancel.cancelled() => {
                abort.store(true, Ordering::Release);
                Err(Failure::Raise("RuntimeError", "python run cancelled".into(), true))
            }
            outcome = tokio::time::timeout_at(deadline, call) => match outcome {
                Err(_) => {
                    abort.store(true, Ordering::Release);
                    Err(Failure::Soft(format!("model call timed out after {} s", timeout.as_secs_f64())))
                }
                Ok(Err(error)) => Err(Failure::Soft(
                    refact_chat_api::diagnostics::safe_provider_error_diagnostic(&error),
                )),
                Ok(Ok(choices)) => {
                    let reply = choices.into_iter().next().and_then(|m| m.into_iter().last());
                    match reply.filter(|m| m.role == "assistant") {
                        Some(reply) => {
                            let usage = reply.usage.clone().unwrap_or_default();
                            Ok(Answer {
                                text: reply.content.content_text_only(),
                                input_tokens: (usage.prompt_tokens
                                    + usage.cache_read_tokens.unwrap_or(0)
                                    + usage.cache_creation_tokens.unwrap_or(0)) as u64,
                                output_tokens: usage.completion_tokens as u64,
                            })
                        }
                        None => Err(Failure::Soft("model returned no answer".into())),
                    }
                }
            },
        };
        crate::chat::stream_core::clear_unbound_openai_codex_websocket_session(&stream_id).await;
        result
    }
}

/// Internal stream id of one `llm()` call: no chat session or trajectory has it.
pub fn llm_stream_id(chat_id: &str, run_id: &str, seq: u32) -> String {
    format!("{chat_id}-pyllm-{run_id}-{seq}")
}

struct CarrierAudit<'a>(&'a [ChatMessage]);
impl refact_privacy::PrivacyAudited for CarrierAudit<'_> {
    fn privacy_records(
        &self,
    ) -> Result<Vec<(usize, refact_privacy::FileRecord)>, refact_privacy::PrivacyAuditError> {
        refact_privacy::records_from_messages(self.0)
    }
}
