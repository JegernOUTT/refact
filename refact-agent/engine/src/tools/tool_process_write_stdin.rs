use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::exec::types::normalize_workspace_path;
use crate::exec::{ExecOutputChunk, ExecProcessId, ExecProcessSnapshot, ExecWriteStdinResult};
use crate::files_correction::get_active_project_path;
use crate::postprocessing::pp_command_output::{output_mini_postprocessing, OutputFilter};
use crate::tools::file_edit::auxiliary::active_execution_scope;
use crate::tools::tool_process::{process_value, read_value, status_label};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};
use crate::worktrees::scope::ExecutionScope;

const DEFAULT_YIELD_TIME_MS: u64 = 250;
const MAX_YIELD_TIME_MS: u64 = 10_000;

#[derive(serde::Serialize, schemars::JsonSchema)]
struct ProcessWriteStdinOutput {
    process_id: String,
    bytes_written: usize,
    chunks: Vec<crate::tools::tool_process::ProcessChunkOutput>,
    next_seq: u64,
}

pub struct ToolProcessWriteStdin {
    pub config_path: String,
}

struct WriteStdinArgs {
    process_id: ExecProcessId,
    chars: String,
    yield_time_ms: u64,
}

#[async_trait]
impl Tool for ToolProcessWriteStdin {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let parsed = parse_write_stdin_args(args)?;
        let (gcx, exec_registry, chat_id, execution_scope) = {
            let ccx_lock = ccx.lock().await;
            (
                ccx_lock.app.gcx.clone(),
                ccx_lock.app.runtime.exec_registry.clone(),
                ccx_lock.chat_id.clone(),
                ccx_lock.execution_scope.clone(),
            )
        };
        let workspace = current_workspace(gcx, execution_scope.as_ref()).await;
        exec_registry
            .authorize_process_access(&parsed.process_id, &chat_id, workspace.as_deref())
            .await?;
        let result = exec_registry
            .write_stdin(&parsed.process_id, &parsed.chars, parsed.yield_time_ms)
            .await?;
        let snapshot = exec_registry
            .authorize_process_access(&parsed.process_id, &chat_id, workspace.as_deref())
            .await?;
        Ok(tool_result(
            tool_call_id,
            format_write_stdin_result(&snapshot, &result),
            exec_extra(&snapshot, &result),
        ))
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "process_write_stdin".to_string(),
            display_name: "Process Write Stdin".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Write input to a running PTY-backed process's stdin (process must be started with tty=true) and wait briefly for new output. Pass empty 'chars' to poll for new output without writing.".to_string(),
            input_schema: process_write_stdin_input_schema(),
            output_schema: Some(refact_tool_api::tool_output::output_schema_for::<ProcessWriteStdinOutput>()),
            annotations: None,
        }
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }

    fn has_config_path(&self) -> Option<String> {
        Some(self.config_path.clone())
    }
}

fn process_write_stdin_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "process_id": { "type": "string" },
            "chars": { "type": "string", "default": "" },
            "yield_time_ms": { "type": "integer", "default": DEFAULT_YIELD_TIME_MS, "minimum": 0, "maximum": MAX_YIELD_TIME_MS }
        },
        "required": ["process_id"]
    })
}

fn parse_write_stdin_args(args: &HashMap<String, Value>) -> Result<WriteStdinArgs, String> {
    Ok(WriteStdinArgs {
        process_id: parse_process_id(args)?,
        chars: parse_optional_chars(args)?,
        yield_time_ms: parse_yield_time_ms(args)?,
    })
}

fn parse_process_id(args: &HashMap<String, Value>) -> Result<ExecProcessId, String> {
    match args.get("process_id") {
        Some(Value::String(process_id)) if !process_id.trim().is_empty() => {
            let process_id = process_id.trim().to_string();
            if !process_id.starts_with("exec_") {
                return Err("process_id must be a runtime-owned exec_* ID".to_string());
            }
            Ok(ExecProcessId(process_id))
        }
        Some(Value::String(_)) => Err("Argument `process_id` cannot be empty".to_string()),
        Some(value) => Err(format!("argument `process_id` is not a string: {value:?}")),
        None => Err("Missing argument `process_id`".to_string()),
    }
}

fn parse_optional_chars(args: &HashMap<String, Value>) -> Result<String, String> {
    match args.get("chars") {
        Some(Value::String(chars)) => Ok(chars.clone()),
        Some(value) => Err(format!("argument `chars` is not a string: {value:?}")),
        None => Ok(String::new()),
    }
}

