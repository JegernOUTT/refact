use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{Map, Value};

use crate::call_validation::ChatMessage;
use crate::global_context::GlobalContext;
use crate::coordination::scope::{WorkCaller, resolve_path};

pub struct LaunchScope {
    pub caller: WorkCaller,
    pub child_model: crate::caps::BaseModelRecord,
    pub limits: refact_work_core::CoordinationSettings,
}

pub(crate) fn clear_current_context(
    gcx: &Arc<GlobalContext>,
    messages: &[ChatMessage],
    model: &crate::caps::BaseModelRecord,
) -> Result<(), String> {
    struct CurrentAudit(Vec<refact_privacy::FileRecord>);
    impl refact_privacy::PrivacyAudited for CurrentAudit {
        fn privacy_records(
            &self,
        ) -> Result<Vec<(usize, refact_privacy::FileRecord)>, refact_privacy::PrivacyAuditError>
        {
            Ok(self.0.iter().cloned().map(|record| (0, record)).collect())
        }
    }
    let policy = gcx
        .privacy_policy_load
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .policy
        .clone();
    let mut records = crate::privacy::records::records_to_carry(messages)
        .map_err(|_| "launch provenance unavailable")?;
    let paths = records
        .iter()
        .map(|record| std::path::PathBuf::from(&record.path))
        .collect::<Vec<_>>();
    records.extend(
        crate::privacy::records::declared_file_records(gcx, paths)
            .map_err(|_| "launch source policy unavailable")?,
    );
    crate::privacy::destinations::clear_for_model(gcx, CurrentAudit(records), model)
        .map_err(|_| "launch context is no longer permitted")?;
    if !Arc::ptr_eq(
        &policy,
        &gcx.privacy_policy_load
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .policy,
    ) {
        return Err("launch policy changed during clearance".into());
    }
    Ok(())
}

impl LaunchScope {
    pub fn clear(&self, gcx: &Arc<GlobalContext>, messages: &[ChatMessage]) -> Result<(), String> {
        let parent = self
            .caller
            .model
            .as_ref()
            .ok_or("launch destination unavailable")?;
        for model in [parent, &self.child_model] {
            clear_current_context(gcx, messages, model)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HistoryForkMode {
    Fresh,
    ForkCompletedPrefix,
}

#[derive(Clone, Debug)]
pub enum LaunchContextEntry {
    RawText {
        label: Option<String>,
        text: String,
    },
    MessageRange {
        start_message_id: String,
        end_message_id: Option<String>,
    },
    FileRange {
        path: String,
        line1: Option<usize>,
        line2: Option<usize>,
    },
    TaskDoc {
        task_id: String,
        slug: Option<String>,
    },
    MemoryRef {
        query: String,
    },
}

#[derive(Clone, Debug, Default)]
pub struct LaunchSpec {
    pub custom_instructions: Option<String>,
    pub profile_augmentation: Option<String>,
    pub context_entries: Vec<LaunchContextEntry>,
    pub history_mode: Option<HistoryForkMode>,
    pub token_budget: Option<usize>,
    pub include_parent_goal: bool,
    pub include_parent_plan: bool,
    pub deliverable: Option<refact_chat_api::DeliverableContract>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct SkippedContextEntry {
    pub description: String,
    pub reason: String,
}

#[derive(Clone, Debug, Default)]
pub struct ResolvedLaunchContext {
    pub prepended_messages: Vec<ChatMessage>,
    pub skipped: Vec<SkippedContextEntry>,
    pub excluded: Vec<SkippedContextEntry>,
    pub estimated_tokens: usize,
    pub serialized_bytes: usize,
}

#[cfg(test)]
pub fn parse_launch_spec(args: &HashMap<String, Value>) -> Result<Option<LaunchSpec>, String> {
    parse_launch_spec_with_limits(args, &crate::file_activity::settings())
}

pub fn parse_launch_spec_with_limits(
    args: &HashMap<String, Value>,
    limits: &refact_work_core::CoordinationSettings,
) -> Result<Option<LaunchSpec>, String> {
    let Some(value) = args.get("launch_spec") else {
        return Ok(None);
    };
    match value {
        Value::Null => Ok(None),
        Value::Object(object) => Ok(Some(parse_launch_spec_object(object, limits)?)),
        other => Err(format!(
            "argument `launch_spec` must be an object: {other:?}"
        )),
    }
}

fn parse_launch_spec_object(
    object: &Map<String, Value>,
    limits: &refact_work_core::CoordinationSettings,
) -> Result<LaunchSpec, String> {
    if crate::coordination::bounds::serialized_size(object, limits.launch_context_bytes).is_none() {
        return Err("launch specification exceeds configured byte bound".into());
    }
    if object
        .get("context")
        .and_then(Value::as_array)
        .is_some_and(|entries| entries.len() > limits.launch_context_entries)
    {
        return Err("launch specification exceeds configured entry bound".into());
    }
    for key in object.keys() {
        if !matches!(
            key.as_str(),
            "custom_instructions"
                | "profile_augmentation"
                | "history_mode"
                | "token_budget"
                | "context"
                | "include_parent_goal"
                | "deliverable"
        ) {
            return Err(format!("unsupported launch_spec field: {key}"));
        }
    }
    let flag = |name: &str| -> Result<bool, String> {
        match object.get(name) {
            None | Some(Value::Null) => Ok(false),
            Some(Value::Bool(value)) => Ok(*value),
            other => Err(format!("launch_spec.{name} must be a boolean: {other:?}")),
        }
    };
    let include_parent_goal = flag("include_parent_goal")?;
    let include_parent_plan = false;
    let deliverable = match object.get("deliverable") {
        None | Some(Value::Null) => None,
        Some(value @ Value::Object(_)) => {
            let contract: refact_chat_api::DeliverableContract =
                serde_json::from_value(value.clone())
                    .map_err(|error| format!("launch_spec.deliverable is invalid: {error}"))?;
            if value.as_object().is_some_and(|map| {
                map.keys().any(|key| {
                    !matches!(
                        key.as_str(),
                        "required_sections" | "output_schema" | "evidence_required"
                    )
                })
            }) {
                return Err("unsupported field in launch_spec.deliverable".into());
            }
            contract.validate(limits.launch_context_entries, limits.launch_context_bytes)?;
            Some(contract).filter(|contract| !contract.is_empty())
        }
        other => {
            return Err(format!(
                "launch_spec.deliverable must be an object: {other:?}"
            ))
        }
    };
    let custom_instructions = optional_nonempty_string(object, "custom_instructions")?;
    let profile_augmentation = optional_nonempty_string(object, "profile_augmentation")?;
    let history_mode = match object.get("history_mode") {
        None | Some(Value::Null) => None,
        Some(Value::String(mode)) => Some(match mode.as_str() {
            "fresh" => HistoryForkMode::Fresh,
            "fork_completed_prefix" => HistoryForkMode::ForkCompletedPrefix,
            other => {
                return Err(format!(
                    "launch_spec.history_mode must be `fresh` or `fork_completed_prefix`, got `{other}`"
                ))
            }
        }),
        other => {
            return Err(format!(
                "launch_spec.history_mode must be a string: {other:?}"
            ))
        }
    };
    let token_budget = match object.get("token_budget") {
        None | Some(Value::Null) => None,
        Some(Value::Number(number)) => Some(
            number
                .as_u64()
                .and_then(|value| usize::try_from(value).ok())
                .filter(|value| *value > 0)
                .ok_or_else(|| "launch_spec.token_budget must be a positive integer".to_string())?,
        ),
        other => {
            return Err(format!(
                "launch_spec.token_budget must be a positive integer: {other:?}"
            ))
        }
    };
    if token_budget.is_some_and(|budget| budget > limits.launch_context_tokens) {
        return Err("launch_spec.token_budget exceeds configured token bound".into());
    }
    let context_entries = match object.get("context") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(entries)) => entries
            .iter()
            .enumerate()
            .map(|(idx, entry)| parse_context_entry(idx, entry))
            .collect::<Result<Vec<_>, _>>()?,
        other => return Err(format!("launch_spec.context must be an array: {other:?}")),
    };
    Ok(LaunchSpec {
        custom_instructions,
        profile_augmentation,
        context_entries,
        history_mode,
        token_budget,
        include_parent_goal,
        include_parent_plan,
        deliverable,
    })
}

fn optional_nonempty_string(
    object: &Map<String, Value>,
    name: &str,
) -> Result<Option<String>, String> {
    match object.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.trim().is_empty() => {
            Ok(Some(value.trim().to_string()))
        }
        Some(Value::String(_)) => Ok(None),
        other => Err(format!("launch_spec.{name} must be a string: {other:?}")),
    }
}

fn parse_context_entry(idx: usize, entry: &Value) -> Result<LaunchContextEntry, String> {
    let Value::Object(object) = entry else {
        return Err(format!("launch_spec.context[{idx}] must be an object"));
    };
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("launch_spec.context[{idx}].type is required"))?;
    let allowed: &[&str] = match kind {
        "raw_text" => &["type", "label", "text"],
        "message_range" => &["type", "start_message_id", "end_message_id"],
        "file_range" => &["type", "path", "line1", "line2"],
        "task_doc" => &["type", "task_id", "slug"],
        "memory_ref" => &["type", "query"],
        _ => &[],
    };
    if !allowed.is_empty() && object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("unsupported field in launch_spec.context[{idx}]"));
    }
    match kind {
        "raw_text" => {
            let text = object
                .get("text")
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|text| !text.trim().is_empty())
                .ok_or_else(|| format!("launch_spec.context[{idx}].text must be a non-empty string"))?;
            let label = object
                .get("label")
                .and_then(Value::as_str)
                .map(str::to_string);
            Ok(LaunchContextEntry::RawText { label, text })
        }
        "message_range" => {
            let start_message_id = object
                .get("start_message_id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|id| !id.trim().is_empty())
                .ok_or_else(|| {
                    format!("launch_spec.context[{idx}].start_message_id must be a non-empty string")
                })?;
            let end_message_id = object
                .get("end_message_id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|id| !id.trim().is_empty());
            Ok(LaunchContextEntry::MessageRange {
                start_message_id,
                end_message_id,
            })
        }
        "file_range" => {
            let path = object
                .get("path")
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|path| !path.trim().is_empty())
                .ok_or_else(|| format!("launch_spec.context[{idx}].path must be a non-empty string"))?;
            let parse_line = |name: &str| -> Result<Option<usize>, String> {
                object.get(name).filter(|value| !value.is_null()).map(|value| {
                    value.as_u64().and_then(|value| usize::try_from(value).ok()).filter(|value| *value > 0)
                        .ok_or_else(|| format!("launch_spec.context[{idx}].{name} must be a positive integer"))
                }).transpose()
            };
            let line1 = parse_line("line1")?;
            let line2 = parse_line("line2")?;
            if line1.zip(line2).is_some_and(|(first, last)| first > last) {
                return Err("file range end precedes start".into());
            }
            Ok(LaunchContextEntry::FileRange { path, line1, line2 })
        }
        "task_doc" => {
            let task_id = object
                .get("task_id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|id| !id.trim().is_empty())
                .ok_or_else(|| format!("launch_spec.context[{idx}].task_id must be a non-empty string"))?;
            let slug = optional_nonempty_string(object, "slug")?;
            Ok(LaunchContextEntry::TaskDoc { task_id, slug })
        }
        "memory_ref" => {
            let query = object
                .get("query")
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|query| !query.trim().is_empty())
                .ok_or_else(|| format!("launch_spec.context[{idx}].query must be a non-empty string"))?;
            Ok(LaunchContextEntry::MemoryRef { query })
        }
        other => Err(format!(
            "launch_spec.context[{idx}].type `{other}` is not supported; expected one of: raw_text, message_range, file_range, task_doc, memory_ref"
        )),
    }
}

