use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::exec::types::current_timestamp_ms;
use crate::exec::{
    ExecOutputChunk, ExecProcessId, ExecProcessSnapshot, ExecReadResult, ExecRegistry, ExecStatus,
    ExecWriteStdinResult,
};
use crate::postprocessing::pp_command_output::{output_mini_postprocessing, OutputFilter};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

const DEFAULT_YIELD_TIME_MS: u64 = 250;
const MAX_YIELD_TIME_MS: u64 = 10_000;

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
        let exec_registry = {
            let ccx_lock = ccx.lock().await;
            ccx_lock.app.runtime.exec_registry.clone()
        };
        let result = exec_registry
            .write_stdin(&parsed.process_id, &parsed.chars, parsed.yield_time_ms)
            .await?;
        let snapshot = require_process(&exec_registry, &parsed.process_id).await?;
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
            output_schema: None,
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

async fn require_process(
    registry: &ExecRegistry,
    process_id: &ExecProcessId,
) -> Result<ExecProcessSnapshot, String> {
    registry
        .get(process_id)
        .await
        .ok_or_else(|| format!("process not found: {process_id}"))
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
    extra
}

fn process_value(snapshot: &ExecProcessSnapshot) -> Value {
    let cwd = snapshot
        .meta
        .cwd
        .as_ref()
        .map(|path| path.to_string_lossy().to_string());
    let workspace = snapshot
        .meta
        .owner
        .workspace
        .as_ref()
        .map(|path| path.to_string_lossy().to_string());
    let started_at_ms = snapshot
        .meta
        .started_at_ms
        .unwrap_or(snapshot.meta.created_at_ms);
    let duration = Duration::from_millis(
        snapshot
            .meta
            .ended_at_ms
            .unwrap_or_else(current_timestamp_ms)
            .saturating_sub(started_at_ms),
    );
    json!({
        "process_id": snapshot.meta.process_id.as_str(),
        "status": status_label(&snapshot.status),
        "status_detail": serde_json::to_value(&snapshot.status).unwrap_or(Value::Null),
        "mode": snapshot.meta.mode.to_string(),
        "service_name": snapshot.meta.owner.service_name.as_deref(),
        "chat_id": snapshot.meta.owner.chat_id.as_deref(),
        "tool_call_id": snapshot.meta.owner.tool_call_id.as_deref(),
        "workspace": workspace,
        "command": snapshot.meta.command.as_str(),
        "cwd": cwd,
        "short_description": snapshot.meta.short_description.as_str(),
        "created_at": snapshot.meta.created_at_ms,
        "created_at_ms": snapshot.meta.created_at_ms,
        "started_at": snapshot.meta.started_at_ms,
        "started_at_ms": snapshot.meta.started_at_ms,
        "ended_at": snapshot.meta.ended_at_ms,
        "ended_at_ms": snapshot.meta.ended_at_ms,
        "duration_ms": duration.as_millis() as u64,
        "exit_code": exit_code(&snapshot.status),
    })
}

fn read_value(read: &ExecReadResult) -> Value {
    json!({
        "process_id": read.process_id.as_str(),
        "found": read.found,
        "since_seq": read.since_seq,
        "next_seq": read.next_seq,
        "latest_seq": read.latest_seq,
        "total_bytes_appended": read.total_bytes_appended,
        "total_lines_appended": read.total_lines_appended,
        "dropped_chunks": read.dropped_chunks,
        "dropped_bytes": read.dropped_bytes,
        "truncated_chunks": read.truncated_chunks,
        "current_bytes": read.current_bytes,
        "max_bytes": read.max_bytes,
        "chunk_count": read.chunk_count,
        "is_truncated": read.is_truncated,
    })
}

fn status_label(status: &ExecStatus) -> &'static str {
    match status {
        ExecStatus::Starting => "starting",
        ExecStatus::Running => "running",
        ExecStatus::Exited { .. } => "exited",
        ExecStatus::Failed { .. } => "failed",
        ExecStatus::Killed => "killed",
        ExecStatus::TimedOut => "timed_out",
    }
}

fn exit_code(status: &ExecStatus) -> Option<i32> {
    match status {
        ExecStatus::Exited { exit_code } => *exit_code,
        ExecStatus::Starting
        | ExecStatus::Running
        | ExecStatus::Failed { .. }
        | ExecStatus::Killed
        | ExecStatus::TimedOut => None,
    }
}