fn parse_yield_time_ms(args: &HashMap<String, Value>) -> Result<u64, String> {
    let value = match args.get("yield_time_ms") {
        Some(Value::Number(number)) => number.as_u64().ok_or_else(|| {
            "argument `yield_time_ms` must be an integer from 0 to 10000".to_string()
        })?,
        Some(Value::String(value)) if value.trim().is_empty() => DEFAULT_YIELD_TIME_MS,
        Some(Value::String(value)) => value.trim().parse::<u64>().map_err(|_| {
            "argument `yield_time_ms` must be an integer from 0 to 10000".to_string()
        })?,
        Some(value) => {
            return Err(format!(
                "argument `yield_time_ms` is not a string or number: {value:?}"
            ));
        }
        None => DEFAULT_YIELD_TIME_MS,
    };
    if value > MAX_YIELD_TIME_MS {
        return Err(format!(
            "argument `yield_time_ms` exceeds maximum of {MAX_YIELD_TIME_MS}"
        ));
    }
    Ok(value)
}

async fn current_workspace(
    gcx: Arc<crate::global_context::GlobalContext>,
    execution_scope: Option<&ExecutionScope>,
) -> Option<std::path::PathBuf> {
    if let Some(scope) = active_execution_scope(execution_scope) {
        return Some(normalize_workspace_path(scope.effective_root()));
    }
    get_active_project_path(gcx)
        .await
        .map(|path| normalize_workspace_path(&path))
}

fn tool_result(
    tool_call_id: &String,
    content: String,
    extra: serde_json::Map<String, Value>,
) -> (bool, Vec<ContextEnum>) {
    let message = ChatMessage {
        role: "tool".to_string(),
        content: ChatContent::SimpleText(content),
        tool_call_id: tool_call_id.clone(),
        output_filter: Some(OutputFilter::no_limits()),
        extra,
        ..Default::default()
    };
    (false, vec![ContextEnum::ChatMessage(message)])
}

fn format_write_stdin_result(
    snapshot: &ExecProcessSnapshot,
    result: &ExecWriteStdinResult,
) -> String {
    let mut out = format!(
        "Process stdin written\nprocess_id: {}\nshort_description: {}\nstatus: {}\nmode: {}\nbytes_written: {}\nchunks_returned: {}\nsince_seq: {}\nnext_seq: {}\nlatest_seq: {}\n",
        snapshot.meta.process_id,
        snapshot.meta.short_description,
        status_label(&snapshot.status),
        snapshot.meta.mode,
        result.bytes_written,
        result.chunks_returned,
        result.read.since_seq,
        result.read.next_seq,
        result.read.latest_seq
    );
    append_section(&mut out, "combined", &collect_combined(&result.read.chunks));
    out.push_str(&format!(
        "transcript: next_seq={}, latest_seq={}, current_bytes={}, dropped_bytes={}, truncated_chunks={}, is_truncated={}\n",
        result.read.next_seq,
        result.read.latest_seq,
        result.read.current_bytes,
        result.read.dropped_bytes,
        result.read.truncated_chunks,
        result.read.is_truncated
    ));
    out
}

fn append_section(out: &mut String, title: &str, text: &str) {
    out.push_str(&format!("\n{title}:\n"));
    if text.is_empty() {
        out.push_str("<empty>\n");
    } else {
        out.push_str(&output_mini_postprocessing(
            &OutputFilter::no_limits(),
            text,
        ));
        if !out.ends_with('\n') {
            out.push('\n');
        }
    }
}

fn collect_combined(chunks: &[ExecOutputChunk]) -> String {
    chunks.iter().map(|chunk| chunk.text.as_str()).collect()
}

fn exec_extra(
    snapshot: &ExecProcessSnapshot,
    result: &ExecWriteStdinResult,
) -> serde_json::Map<String, Value> {
    let mut value = process_value(snapshot);
    value["transcript"] = read_value(&result.read);
    value["bytes_written"] = json!(result.bytes_written);
    value["chunks_returned"] = json!(result.chunks_returned);
    let mut extra = serde_json::Map::new();
    extra.insert("exec".to_string(), value);
    crate::tools::tool_process::attach_output(
        &mut extra,
        ProcessWriteStdinOutput {
            process_id: snapshot.meta.process_id.as_str().into(),
            bytes_written: result.bytes_written,
            chunks: crate::tools::tool_process::output_chunks(&result.read.chunks),
            next_seq: result.read.next_seq,
        },
    );
    extra
}