fn message_group_bounds(messages: &[ChatMessage], start_idx: usize) -> usize {
    let mut end = start_idx;
    if messages[start_idx].role == "assistant" && messages[start_idx].tool_calls.is_some() {
        let expected_ids: std::collections::HashSet<&str> = messages[start_idx]
            .tool_calls
            .as_ref()
            .map(|calls| calls.iter().map(|call| call.id.as_str()).collect())
            .unwrap_or_default();
        let mut remaining = expected_ids;
        let mut cursor = start_idx + 1;
        while cursor < messages.len() && !remaining.is_empty() {
            let message = &messages[cursor];
            if is_tool_result(message) && remaining.contains(message.tool_call_id.as_str()) {
                remaining.remove(message.tool_call_id.as_str());
                end = cursor;
                cursor += 1;
            } else if !matches!(message.role.as_str(), "assistant" | "user" | "system")
                && message.tool_call_id.is_empty()
            {
                cursor += 1;
            } else {
                break;
            }
        }
    }
    end
}

fn is_tool_result(message: &ChatMessage) -> bool {
    !message.tool_call_id.is_empty()
        && matches!(
            message.role.as_str(),
            "tool" | "context_file" | "diff" | "plain_text"
        )
}

fn find_message_index(messages: &[ChatMessage], message_id: &str) -> Option<usize> {
    messages
        .iter()
        .position(|message| message.message_id == message_id)
}

fn extract_message_range(
    messages: &[ChatMessage],
    start_message_id: &str,
    end_message_id: Option<&str>,
    limits: &refact_work_core::CoordinationSettings,
) -> Result<Vec<ChatMessage>, String> {
    let start_idx = find_message_index(messages, start_message_id)
        .ok_or_else(|| format!("message_id `{start_message_id}` not found in this chat"))?;
    let mut end_idx = match end_message_id {
        Some(end_id) => find_message_index(messages, end_id)
            .ok_or_else(|| format!("message_id `{end_id}` not found in this chat"))?,
        None => start_idx,
    };
    if end_idx < start_idx {
        return Err(format!(
            "message_range end_message_id `{}` occurs before start_message_id `{}`",
            end_message_id.unwrap_or_default(),
            start_message_id
        ));
    }
    end_idx = end_idx.max(message_group_bounds(messages, end_idx));
    let selected = &messages[start_idx..=end_idx];
    if selected.len() > limits.launch_context_messages {
        return Err("selected context exceeds configured message cap".into());
    }
    if crate::coordination::bounds::serialized_size(&selected, limits.launch_context_bytes)
        .is_none()
    {
        return Err("selected context exceeds configured byte bound".into());
    }
    validate_complete_groups(selected)?;
    Ok(selected.to_vec())
}

fn validate_complete_groups(selected: &[ChatMessage]) -> Result<(), String> {
    let mut calls = std::collections::HashSet::new();
    let mut pending = std::collections::HashSet::new();
    let mut message_ids = std::collections::HashSet::new();
    for message in selected {
        if !message.message_id.is_empty() && !message_ids.insert(message.message_id.as_str()) {
            return Err("selected context contains duplicate message IDs".into());
        }
        if message.role == "system" {
            return Err(
                "selected range contains parent system policy; use custom_instructions instead"
                    .into(),
            );
        }
        if !pending.is_empty() && matches!(message.role.as_str(), "assistant" | "user") {
            return Err("selected context must contain complete tool-call/result groups".into());
        }
        if let Some(tool_calls) = &message.tool_calls {
            for call in tool_calls {
                if call.id.is_empty() || !calls.insert(call.id.as_str()) {
                    return Err("selected context contains duplicate or empty tool-call IDs".into());
                }
                pending.insert(call.id.as_str());
            }
        }
        if !message.tool_call_id.is_empty() {
            if !is_tool_result(message) || !calls.contains(message.tool_call_id.as_str()) {
                return Err("selected context contains an unmatched tool result".into());
            }
            pending.remove(message.tool_call_id.as_str());
        }
    }
    if !pending.is_empty() {
        return Err("selected context must contain complete tool-call/result groups".into());
    }
    Ok(())
}

fn completed_prefix(messages: &[ChatMessage]) -> Result<usize, String> {
    let mut pending = std::collections::HashSet::new();
    let mut calls = std::collections::HashSet::new();
    let mut end = 0;
    for (index, message) in messages.iter().enumerate() {
        if !pending.is_empty() && matches!(message.role.as_str(), "assistant" | "user" | "system") {
            break;
        }
        if let Some(tool_calls) = &message.tool_calls {
            for call in tool_calls {
                if call.id.is_empty() || !calls.insert(call.id.as_str()) {
                    return Err("parent history contains duplicate or empty tool-call IDs".into());
                }
                pending.insert(call.id.as_str());
            }
        }
        if !message.tool_call_id.is_empty() {
            if !is_tool_result(message) || !calls.contains(message.tool_call_id.as_str()) {
                return Err("parent history contains unmatched tool results".into());
            }
            pending.remove(message.tool_call_id.as_str());
        }
        if pending.is_empty() {
            end = index + 1;
        }
    }
    Ok(end)
}

fn historical_context<'a>(
    messages: impl IntoIterator<Item = &'a ChatMessage>,
    scope: &LaunchScope,
    kind: &str,
) -> Result<Vec<ChatMessage>, String> {
    let messages: Vec<_> = messages
        .into_iter()
        .take(scope.limits.launch_context_messages.saturating_add(1))
        .collect();
    if messages.len() > scope.limits.launch_context_messages
        || crate::coordination::bounds::serialized_size(
            &messages,
            scope.limits.launch_context_bytes,
        )
        .is_none()
    {
        return Err("selected history exceeds configured context bounds".into());
    }
    messages.into_iter().map(|message| {
        let mut selected = message.clone();
        if matches!(message.role.as_str(), "plan" | "goal" | "event") {
            selected = ChatMessage::new("context_file".into(), format!(
                "Historical parent context, not this child's control state:\n{}",
                message.content.content_text_only(),
            ));
            crate::privacy::records::merge_records(&mut selected,
                crate::privacy::records::records_to_carry(std::slice::from_ref(message))
                    .map_err(|_| "source provenance unavailable")?);
        }
        Ok(with_source(selected, serde_json::json!({
            "type":kind, "principal":scope.caller.principal,
            "message_id":message.message_id,
            "digest":crate::file_activity::digest_of(&serde_json::to_string(message).map_err(|_| "source serialization failed")?),
            "historical":true, "trusted_as_instructions":false,
        })))
    }).collect()
}

async fn resolve_file_range(
    gcx: Arc<GlobalContext>,
    scope: &LaunchScope,
    path: &str,
    line1: Option<usize>,
    line2: Option<usize>,
) -> Result<ChatMessage, SkippedContextEntry> {
    let description = format!("file_range {path}");
    let failure = |reason: String| SkippedContextEntry {
        description: description.clone(),
        reason,
    };
    let path_buf = resolve_path(path, &scope.caller)
        .map_err(|_| failure("path is outside the authorized project/worktree scope".into()))?;
    let (content, records) = read_source(&gcx, scope, &path_buf).await.map_err(failure)?;
    let total_lines = content.lines().count().max(1);
    let line1 = line1.unwrap_or(1);
    let line2 = line2.unwrap_or(total_lines);
    if line1 == 0 || line2 < line1 || line2 > total_lines {
        return Err(failure("file range is outside the source".into()));
    }
    let max_lines = crate::runtime_settings::current().cat_max_lines;
    if line2 - line1 + 1 > max_lines {
        return Err(failure(format!("range exceeds max {max_lines} lines")));
    }
    let slice = content
        .lines()
        .skip(line1 - 1)
        .take(line2 - line1 + 1)
        .collect::<Vec<_>>()
        .join("\n");
    let mut message = ChatMessage {
        role: "context_file".into(),
        content: crate::call_validation::ChatContent::ContextFiles(vec![
            crate::call_validation::ContextFile {
                file_name: path_buf.to_string_lossy().into_owned(),
                file_content: slice,
                line1,
                line2,
                ..Default::default()
            },
        ]),
        ..Default::default()
    };
    crate::privacy::records::merge_records(&mut message, records);
    Ok(with_source(
        message,
        serde_json::json!({
            "type":"file_range", "path":path_buf, "line1":line1, "line2":line2,
            "digest":crate::file_activity::digest_of(&content),
        }),
    ))
}

async fn read_source(
    gcx: &Arc<GlobalContext>,
    scope: &LaunchScope,
    path: &std::path::Path,
) -> Result<(String, Vec<refact_privacy::FileRecord>), String> {
    use tokio::io::AsyncReadExt;
    let path = resolve_path(
        path.to_str().ok_or("source path unavailable")?,
        &scope.caller,
    )
    .map_err(|_| "source is outside the authorized project/worktree scope")?;
    let records = crate::privacy::records::declared_file_records(gcx, [path.clone()])
        .map_err(|_| "source privacy unavailable")?;
    let mut audit = ChatMessage::default();
    crate::privacy::records::merge_records(&mut audit, records.clone());
    scope.clear(gcx, &[audit])?;
    let limit = scope.limits.launch_source_bytes;
    let before = tokio::fs::symlink_metadata(&path)
        .await
        .map_err(|_| "source unavailable")?;
    if !before.is_file() || before.file_type().is_symlink() {
        return Err("source must be a regular file".into());
    }
    let mut options = tokio::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    let file = options
        .open(&path)
        .await
        .map_err(|_| "source unavailable")?;
    let opened = file.metadata().await.map_err(|_| "source unavailable")?;
    if !same_source(&before, &opened) {
        return Err("source changed while opening; retry the launch preview".into());
    }
    let mut reader = file.take(limit.saturating_add(1) as u64);
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| "source is not readable")?;
    if bytes.len() > limit {
        return Err("source exceeds configured launch read bound".into());
    }
    let after = reader
        .get_ref()
        .metadata()
        .await
        .map_err(|_| "source unavailable")?;
    let named = tokio::fs::symlink_metadata(&path)
        .await
        .map_err(|_| "source unavailable")?;
    if !same_source(&opened, &after)
        || !same_source(&opened, &named)
        || resolve_path(
            path.to_str().ok_or("source path unavailable")?,
            &scope.caller,
        )
        .map_err(|_| "source scope changed")?
            != path
    {
        return Err("source changed while reading; retry the launch preview".into());
    }
    let content = String::from_utf8(bytes).map_err(|_| "source is not readable UTF-8")?;
    let mut audit = ChatMessage::default();
    crate::privacy::records::merge_records(&mut audit, records.clone());
    scope.clear(gcx, &[audit])?;
    Ok((content, records))
}

fn same_source(before: &std::fs::Metadata, after: &std::fs::Metadata) -> bool {
    if !after.is_file()
        || after.file_type().is_symlink()
        || before.len() != after.len()
        || before.modified().ok() != after.modified().ok()
    {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if before.dev() != after.dev()
            || before.ino() != after.ino()
            || before.ctime() != after.ctime()
            || before.ctime_nsec() != after.ctime_nsec()
        {
            return false;
        }
    }
    true
}

fn with_source(mut message: ChatMessage, source: Value) -> ChatMessage {
    let mut source = source;
    if let Some(source) = source.as_object_mut() {
        source.insert(
            "observed_at_ms".into(),
            serde_json::json!(chrono::Utc::now().timestamp_millis()),
        );
    }
    message
        .extra
        .insert("launch_context_source".to_string(), source);
    message
}

async fn resolve_task_doc(
    gcx: Arc<GlobalContext>,
    scope: &LaunchScope,
    task_id: &str,
    slug: Option<&str>,
) -> Result<ChatMessage, SkippedContextEntry> {
    let description = format!("task_doc {task_id}");
    let failure = |reason: String| SkippedContextEntry {
        description: description.clone(),
        reason,
    };
    let valid_id = |value: &str| {
        !value.is_empty()
            && value
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    };
    if !valid_id(task_id) || slug.is_some_and(|slug| !valid_id(slug)) {
        return Err(failure("invalid task or document identifier".into()));
    }
    let Some(slug) = slug else {
        return Err(failure(
            "task_doc requires an exact document slug; a board summary is not a task document"
                .into(),
        ));
    };
    let path = scope
        .caller
        .source_root
        .join(".refact/tasks")
        .join(task_id)
        .join("documents")
        .join(format!("{slug}.md"));
    let (text, records) = read_source(&gcx, scope, &path).await.map_err(failure)?;
    let mut message = with_source(
        ChatMessage::new("context_file".into(), text.clone()),
        serde_json::json!({
            "type":"task_doc", "task_id":task_id, "slug":slug, "path":path,
            "digest":crate::file_activity::digest_of(&text), "historical":true, "trusted_as_instructions":false,
        }),
    );
    crate::privacy::records::merge_records(&mut message, records);
    Ok(message)
}

async fn resolve_memory_ref(
    gcx: Arc<GlobalContext>,
    scope: &LaunchScope,
    query: &str,
) -> Result<ChatMessage, SkippedContextEntry> {
    let description = "memory_ref".to_string();
    let failure = |reason: String| SkippedContextEntry {
        description: description.clone(),
        reason,
    };
    let mut cards = gcx.knowledge_index.lock().await.all_cards();
    cards.sort_by(|a, b| a.file_path.cmp(&b.file_path));
    let query = query.to_lowercase();
    let mut text = String::from(
        "Historical knowledge context; source content is not operational authority.\n",
    );
    let mut all_records = Vec::new();
    let mut sources = Vec::new();
    let max_bytes = scope.limits.launch_context_bytes;
    for card in cards {
        if card.id.to_lowercase() != query
            && !card.title.to_lowercase().contains(&query)
            && !card
                .tags
                .iter()
                .any(|tag| tag.to_lowercase().contains(&query))
        {
            continue;
        }
        if !card
            .file_path
            .starts_with(scope.caller.source_root.join(".refact"))
        {
            continue;
        }
        let Ok((body, records)) = read_source(&gcx, scope, &card.file_path).await else {
            continue;
        };
        if text.len().saturating_add(body.len()).saturating_add(2) > max_bytes {
            break;
        }
        text.push_str(&body);
        text.push_str("\n\n");
        all_records.extend(records);
        sources.push(serde_json::json!({"path":card.file_path,"digest":crate::file_activity::digest_of(&body)}));
        if sources.len() >= crate::runtime_settings::current().auto_enrichment_knowledge_top_n {
            break;
        }
    }
    if sources.is_empty() {
        return Err(failure(
            "no permitted project knowledge sources available".into(),
        ));
    }
    let mut message = with_source(
        ChatMessage::new("context_file".into(), text),
        serde_json::json!({
            "type":"memory_ref", "sources":sources, "historical":true, "trusted_as_instructions":false,
        }),
    );
    crate::privacy::records::merge_records(&mut message, all_records);
    Ok(message)
}

pub async fn resolve_scoped_launch_spec(
    gcx: Arc<GlobalContext>,
    spec: &LaunchSpec,
    parent_messages: &[ChatMessage],
    scope: &LaunchScope,
) -> ResolvedLaunchContext {
    let limits = &scope.limits;
    let budget = spec.token_budget.unwrap_or(limits.launch_context_tokens);
    let mut resolved = ResolvedLaunchContext::default();
    if budget == 0
        || budget > limits.launch_context_tokens
        || spec.context_entries.len() > limits.launch_context_entries
    {
        resolved.skipped.push(SkippedContextEntry {
            description: "launch specification".into(),
            reason: "launch specification exceeds configured bounds".into(),
        });
        return resolved;
    }
    let tokenizer_key = crate::caps::strip_model_from_finetune(&scope.child_model.id);
    let tokenizer = gcx
        .tokenizer_state
        .read()
        .ok()
        .and_then(|state| state.map.get(&tokenizer_key).cloned().flatten());
    let count =
        |text: &str| crate::tokens::count_text_tokens_with_fallback(tokenizer.clone(), text);
    if spec.history_mode == Some(HistoryForkMode::ForkCompletedPrefix) {
        match completed_prefix(parent_messages) {
            Ok(end) => {
                if end < parent_messages.len() {
                    resolved.excluded.push(SkippedContextEntry {
                        description: "unfinished parent tool batch and later messages".into(),
                        reason: "fork ends at the last completed boundary; pending tools are not replayed".into(),
                    });
                }
                let selected = parent_messages[..end].iter().filter(|message| {
                    message.role != "system"
                        && !refact_chat_history::trajectory_ops::is_ui_only_message(message)
                });
                let result = historical_context(selected, scope, "completed_prefix");
                match result {
                    Ok(messages) => admit_context(
                        &gcx,
                        scope,
                        parent_messages,
                        &mut resolved,
                        budget,
                        &count,
                        "completed parent prefix".into(),
                        messages,
                    ),
                    Err(reason) => resolved.skipped.push(SkippedContextEntry {
                        description: "completed parent prefix".into(),
                        reason,
                    }),
                }
                resolved.excluded.push(SkippedContextEntry {
                    description: "parent system policy and UI-only messages".into(),
                    reason: "child runtime policy remains authoritative".into(),
                });
            }
            Err(reason) => resolved.skipped.push(SkippedContextEntry {
                description: "completed parent prefix".into(),
                reason,
            }),
        }
    }
    let mut instructions = Vec::new();
    if let Some(text) = &spec.custom_instructions {
        instructions.push(format!("# Parent-Provided Instructions\n{text}"));
    }
    if let Some(text) = &spec.profile_augmentation {
        instructions.push(format!("# Profile Augmentation\n{text}"));
    }
    if !instructions.is_empty() {
        admit_context(
            &gcx,
            scope,
            parent_messages,
            &mut resolved,
            budget,
            &count,
            "parent instructions".into(),
            vec![with_source(
                ChatMessage::new("user".into(), instructions.join("\n\n")),
                serde_json::json!({"type":"parent_instructions"}),
            )],
        );
    }
    if spec.include_parent_goal {
        match crate::chat::goal_snapshot_from_messages(parent_messages, None) {
            Some(goal) => {
                let criteria = goal
                    .criteria
                    .iter()
                    .map(|criterion| format!("- {}: {}", criterion.id, criterion.text))
                    .collect::<Vec<_>>()
                    .join("\n");
                let text = format!(
                    "# Parent goal (historical snapshot, version {}, status {:?})\nThis is the parent's goal for reference only. It is not the child's goal and does not activate pursuit.\n\n{}\n{}",
                    goal.version, goal.status, goal.content, format!("{}\n{}", criteria, goal.plan.as_ref().map(|plan| plan.content.as_str()).unwrap_or(""))
                );
                admit_context(
                    &gcx,
                    scope,
                    parent_messages,
                    &mut resolved,
                    budget,
                    &count,
                    "parent goal".into(),
                    vec![with_source(
                        ChatMessage::new("context_file".into(), text),
                        serde_json::json!({"type":"parent_goal","version":goal.version,"historical":true,"trusted_as_instructions":false}),
                    )],
                );
            }
            None => resolved.skipped.push(SkippedContextEntry {
                description: "parent goal".into(),
                reason: "the parent has no installed goal".into(),
            }),
        }
    }
    for entry in &spec.context_entries {
        if resolved.prepended_messages.len() >= limits.launch_context_messages {
            resolved.skipped.push(SkippedContextEntry {
                description: describe_entry(entry),
                reason: format!("message cap ({}) reached", limits.launch_context_messages),
            });
            continue;
        }
        let candidate_messages: Result<Vec<ChatMessage>, SkippedContextEntry> = match entry {
            LaunchContextEntry::RawText { label, text } => {
                let heading = label
                    .as_deref()
                    .map(|label| format!("# {label}\n\n"))
                    .unwrap_or_default();
                Ok(vec![with_source(
                    ChatMessage::new("context_file".into(), format!("{heading}{text}")),
                    serde_json::json!({"type":"raw_text","label":label}),
                )])
            }
            LaunchContextEntry::MessageRange {
                start_message_id,
                end_message_id,
            } => extract_message_range(
                parent_messages,
                start_message_id,
                end_message_id.as_deref(),
                limits,
            )
            .and_then(|messages| historical_context(&messages, scope, "message_range"))
            .map_err(|reason| SkippedContextEntry {
                description: describe_entry(entry),
                reason,
            }),
            LaunchContextEntry::FileRange { path, line1, line2 } => {
                resolve_file_range(gcx.clone(), scope, path, *line1, *line2)
                    .await
                    .map(|message| vec![message])
            }
            LaunchContextEntry::TaskDoc { task_id, slug } => {
                resolve_task_doc(gcx.clone(), scope, task_id, slug.as_deref())
                    .await
                    .map(|message| vec![message])
            }
            LaunchContextEntry::MemoryRef { query } => {
                resolve_memory_ref(gcx.clone(), scope, query)
                    .await
                    .map(|message| vec![message])
            }
        };
        match candidate_messages {
            Ok(messages) => admit_context(
                &gcx,
                scope,
                parent_messages,
                &mut resolved,
                budget,
                &count,
                describe_entry(entry),
                messages,
            ),
            Err(skip) => resolved.skipped.push(skip),
        }
    }
    if let Err(reason) = validate_complete_groups(&resolved.prepended_messages) {
        resolved.prepended_messages.clear();
        resolved.estimated_tokens = 0;
        resolved.serialized_bytes = 0;
        resolved.skipped.push(SkippedContextEntry {
            description: "combined launch context".into(),
            reason,
        });
    }
    resolved
}

fn model_visible_tokens(messages: &[ChatMessage], count: &impl Fn(&str) -> usize) -> usize {
    use crate::llm::adapters::render_extra::{is_context_role, render_context_message};

    messages.iter().fold(0usize, |total, message| {
        let role = if is_context_role(&message.role) {
            if message.tool_call_id.is_empty() {
                "user"
            } else {
                "tool"
            }
        } else if message.role == "diff" {
            "tool"
        } else {
            &message.role
        };
        let content =
            render_context_message(message).unwrap_or_else(|| message.content.content_text_only());
        let mut tokens = count(role).saturating_add(count(&content));
        if let crate::call_validation::ChatContent::Multimodal(elements) = &message.content {
            let images = ChatMessage {
                content: crate::call_validation::ChatContent::Multimodal(
                    elements
                        .iter()
                        .filter(|element| !element.is_text())
                        .cloned()
                        .collect(),
                ),
                ..Default::default()
            };
            tokens = tokens.saturating_add(
                refact_chat_history::trajectory_ops::approx_token_count(&[images])
                    .saturating_sub(10),
            );
        }
        if let Some(calls) = &message.tool_calls {
            for call in calls {
                tokens = tokens
                    .saturating_add(count(&call.id))
                    .saturating_add(count(&call.function.name))
                    .saturating_add(count(&call.function.arguments));
            }
        }
        if let Some(blocks) = &message.thinking_blocks {
            for block in blocks {
                tokens = tokens.saturating_add(count(&block.to_string()));
            }
        }
        total.saturating_add(tokens)
    })
}

fn admit_context(
    gcx: &Arc<GlobalContext>,
    scope: &LaunchScope,
    parent_messages: &[ChatMessage],
    resolved: &mut ResolvedLaunchContext,
    budget: usize,
    count: &impl Fn(&str) -> usize,
    description: String,
    mut messages: Vec<ChatMessage>,
) {
    let result = (|| {
        let remaining = scope
            .limits
            .launch_context_bytes
            .saturating_sub(resolved.serialized_bytes);
        if crate::coordination::bounds::serialized_size(&messages, remaining).is_none() {
            return Err("configured launch context byte bound exceeded".to_string());
        }
        if messages.iter_mut().any(|message| {
            crate::privacy::records::carry_records_into(message, parent_messages).is_err()
        }) {
            return Err("parent provenance unavailable".to_string());
        }
        scope.clear(gcx, &messages)?;
        if resolved
            .prepended_messages
            .len()
            .saturating_add(messages.len())
            > scope.limits.launch_context_messages
        {
            return Err(format!(
                "message cap ({}) exceeded",
                scope.limits.launch_context_messages
            ));
        }
        let bytes = crate::coordination::bounds::serialized_size(&messages, remaining)
            .ok_or_else(|| format!(
                "privacy provenance exceeds configured launch context byte bound ({} bytes; {remaining} remain)",
                scope.limits.launch_context_bytes,
            ))?;
        let tokens = model_visible_tokens(&messages, count);
        let remaining_tokens = budget.saturating_sub(resolved.estimated_tokens);
        if tokens > remaining_tokens {
            return Err(format!(
                "token budget exceeded; entry needs ~{tokens} tokens; {remaining_tokens} of {budget} remain"
            ));
        }
        Ok((bytes, tokens))
    })();
    match result {
        Ok((bytes, tokens)) => {
            resolved.serialized_bytes += bytes;
            resolved.estimated_tokens += tokens;
            resolved.prepended_messages.extend(messages);
        }
        Err(reason) => resolved.skipped.push(SkippedContextEntry {
            description,
            reason,
        }),
    }
}

fn describe_entry(entry: &LaunchContextEntry) -> String {
    match entry {
        LaunchContextEntry::RawText { label, .. } => {
            format!(
                "raw_text{}",
                label
                    .as_deref()
                    .map(|l| format!(" ({l})"))
                    .unwrap_or_default()
            )
        }
        LaunchContextEntry::MessageRange {
            start_message_id,
            end_message_id,
        } => format!(
            "message_range {start_message_id}..{}",
            end_message_id.as_deref().unwrap_or(start_message_id)
        ),
        LaunchContextEntry::FileRange { path, line1, line2 } => format!(
            "file_range {path}:{}-{}",
            line1.unwrap_or(1),
            line2
                .map(|v| v.to_string())
                .unwrap_or_else(|| "end".to_string())
        ),
        LaunchContextEntry::TaskDoc { task_id, .. } => format!("task_doc {task_id}"),
        LaunchContextEntry::MemoryRef { query } => format!("memory_ref {query}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn current_clearance_preserves_old_zone_and_applies_new_path_restrictions() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let workspace = tempfile::tempdir().unwrap();
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![workspace.path().into()];
        let path = workspace.path().join("context.txt");
        std::fs::write(&path, "accepted text").unwrap();
        let mut message = ChatMessage::new("user".into(), "accepted text".into());
        crate::privacy::records::attach_record(
            &mut message,
            crate::privacy::records::declared_file_record(&gcx, &path).unwrap(),
        );
        let model = crate::caps::BaseModelRecord {
            id: "allowed/model".into(),
            ..Default::default()
        };
        assert!(clear_current_context(&gcx, std::slice::from_ref(&message), &model).is_ok());
        gcx.privacy_policy_load.write().unwrap().policy = Arc::new(refact_privacy::PrivacyPolicy {
            blocked: vec!["context.txt".into()],
            ..Default::default()
        });
        assert!(clear_current_context(&gcx, std::slice::from_ref(&message), &model).is_err());
        gcx.privacy_policy_load.write().unwrap().policy =
            Arc::new(refact_privacy::PrivacyPolicy::default());
        crate::privacy::records::attach_record(
            &mut message,
            refact_privacy::FileRecord {
                path: path.to_string_lossy().into_owned(),
                zone: "blocked".into(),
                attribution: refact_privacy::Attribution::Declared,
            },
        );
        assert!(clear_current_context(&gcx, &[message], &model).is_err());
    }
    use serde_json::json;
    use std::path::Path;

    async fn resolve_launch_spec(
        gcx: Arc<GlobalContext>,
        spec: &LaunchSpec,
        messages: &[ChatMessage],
    ) -> ResolvedLaunchContext {
        resolve_with_limits(
            gcx,
            spec,
            messages,
            refact_work_core::CoordinationSettings::default(),
        )
        .await
    }

    async fn resolve_with_limits(
        gcx: Arc<GlobalContext>,
        spec: &LaunchSpec,
        messages: &[ChatMessage],
        limits: refact_work_core::CoordinationSettings,
    ) -> ResolvedLaunchContext {
        let fallback = tempfile::tempdir().unwrap();
        let root = gcx
            .documents_state
            .workspace_folders
            .lock()
            .unwrap()
            .first()
            .cloned()
            .unwrap_or_else(|| fallback.path().into());
        let model = crate::caps::BaseModelRecord {
            id: "test/model".into(),
            ..Default::default()
        };
        let scope = LaunchScope {
            caller: WorkCaller {
                source_root: root.clone(),
                checkout_root: root,
                principal: "test".into(),
                root_chat_id: None,
                model: Some(model.clone()),
            },
            child_model: model,
            limits,
        };
        resolve_scoped_launch_spec(gcx, spec, messages, &scope).await
    }

    fn msg(id: &str, role: &str, text: &str) -> ChatMessage {
        ChatMessage {
            message_id: id.to_string(),
            role: role.to_string(),
            content: crate::call_validation::ChatContent::SimpleText(text.to_string()),
            ..Default::default()
        }
    }

    fn call_message(id: &str, call: &str) -> ChatMessage {
        serde_json::from_value(json!({"role":"assistant","message_id":id,"content":"calling",
            "tool_calls":[{"id":call,"type":"function","function":{"name":"cat","arguments":"{}"}}]})).unwrap()
    }

    fn parent_with_privacy(root: &Path, count: usize) -> Vec<ChatMessage> {
        (0..count)
            .map(|index| {
                let mut message = msg(&format!("parent-{index}"), "user", "parent context");
                crate::privacy::records::attach_record(
                    &mut message,
                    refact_privacy::FileRecord {
                        path: root
                            .join(format!("source-{index}.rs"))
                            .to_string_lossy()
                            .into_owned(),
                        zone: "normal".into(),
                        attribution: refact_privacy::Attribution::Declared,
                    },
                );
                message
            })
            .collect()
    }

    #[tokio::test]
    async fn custom_instructions_from_a_long_parent_are_admitted() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let root = tempfile::tempdir().unwrap();
        authorize_workspace(&gcx, root.path());
        let parent = parent_with_privacy(root.path(), 2_000);
        let spec = LaunchSpec {
            custom_instructions: Some("Never modify files.".into()),
            ..Default::default()
        };
        let result = resolve_launch_spec(gcx.clone(), &spec, &parent).await;
        assert!(result.skipped.is_empty(), "{:?}", result.skipped);
        assert_eq!(result.prepended_messages.len(), 1);
        assert!(result.estimated_tokens < 100, "{}", result.estimated_tokens);
        assert_eq!(
            crate::privacy::records::records_to_carry(&result.prepended_messages).unwrap(),
            crate::privacy::records::records_to_carry(&parent).unwrap(),
        );
        let without_parent = resolve_launch_spec(gcx.clone(), &spec, &[]).await;
        assert_eq!(result.estimated_tokens, without_parent.estimated_tokens);
        assert!(result.serialized_bytes > without_parent.serialized_bytes);
        assert!(
            result.serialized_bytes
                <= refact_work_core::CoordinationSettings::default().launch_context_bytes
        );
        for spec in [
            LaunchSpec {
                profile_augmentation: Some("Be terse.".into()),
                ..Default::default()
            },
            LaunchSpec {
                context_entries: vec![LaunchContextEntry::RawText {
                    label: Some("brief".into()),
                    text: "Read only.".into(),
                }],
                ..Default::default()
            },
        ] {
            let result = resolve_launch_spec(gcx.clone(), &spec, &parent).await;
            assert!(result.skipped.is_empty(), "{:?}", result.skipped);
            assert!(result.estimated_tokens < 100);
            assert_eq!(
                crate::privacy::records::records_to_carry(&result.prepended_messages)
                    .unwrap()
                    .len(),
                2_000
            );
        }
    }

    #[tokio::test]
    async fn oversized_custom_instructions_are_refused_with_accurate_numbers() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let spec = LaunchSpec {
            custom_instructions: Some("word ".repeat(1_000)),
            ..Default::default()
        };
        let full = resolve_launch_spec(gcx.clone(), &spec, &[]).await;
        assert!(full.skipped.is_empty());
        let budget = full.estimated_tokens - 1;
        let bounded = resolve_launch_spec(
            gcx,
            &LaunchSpec {
                token_budget: Some(budget),
                ..spec
            },
            &[],
        )
        .await;
        assert!(bounded.prepended_messages.is_empty());
        assert_eq!(bounded.skipped.len(), 1);
        assert_eq!(bounded.skipped[0].description, "parent instructions");
        assert_eq!(
            bounded.skipped[0].reason,
            format!(
                "token budget exceeded; entry needs ~{} tokens; {budget} of {budget} remain",
                full.estimated_tokens,
            )
        );
    }

    #[tokio::test]
    async fn uncleared_provenance_still_blocks_the_launch() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let root = tempfile::tempdir().unwrap();
        authorize_workspace(&gcx, root.path());
        gcx.privacy_policy_load.write().unwrap().policy = Arc::new(refact_privacy::PrivacyPolicy {
            zones: vec![refact_privacy::Zone {
                name: "parent_only".into(),
                patterns: vec![],
                send_to: vec!["test".into()],
                on_shell_read: refact_privacy::ShellBehavior::Withhold,
            }],
            ..Default::default()
        });
        let mut parent = parent_with_privacy(root.path(), 1);
        crate::privacy::records::attach_record(
            &mut parent[0],
            refact_privacy::FileRecord {
                path: root
                    .path()
                    .join("private.rs")
                    .to_string_lossy()
                    .into_owned(),
                zone: "parent_only".into(),
                attribution: refact_privacy::Attribution::Declared,
            },
        );
        let model = crate::caps::BaseModelRecord {
            id: "test/parent".into(),
            ..Default::default()
        };
        let scope = LaunchScope {
            caller: WorkCaller {
                source_root: root.path().into(),
                checkout_root: root.path().into(),
                principal: "parent".into(),
                root_chat_id: None,
                model: Some(model.clone()),
            },
            child_model: crate::caps::BaseModelRecord {
                id: "other/child".into(),
                ..Default::default()
            },
            limits: Default::default(),
        };
        assert!(clear_current_context(&gcx, &parent, &model).is_ok());
        let result = resolve_scoped_launch_spec(
            gcx,
            &LaunchSpec {
                custom_instructions: Some("Read only.".into()),
                ..Default::default()
            },
            &parent,
            &scope,
        )
        .await;
        assert!(result.prepended_messages.is_empty());
        assert_eq!(result.skipped.len(), 1);
        assert_eq!(
            result.skipped[0].reason,
            "launch context is no longer permitted"
        );
    }

    #[tokio::test]
    async fn provenance_byte_overflow_is_refused_without_dropping_records() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let root = tempfile::tempdir().unwrap();
        authorize_workspace(&gcx, root.path());
        let parent = parent_with_privacy(root.path(), 2_000);
        let spec = LaunchSpec {
            custom_instructions: Some("Read only.".into()),
            ..Default::default()
        };
        let limits = refact_work_core::CoordinationSettings {
            launch_context_bytes: 4_096,
            ..Default::default()
        };
        assert!(resolve_with_limits(gcx.clone(), &spec, &[], limits.clone())
            .await
            .skipped
            .is_empty());
        let result = resolve_with_limits(gcx, &spec, &parent, limits).await;
        assert!(result.prepended_messages.is_empty());
        assert_eq!(result.serialized_bytes, 0);
        assert_eq!(result.skipped.len(), 1);
        assert!(result.skipped[0]
            .reason
            .starts_with("privacy provenance exceeds configured launch context byte bound"));
        assert_eq!(
            crate::privacy::records::records_to_carry(&parent)
                .unwrap()
                .len(),
            2_000
        );
    }

    #[tokio::test]
    async fn fresh_and_completed_prefix_are_distinct_without_copying_parent_control() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut answer = msg("tool-result", "context_file", "observed tool body");
        answer.tool_call_id = "read".into();
        let messages = vec![
            msg("policy", "system", "parent-only policy"),
            msg("user", "user", "parent question"),
            call_message("call", "read"),
            answer,
            crate::chat::internal_roles::goal(
                "agent",
                1,
                "parent goal",
                None,
                true,
                Default::default(),
            ),
            call_message("pending", "still-running"),
            msg("late", "event", "later message"),
        ];
        let fresh = resolve_launch_spec(
            gcx.clone(),
            &LaunchSpec {
                history_mode: Some(HistoryForkMode::Fresh),
                ..Default::default()
            },
            &messages,
        )
        .await;
        assert!(fresh.prepended_messages.is_empty());
        let spec = LaunchSpec {
            history_mode: Some(HistoryForkMode::ForkCompletedPrefix),
            ..Default::default()
        };
        let fork = resolve_launch_spec(gcx.clone(), &spec, &messages).await;
        assert!(fork.skipped.is_empty(), "{:?}", fork.skipped);
        assert_eq!(fork.prepended_messages.len(), 4);
        assert!(fork
            .prepended_messages
            .iter()
            .all(|message| !matches!(message.role.as_str(), "system" | "goal" | "plan" | "event")));
        assert!(fork
            .prepended_messages
            .iter()
            .all(|message| message.message_id != "pending" && message.message_id != "late"));
        assert!(fork
            .excluded
            .iter()
            .any(|entry| entry.reason.contains("pending tools are not replayed")));
        assert_eq!(
            messages[0].content.content_text_only(),
            "parent-only policy"
        );
        let limits = refact_work_core::CoordinationSettings {
            launch_context_messages: 2,
            ..Default::default()
        };
        let bounded = resolve_with_limits(gcx, &spec, &messages, limits).await;
        assert!(bounded.prepended_messages.is_empty());
        assert_eq!(bounded.skipped.len(), 1);
    }

    #[test]
    fn historical_selection_stops_at_the_configured_message_bound() {
        let message = msg("source", "user", "bounded history");
        let scope = LaunchScope {
            caller: WorkCaller {
                source_root: "/project".into(),
                checkout_root: "/project".into(),
                principal: "parent".into(),
                root_chat_id: None,
                model: None,
            },
            child_model: Default::default(),
            limits: refact_work_core::CoordinationSettings {
                launch_context_messages: 2,
                ..Default::default()
            },
        };
        let visited = std::cell::Cell::new(0);
        let source = std::iter::repeat(&message).inspect(|_| {
            visited.set(visited.get() + 1);
            assert!(visited.get() <= 3);
        });
        assert!(historical_context(source, &scope, "completed_prefix").is_err());
        assert_eq!(visited.get(), 3);
    }

    #[tokio::test]
    async fn overlapping_context_ranges_cannot_duplicate_accepted_history() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let entry = LaunchContextEntry::MessageRange {
            start_message_id: "one".into(),
            end_message_id: None,
        };
        let result = resolve_launch_spec(
            gcx,
            &LaunchSpec {
                context_entries: vec![entry.clone(), entry],
                ..Default::default()
            },
            &[msg("one", "user", "selected once")],
        )
        .await;
        assert!(result.prepended_messages.is_empty());
        assert_eq!(result.estimated_tokens, 0);
        assert!(result
            .skipped
            .iter()
            .any(|entry| entry.reason.contains("duplicate message")));
    }

    #[test]
    fn message_selection_requires_ordered_complete_tool_groups_and_unique_calls() {
        let call = call_message("call", "read");
        let mut result = msg("result", "diff", "result");
        result.tool_call_id = "read".into();
        assert!(validate_complete_groups(&[result.clone(), call.clone()]).is_err());
        assert!(validate_complete_groups(&[
            call.clone(),
            result.clone(),
            call.clone(),
            result.clone()
        ])
        .is_err());
        assert!(validate_complete_groups(&[
            call.clone(),
            msg("user", "user", "intervening"),
            result.clone()
        ])
        .is_err());
        let messages = vec![call, msg("notice", "event", "tool event"), result];
        let limits = refact_work_core::CoordinationSettings::default();
        assert_eq!(
            extract_message_range(&messages, "call", None, &limits)
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn parser_respects_configured_entries_bytes_and_narrowing_only_tokens() {
        let mut limits = refact_work_core::CoordinationSettings::default();
        let args = HashMap::from([(
            "launch_spec".into(),
            json!({
                "context":[{"type":"raw_text","text":"first"},{"type":"raw_text","text":"second"}]
            }),
        )]);
        limits.launch_context_entries = 1;
        assert!(parse_launch_spec_with_limits(&args, &limits)
            .unwrap_err()
            .contains("entry bound"));
        limits.launch_context_entries = 2;
        limits.launch_context_bytes = serde_json::to_vec(&args["launch_spec"]).unwrap().len();
        assert!(parse_launch_spec_with_limits(&args, &limits).is_ok());
        limits.launch_context_bytes -= 1;
        assert!(parse_launch_spec_with_limits(&args, &limits)
            .unwrap_err()
            .contains("byte bound"));
        limits.launch_context_bytes = 4096;
        limits.launch_context_tokens = 100;
        for value in [json!(0), json!(101), json!(-1), json!(1.5)] {
            let args = HashMap::from([("launch_spec".into(), json!({"token_budget":value}))]);
            assert!(parse_launch_spec_with_limits(&args, &limits).is_err());
        }
    }

    #[tokio::test]
    async fn context_limits_include_instructions_and_metadata() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let spec = LaunchSpec {
            custom_instructions: Some("retain exact instructions".into()),
            context_entries: vec![LaunchContextEntry::RawText {
                label: None,
                text: "context".into(),
            }],
            ..Default::default()
        };
        let full = resolve_launch_spec(gcx.clone(), &spec, &[]).await;
        assert_eq!(full.prepended_messages.len(), 2);
        assert!(full.skipped.is_empty());
        assert!(
            full.serialized_bytes >= serde_json::to_vec(&full.prepended_messages).unwrap().len()
        );
        {
            let limits = refact_work_core::CoordinationSettings {
                launch_context_messages: 1,
                ..Default::default()
            };
            let result = resolve_with_limits(gcx.clone(), &spec, &[], limits).await;
            assert_eq!(result.prepended_messages.len(), 1);
            assert_eq!(result.skipped.len(), 1);
            assert!(result.skipped[0].reason.contains("message cap"));
        }
        {
            let limits = refact_work_core::CoordinationSettings {
                launch_context_bytes: full.serialized_bytes - 1,
                ..Default::default()
            };
            let result = resolve_with_limits(gcx.clone(), &spec, &[], limits).await;
            assert_eq!(result.prepended_messages.len(), 1);
            assert!(result.skipped[0].reason.contains("byte bound"));
        }
        {
            let limits = refact_work_core::CoordinationSettings {
                launch_context_tokens: 1,
                ..Default::default()
            };
            let result = resolve_with_limits(gcx.clone(), &spec, &[], limits).await;
            assert!(result.prepended_messages.is_empty());
            assert_eq!(result.skipped.len(), 2);
            assert_eq!(result.estimated_tokens, 0);
        }
        let after = resolve_launch_spec(gcx, &spec, &[]).await;
        assert_eq!(after.serialized_bytes, full.serialized_bytes);
        assert_eq!(after.estimated_tokens, full.estimated_tokens);
    }

    #[tokio::test]
    async fn source_limit_changes_apply_to_future_reads_without_truncation() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let temp = tempfile::tempdir().unwrap();
        authorize_workspace(&gcx, temp.path());
        std::fs::write(temp.path().join("source.txt"), "éé").unwrap();
        let spec = LaunchSpec {
            context_entries: vec![LaunchContextEntry::FileRange {
                path: "source.txt".into(),
                line1: None,
                line2: None,
            }],
            ..Default::default()
        };
        {
            let limits = refact_work_core::CoordinationSettings {
                launch_source_bytes: 3,
                ..Default::default()
            };
            let result = resolve_with_limits(gcx.clone(), &spec, &[], limits).await;
            assert!(result.prepended_messages.is_empty());
            assert!(result.skipped[0].reason.contains("read bound"));
        }
        let result = resolve_launch_spec(gcx, &spec, &[]).await;
        assert!(result.skipped.is_empty());
        assert!(result.prepended_messages[0]
            .content
            .content_text_only()
            .contains("éé"));
    }

    #[tokio::test]
    async fn exact_task_document_carries_revision_and_never_substitutes_board_summary() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let root = tempfile::tempdir().unwrap();
        authorize_workspace(&gcx, root.path());
        let docs = root.path().join(".refact/tasks/task-one/documents");
        std::fs::create_dir_all(&docs).unwrap();
        let path = docs.join("accepted.md");
        std::fs::write(&path, "exact document body").unwrap();
        std::fs::write(
            docs.parent().unwrap().join("board.yaml"),
            "not the requested document",
        )
        .unwrap();
        let spec = LaunchSpec {
            context_entries: vec![LaunchContextEntry::TaskDoc {
                task_id: "task-one".into(),
                slug: Some("accepted".into()),
            }],
            ..Default::default()
        };
        let first = resolve_launch_spec(gcx.clone(), &spec, &[]).await;
        assert!(first.skipped.is_empty());
        let message = &first.prepended_messages[0];
        assert_eq!(message.content.content_text_only(), "exact document body");
        assert!(
            crate::privacy::records::records_to_carry(&first.prepended_messages)
                .unwrap()
                .iter()
                .any(|record| record.path == path.to_string_lossy())
        );
        let source = &message.extra["launch_context_source"];
        assert_eq!(source["slug"], "accepted");
        assert_eq!(source["trusted_as_instructions"], false);
        assert!(source["observed_at_ms"].as_i64().unwrap() > 0);
        std::fs::write(&path, "updated exact document").unwrap();
        let changed = resolve_launch_spec(gcx.clone(), &spec, &[]).await;
        assert_ne!(
            changed.prepended_messages[0].extra["launch_context_source"]["digest"],
            source["digest"]
        );
        assert_eq!(message.content.content_text_only(), "exact document body");
        let missing = LaunchSpec {
            context_entries: vec![LaunchContextEntry::TaskDoc {
                task_id: "task-one".into(),
                slug: None,
            }],
            ..Default::default()
        };
        let missing = resolve_launch_spec(gcx, &missing, &[]).await;
        assert!(missing.prepended_messages.is_empty());
        assert!(missing.skipped[0].reason.contains("exact document slug"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn launch_sources_reject_nonregular_files_and_foreign_aliases() {
        use std::os::unix::fs::symlink;
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let root = tempfile::tempdir().unwrap();
        let foreign = tempfile::tempdir().unwrap();
        authorize_workspace(&gcx, root.path());
        std::fs::write(foreign.path().join("secret"), "foreign body").unwrap();
        symlink(foreign.path().join("secret"), root.path().join("alias")).unwrap();
        let fifo = root.path().join("pipe");
        let fifo_c = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo_c.as_ptr(), 0o600) }, 0);
        for path in ["pipe", "alias"] {
            let spec = LaunchSpec {
                context_entries: vec![LaunchContextEntry::FileRange {
                    path: path.into(),
                    line1: None,
                    line2: None,
                }],
                ..Default::default()
            };
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                resolve_launch_spec(gcx.clone(), &spec, &[]),
            )
            .await
            .expect("source validation must not wait for a pipe writer");
            assert!(result.prepended_messages.is_empty());
            assert_eq!(result.skipped.len(), 1);
        }
    }

    #[test]
    fn parse_launch_spec_rejects_unknown_context_type() {
        let mut args = HashMap::new();
        args.insert(
            "launch_spec".to_string(),
            json!({"context": [{"type": "shell_exec"}]}),
        );
        let error = parse_launch_spec(&args).unwrap_err();
        assert!(error.contains("not supported"), "{error}");
    }

    #[test]
    fn parse_launch_spec_rejects_bad_history_mode() {
        let mut args = HashMap::new();
        args.insert(
            "launch_spec".to_string(),
            json!({"history_mode": "time_travel"}),
        );
        let error = parse_launch_spec(&args).unwrap_err();
        assert!(error.contains("fresh"), "{error}");
    }

    #[tokio::test]
    async fn resolve_message_range_includes_complete_tool_group() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let mut assistant = msg("a1", "assistant", "calling tool");
        assistant.tool_calls = Some(vec![crate::call_validation::ChatToolCall {
            id: "call-1".to_string(),
            index: None,
            function: crate::call_validation::ChatToolFunction {
                arguments: "{}".to_string(),
                name: "cat".to_string(),
            },
            tool_type: "function".to_string(),
            extra_content: None,
            started_at_ms: None,
            completed_at_ms: None,
        }]);
        let mut tool_result = msg("t1", "tool", "result");
        tool_result.tool_call_id = "call-1".to_string();
        let messages = vec![msg("u1", "user", "do it"), assistant, tool_result];
        let spec = LaunchSpec {
            context_entries: vec![LaunchContextEntry::MessageRange {
                start_message_id: "a1".to_string(),
                end_message_id: None,
            }],
            ..Default::default()
        };
        let resolved = resolve_launch_spec(gcx, &spec, &messages).await;
        assert!(resolved.skipped.is_empty());
        assert_eq!(resolved.prepended_messages.len(), 2);
        assert_eq!(resolved.prepended_messages[0].message_id, "a1");
        assert_eq!(resolved.prepended_messages[1].message_id, "t1");
    }

    /// Register `root` as the only authorized project directory so that
    /// `check_if_its_inside_a_workspace_worktree_or_config` admits paths under it.
    /// Tests that skip this step are correctly rejected by the fail-closed scope gate.
    fn authorize_workspace(gcx: &Arc<GlobalContext>, root: &Path) {
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.to_path_buf()];
    }

    #[tokio::test]
    async fn resolve_file_range_rejects_paths_outside_the_workspace() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let temp = tempfile::tempdir().unwrap();
        let outside = temp.path().join("lib.rs");
        std::fs::write(&outside, "line1\n").unwrap();
        // Deliberately do NOT authorize the temp dir as a workspace folder.
        let spec = LaunchSpec {
            context_entries: vec![LaunchContextEntry::FileRange {
                path: outside.to_string_lossy().to_string(),
                line1: None,
                line2: None,
            }],
            ..Default::default()
        };
        let resolved = resolve_launch_spec(gcx, &spec, &[]).await;
        assert!(resolved.prepended_messages.is_empty());
        assert_eq!(resolved.skipped.len(), 1);
        assert!(
            resolved.skipped[0]
                .reason
                .contains("outside the authorized"),
            "{:?}",
            resolved.skipped[0]
        );
    }

    #[tokio::test]
    async fn resolve_file_range_rejects_relative_path_without_active_workspace() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let spec = LaunchSpec {
            context_entries: vec![LaunchContextEntry::FileRange {
                path: "src/lib.rs".to_string(),
                line1: None,
                line2: None,
            }],
            ..Default::default()
        };
        let resolved = resolve_launch_spec(gcx, &spec, &[]).await;
        assert!(resolved.prepended_messages.is_empty());
        assert_eq!(resolved.skipped.len(), 1);
        assert!(
            resolved.skipped[0].reason.contains("unavailable"),
            "{:?}",
            resolved.skipped[0]
        );
    }

    #[tokio::test]
    async fn resolve_file_range_blocks_privacy_denied_paths() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let temp = tempfile::tempdir().unwrap();
        let blocked = temp.path().join("secret.env");
        std::fs::write(&blocked, "SECRET=1\n").unwrap();
        // The path is inside the authorized project scope, so the privacy policy
        // — not the scope gate — must be what rejects it.
        authorize_workspace(&gcx, temp.path());
        gcx.privacy_policy_load.write().unwrap().policy = Arc::new(refact_privacy::PrivacyPolicy {
            blocked: vec!["*.env".into()],
            ..Default::default()
        });
        let spec = LaunchSpec {
            context_entries: vec![LaunchContextEntry::FileRange {
                path: blocked.to_string_lossy().to_string(),
                line1: None,
                line2: None,
            }],
            ..Default::default()
        };
        let resolved = resolve_launch_spec(gcx, &spec, &[]).await;
        assert!(resolved.prepended_messages.is_empty());
        assert_eq!(resolved.skipped.len(), 1);
        assert!(
            resolved.skipped[0].reason.contains("permitted"),
            "{:?}",
            resolved.skipped[0]
        );
    }

    #[tokio::test]
    async fn resolve_file_range_reads_allowed_slice() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("lib.rs");
        std::fs::write(&file, "line1\nline2\nline3\n").unwrap();
        authorize_workspace(&gcx, temp.path());
        let spec = LaunchSpec {
            context_entries: vec![LaunchContextEntry::FileRange {
                path: file.to_string_lossy().to_string(),
                line1: Some(1),
                line2: Some(2),
            }],
            ..Default::default()
        };
        let resolved = resolve_launch_spec(gcx, &spec, &[]).await;
        assert!(resolved.skipped.is_empty(), "{:?}", resolved.skipped);
        assert_eq!(resolved.prepended_messages.len(), 1);
        let text = resolved.prepended_messages[0].content.content_text_only();
        assert!(text.contains("line1"));
        assert!(text.contains("line2"));
        assert!(!text.contains("line3"));
    }
}
