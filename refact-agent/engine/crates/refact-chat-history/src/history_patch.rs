use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::ops::Range;

use refact_chat_api::internal_roles::{EVENT_ROLE, GOAL_ROLE, PLAN_ROLE};
use refact_chat_api::PythonEngineClass;
use refact_core::active_context::{active_context, ActiveContext, ActiveContextError, MessageOrigin};
use refact_core::chat_types::{
    delivery_id_of_message, is_never_model_visible, ChatContent, ChatMessage, ChatToolCall,
    ChatToolFunction, PendingDelivery, PushMode,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::content::validate_content_with_attachments;
use crate::linearize::{apply_summarization_linearize, reaches_wire};
use crate::trajectory_ops::{generate_valid_tool_id, COMPRESSION_REPORT_ROLE};

pub const MAX_OPS: usize = 50;
pub const MAX_TEXT_BYTES: usize = 16 * 1024;
pub const MAX_NEW_TEXT_BYTES: usize = 64 * 1024;
pub const MAX_AFFECTED_MESSAGES: usize = 2_000;
pub const MAX_RANGE_MESSAGES: usize = 1_000;
pub const MAX_CARD_BYTES: usize = 256 * 1024;
pub const MAX_HISTORY_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_CALLS_PER_INSERT: usize = 16;
pub const OLD_TEXT_BYTES: usize = 4 * 1024;
const CASCADE_TEXT_BYTES: usize = 300;
const RANGE_EDGE_BYTES: usize = 120;
const CARD_ENVELOPE_BYTES: usize = 2 * 1024;
const MAX_TOOL_NAME_CHARS: usize = 64;
const DROPPED_TOOL_KEYS: [&str; 3] = ["tool_output", "tool_enrichment", "read_stamps"];

/// One change of another chat's history. Ids and anchors refer to the history as it was read.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum PatchOp {
    Edit {
        message_id: String,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        find: Option<String>,
        #[serde(default)]
        replace: Option<String>,
    },
    Remove {
        message_id: String,
        #[serde(default)]
        through: Option<String>,
        #[serde(default)]
        with_results: bool,
    },
    Insert {
        after: String,
        role: InsertRole,
        text: String,
        #[serde(default)]
        tool_calls: Option<Vec<InsertedCall>>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsertRole {
    User,
    Assistant,
    System,
}

impl InsertRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::System => "system",
        }
    }
}

/// A closed tool call an inserted assistant message carries: the planner creates its id and its result.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InsertedCall {
    pub name: String,
    pub arguments: serde_json::Map<String, Value>,
    pub result: String,
    #[serde(default)]
    pub failed: bool,
}

/// The target thread's frozen system prompt: `Complete` when its frozen prefix holds both the
/// prompt and the canonical tools, `Partial` when it holds the prompt without the tools.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrozenSystem<'a> {
    Absent,
    Complete(&'a str),
    Partial,
}

/// Facts and helpers the engine supplies: `carry` merges this run's privacy records into a
/// message the patch creates or edits; `secret_spans` returns the byte ranges of the whole
/// matches the card's scrubber would mask in a text.
pub struct PatchEnv<'a> {
    pub frozen: FrozenSystem<'a>,
    pub stamp_source: &'a str,
    pub now_ms: u64,
    pub carry: &'a dyn Fn(&mut ChatMessage) -> Result<(), String>,
    pub secret_spans: &'a dyn Fn(&str) -> Vec<Range<usize>>,
}

/// Why a patch is not planned. Reasons name positions, roles and counts, never message text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchRefusal {
    Invalid(String),
    Refused(String),
    NeedsRebuild(String),
}

impl PatchRefusal {
    pub fn reason(&self) -> &str {
        match self {
            Self::Invalid(reason) | Self::Refused(reason) | Self::NeedsRebuild(reason) => reason,
        }
    }
}

#[derive(Debug)]
pub enum PatchOutcome {
    NothingToChange,
    Planned(PatchPlan),
}

/// `shown` lists the positions whose text the card items show, so their privacy records can
/// travel with the card.
pub struct PatchPlan {
    pub messages: Vec<ChatMessage>,
    pub frozen_system_prompt: Option<String>,
    pub items: Vec<PatchItem>,
    pub shown: Vec<usize>,
    pub class: PythonEngineClass,
    pub warning: String,
    pub summary: PatchSummary,
    pub inserted: Vec<InsertedMessage>,
    pub removed_delivery_ids: Vec<String>,
}

impl std::fmt::Debug for PatchPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PatchPlan")
            .field("class", &self.class)
            .field("summary", &self.summary)
            .field("inserted", &self.inserted)
            .finish_non_exhaustive()
    }
}

/// One approval card row: the only place where message text of the target chat appears.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PatchItem {
    pub label: String,
    pub old: Option<Value>,
    pub new: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InsertedMessage {
    pub op: usize,
    pub message_id: String,
    pub tool_call_ids: Vec<String>,
    pub result_ids: Vec<String>,
}

/// Positions refer to the history as it was read; no message text.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct PatchSummary {
    pub edited: Vec<usize>,
    pub removed: Vec<usize>,
    pub cascades: Vec<Cascade>,
    pub inserted: usize,
    pub inserted_tool_results: usize,
    pub repairs: Vec<Repair>,
    pub affected: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Cascade {
    pub position: usize,
    pub role: String,
    pub with: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Repair {
    FrozenSystemPrompt,
    ToolMetadataDropped { position: usize },
    UsageRemoved { tokens: usize, usd: f64 },
    CheckpointsRemoved { count: usize },
    DeliveryIdsRemoved { count: usize },
}

/// The calls of a trailing assistant message that no `tool` or `diff` result answers anywhere in
/// `messages`, and how many distinct call ids they leave unanswered.
fn unanswered_trailing_calls(messages: &[ChatMessage]) -> Option<(Vec<&ChatToolCall>, usize)> {
    let tool_calls = messages
        .last()
        .filter(|message| message.role == "assistant")
        .and_then(|message| message.tool_calls.as_ref())?;
    let pending: HashSet<&str> = tool_calls.iter().map(|call| call.id.as_str()).collect();
    let answered: HashSet<&str> = messages
        .iter()
        .filter(|message| message.role == "tool" || message.role == "diff")
        .map(|message| message.tool_call_id.as_str())
        .collect();
    let unanswered = tool_calls
        .iter()
        .filter(|call| !answered.contains(call.id.as_str()))
        .collect();
    let answered_pending = answered.iter().filter(|id| pending.contains(*id)).count();
    Some((unanswered, pending.len() - answered_pending))
}

/// The calls the tool prerun executes before the next request: the unanswered calls of a trailing
/// assistant message, provided `allowed` admits exactly one call per unanswered call id.
pub fn tool_prerun_calls(
    messages: &[ChatMessage],
    allowed: impl Fn(&str) -> bool,
) -> Vec<ChatToolCall> {
    let Some((unanswered, required)) = unanswered_trailing_calls(messages) else {
        return Vec::new();
    };
    let runnable: Vec<ChatToolCall> = unanswered
        .into_iter()
        .filter(|call| allowed(&call.function.name))
        .cloned()
        .collect();
    if !runnable.is_empty() && runnable.len() == required {
        runnable
    } else {
        Vec::new()
    }
}

/// Whether some set of allowed tools would make `tool_prerun_calls` run a call of `messages`.
pub fn tool_prerun_possible(messages: &[ChatMessage]) -> bool {
    let Some((unanswered, required)) = unanswered_trailing_calls(messages) else {
        return false;
    };
    if unanswered.len() == required {
        return required > 0;
    }
    let mut per_name: HashMap<&str, usize> = HashMap::new();
    for call in unanswered {
        *per_name.entry(call.function.name.as_str()).or_default() += 1;
    }
    let mut reachable = vec![false; required + 1];
    reachable[0] = true;
    for count in per_name.into_values() {
        for total in (count..=required).rev() {
            reachable[total] |= reachable[total - count];
        }
    }
    required > 0 && reachable[required]
}

/// The latest assistant message with tool calls at or after `mutable_start` (the latest one that
/// declares `tool_call_id` when given), with its call ids.
pub fn latest_tool_call_window(
    messages: &[ChatMessage],
    mutable_start: usize,
    tool_call_id: Option<&str>,
) -> Option<(usize, Vec<String>)> {
    messages
        .iter()
        .enumerate()
        .skip(mutable_start)
        .rev()
        .find_map(|(index, message)| {
            if message.role != "assistant" {
                return None;
            }
            let calls = message
                .tool_calls
                .as_ref()
                .filter(|calls| !calls.is_empty())?;
            tool_call_id
                .map_or(true, |id| calls.iter().any(|call| call.id == id))
                .then(|| (index, calls.iter().map(|call| call.id.clone()).collect()))
        })
}

/// Call ids answered by `tool` and `diff` results after `assistant_index`, up to the next
/// assistant, user, event, plan or goal message.
pub fn tool_results_after(messages: &[ChatMessage], assistant_index: usize) -> HashSet<String> {
    let mut ids = HashSet::new();
    for message in messages.iter().skip(assistant_index + 1) {
        match message.role.as_str() {
            "tool" | "diff" if !message.tool_call_id.is_empty() => {
                ids.insert(message.tool_call_id.clone());
            }
            "assistant" | "user" | EVENT_ROLE | PLAN_ROLE | GOAL_ROLE => break,
            _ => {}
        }
    }
    ids
}

/// True when the latest assistant message with tool calls in the mutable part has a call without
/// a result in its window: the chat then waits for that result before anything else lands.
pub fn pending_tool_result_window(messages: &[ChatMessage], mutable_start: usize) -> bool {
    latest_tool_call_window(messages, mutable_start, None).is_some_and(|(index, ids)| {
        let answered = tool_results_after(messages, index);
        !ids.iter().all(|id| answered.contains(id))
    })
}

/// Plan a patch of `stored` (another chat's whole stored history). Pure: the result is either a
/// refusal or the candidate history with its approval card, class, warning and summary.
pub fn plan_history_patch(
    stored: &[ChatMessage],
    ops: &[PatchOp],
    env: &PatchEnv,
) -> Result<PatchOutcome, PatchRefusal> {
    check_ops(ops, env.secret_spans)?;
    let history_bytes = serialized_len(stored, MAX_HISTORY_BYTES);
    if history_bytes > MAX_HISTORY_BYTES {
        return Err(PatchRefusal::Refused(format!(
            "that chat's history is over {} MiB; compact it first",
            MAX_HISTORY_BYTES / (1024 * 1024)
        )));
    }
    let base = active_context(stored).map_err(history_refusal)?;
    let start = base.report_index.map_or(0, |index| index + 1);
    let base_view = view_of(&base, Origin::Base);
    let base_analysis = analyse(&base_view);
    let resolved = resolve(stored, start, &base_view, &base_analysis, ops, env)?;
    if resolved.is_nothing() {
        return Ok(PatchOutcome::NothingToChange);
    }
    let (items, shown) = card_items(stored, &resolved, ops, env)?;
    let built = build(stored, &resolved, ops, env)?;
    validate(start, &base_view, &base_analysis, &resolved, &built)?;
    Ok(PatchOutcome::Planned(assemble(
        stored, resolved, items, shown, built,
    )))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Origin {
    Payload(usize),
    Base(usize),
    New(usize, usize),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Flaw {
    Unanswered(Origin, String),
    Duplicated(Origin, String),
    Crossed(Origin, Origin),
    Owner(Origin, Option<Origin>, Option<Origin>),
    Start(Origin),
}

/// The trailing tool call loop: its head and every member that reaches the wire, with digests.
struct Tail {
    position: usize,
    head: Origin,
    members: Vec<(Origin, [u8; 32])>,
}

struct Analysis {
    ownership: Vec<(usize, Flaw)>,
    pairing: Vec<(usize, Flaw)>,
    tail: Option<Tail>,
    start: Option<(usize, Flaw)>,
    owned: HashMap<Origin, Vec<Origin>>,
    owners: HashMap<Origin, (Option<Origin>, Option<Origin>)>,
}

impl Analysis {
    fn flaws(&self) -> HashSet<&Flaw> {
        self.ownership
            .iter()
            .chain(&self.pairing)
            .chain(self.start.iter())
            .map(|(_, flaw)| flaw)
            .collect()
    }
}

struct Edit {
    op: usize,
    text: String,
    fragment: Option<(String, String)>,
    identical: bool,
}

#[derive(Clone, Copy)]
enum Removal {
    Target(usize),
    Range(usize),
    Cascade { op: usize, with: usize },
}

impl Removal {
    fn op(self) -> usize {
        match self {
            Self::Target(op)
            | Self::Range(op)
            | Self::Cascade {
                op, ..
            } => op,
        }
    }
}

struct Insert {
    op: usize,
    gap: usize,
    anchor: Option<usize>,
}

struct Resolved {
    edits: BTreeMap<usize, Edit>,
    removals: BTreeMap<usize, Removal>,
    inserts: Vec<Insert>,
    frozen_system_prompt: Option<String>,
    system_edit: Option<(usize, usize)>,
}

impl Resolved {
    fn changed_edits(&self) -> impl Iterator<Item = (&usize, &Edit)> {
        self.edits.iter().filter(|(_, edit)| !edit.identical)
    }

    fn is_nothing(&self) -> bool {
        self.removals.is_empty() && self.inserts.is_empty() && self.changed_edits().count() == 0
    }
}

struct Built {
    messages: Vec<ChatMessage>,
    origins: Vec<Origin>,
    inserted: Vec<InsertedMessage>,
}

fn history_refusal(error: ActiveContextError) -> PatchRefusal {
    match error {
        ActiveContextError::NeedsExplicitRebuild => PatchRefusal::NeedsRebuild(
            "that chat's history needs an explicit rebuild; run chats.compact first".into(),
        ),
        other => PatchRefusal::Refused(format!("that chat's history cannot be patched: {other}")),
    }
}

struct CappedCounter {
    count: usize,
    cap: usize,
}

impl Write for CappedCounter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.count += buf.len();
        if self.count > self.cap {
            return Err(std::io::Error::other("over the limit"));
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Serialized JSON size, counted without building the JSON; stops counting past `cap`.
fn serialized_len<T: Serialize + ?Sized>(value: &T, cap: usize) -> usize {
    let mut counter = CappedCounter {
        count: 0,
        cap,
    };
    let _ = serde_json::to_writer(&mut counter, value);
    counter.count
}

fn json_len<T: Serialize + ?Sized>(value: &T) -> usize {
    serialized_len(value, usize::MAX)
}

fn op_label(op: usize) -> String {
    format!("op {}", op + 1)
}

fn valid_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_TOOL_NAME_CHARS
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn call_card_value(call: &InsertedCall) -> Value {
    json!({"arguments": call.arguments, "result": call.result, "failed": call.failed})
}

fn secret_refusal(
    op: usize,
    text: &str,
    secret_spans: &dyn Fn(&str) -> Vec<Range<usize>>,
) -> Result<(), PatchRefusal> {
    if !secret_spans(text).is_empty() {
        return Err(PatchRefusal::Refused(format!(
            "{}: the new text looks like a secret (a token, key or password); scripts cannot write secrets into chats",
            op_label(op)
        )));
    }
    Ok(())
}

fn size_refusal(op: usize, what: &str, bytes: usize) -> Result<usize, PatchRefusal> {
    if bytes > MAX_TEXT_BYTES {
        return Err(PatchRefusal::Refused(format!(
            "{}: {what} is {bytes} bytes; the limit is {MAX_TEXT_BYTES} bytes",
            op_label(op)
        )));
    }
    Ok(bytes)
}

fn check_ops(
    ops: &[PatchOp],
    secret_spans: &dyn Fn(&str) -> Vec<Range<usize>>,
) -> Result<(), PatchRefusal> {
    if ops.is_empty() {
        return Err(PatchRefusal::Invalid(
            "ops must hold at least one operation".into(),
        ));
    }
    if ops.len() > MAX_OPS {
        return Err(PatchRefusal::Invalid(format!(
            "{} operations in one call; the limit is {MAX_OPS}",
            ops.len()
        )));
    }
    let mut total = 0usize;
    for (op, patch_op) in ops.iter().enumerate() {
        match patch_op {
            PatchOp::Edit {
                message_id,
                text,
                find,
                replace,
            } => {
                if message_id.is_empty() {
                    return Err(PatchRefusal::Invalid(format!(
                        "{}: message_id is empty",
                        op_label(op)
                    )));
                }
                match (text, find, replace) {
                    (Some(text), None, None) => {
                        total += size_refusal(op, "the new text", json_len(text.as_str()))?;
                        secret_refusal(op, text, secret_spans)?;
                    }
                    (None, Some(find), Some(replace)) => {
                        if find.is_empty() {
                            return Err(PatchRefusal::Invalid(format!(
                                "{}: find must not be empty",
                                op_label(op)
                            )));
                        }
                        size_refusal(op, "find", json_len(find.as_str()))?;
                        total += size_refusal(op, "replace", json_len(replace.as_str()))?;
                        secret_refusal(op, replace, secret_spans)?;
                    }
                    _ => {
                        return Err(PatchRefusal::Invalid(format!(
                            "{}: an edit takes either text, or find and replace",
                            op_label(op)
                        )))
                    }
                }
            }
            PatchOp::Remove {
                message_id,
                through,
                ..
            } => {
                if message_id.is_empty() || through.as_ref().is_some_and(String::is_empty) {
                    return Err(PatchRefusal::Invalid(format!(
                        "{}: message_id and through must not be empty",
                        op_label(op)
                    )));
                }
            }
            PatchOp::Insert {
                after,
                role,
                text,
                tool_calls,
            } => {
                if after.is_empty() {
                    return Err(PatchRefusal::Invalid(format!(
                        "{}: after is empty",
                        op_label(op)
                    )));
                }
                let calls = tool_calls.as_deref().unwrap_or_default();
                if !calls.is_empty() && *role != InsertRole::Assistant {
                    return Err(PatchRefusal::Invalid(format!(
                        "{}: only an assistant message can carry tool calls",
                        op_label(op)
                    )));
                }
                if text.trim().is_empty() && calls.is_empty() {
                    return Err(PatchRefusal::Invalid(format!(
                        "{}: an inserted {} message needs text",
                        op_label(op),
                        role.as_str()
                    )));
                }
                if calls.len() > MAX_CALLS_PER_INSERT {
                    return Err(PatchRefusal::Refused(format!(
                        "{}: {} tool calls in one message; the limit is {MAX_CALLS_PER_INSERT}",
                        op_label(op),
                        calls.len()
                    )));
                }
                total += size_refusal(op, "the new text", json_len(text.as_str()))?;
                secret_refusal(op, text, secret_spans)?;
                for (index, call) in calls.iter().enumerate() {
                    if !valid_tool_name(&call.name) {
                        return Err(PatchRefusal::Invalid(format!(
                            "{}: tool call {} needs a name of 1-{MAX_TOOL_NAME_CHARS} letters, digits, '_' or '-'",
                            op_label(op),
                            index + 1
                        )));
                    }
                    size_refusal(
                        op,
                        "the arguments of a tool call",
                        json_len(&call.arguments),
                    )?;
                    size_refusal(
                        op,
                        "the result of a tool call",
                        json_len(call.result.as_str()),
                    )?;
                    total += size_refusal(
                        op,
                        "a tool call with its arguments and result",
                        json_len(&call_card_value(call)),
                    )?;
                    secret_refusal(op, &call.result, secret_spans)?;
                    secret_refusal(
                        op,
                        &Value::Object(call.arguments.clone()).to_string(),
                        secret_spans,
                    )?;
                }
            }
        }
        if total > MAX_NEW_TEXT_BYTES {
            return Err(PatchRefusal::Refused(format!(
                "the new text of this call is over {MAX_NEW_TEXT_BYTES} bytes; split the patch"
            )));
        }
    }
    Ok(())
}

fn view_of(
    active: &ActiveContext,
    stored_origin: impl Fn(usize) -> Origin,
) -> Vec<(Origin, &ChatMessage)> {
    active
        .messages
        .iter()
        .zip(&active.origins)
        .map(|(message, origin)| {
            let origin = match origin {
                MessageOrigin::ReportPayload {
                    message_index, ..
                } => Origin::Payload(*message_index),
                MessageOrigin::Stored {
                    message_index,
                } => stored_origin(*message_index),
            };
            (origin, message)
        })
        .collect()
}

fn is_separator(role: &str) -> bool {
    matches!(
        role,
        "user" | "assistant" | "system" | EVENT_ROLE | PLAN_ROLE | GOAL_ROLE
    )
}

fn is_result(message: &ChatMessage) -> bool {
    matches!(message.role.as_str(), "tool" | "diff" | "context_file")
        && !message.tool_call_id.is_empty()
}

fn is_primary_result(message: &ChatMessage) -> bool {
    matches!(message.role.as_str(), "tool" | "diff") && !message.tool_call_id.is_empty()
}

fn head_calls(message: &ChatMessage) -> Option<&[ChatToolCall]> {
    if message.role != "assistant" || !reaches_wire(message) {
        return None;
    }
    message
        .tool_calls
        .as_deref()
        .filter(|calls| !calls.is_empty())
}

fn declared_calls(message: &ChatMessage) -> Option<&[ChatToolCall]> {
    if message.role != "assistant" {
        return None;
    }
    message
        .tool_calls
        .as_deref()
        .filter(|calls| !calls.is_empty())
}

/// For each result, the nearest earlier message that `declares` its call id, else the nearest
/// later one.
fn owners_with<'a>(
    view: &[(Origin, &'a ChatMessage)],
    declares: impl Fn(&'a ChatMessage) -> Option<&'a [ChatToolCall]>,
) -> Vec<Option<usize>> {
    let mut heads_by_call: HashMap<&str, Vec<usize>> = HashMap::new();
    for (position, (_, message)) in view.iter().enumerate() {
        for call in declares(message).unwrap_or_default() {
            let heads = heads_by_call.entry(call.id.as_str()).or_default();
            if heads.last() != Some(&position) {
                heads.push(position);
            }
        }
    }
    view.iter()
        .enumerate()
        .map(|(position, (_, message))| {
            if !is_result(message) {
                return None;
            }
            let heads = heads_by_call.get(message.tool_call_id.as_str())?;
            let after = heads.partition_point(|&head| head < position);
            after
                .checked_sub(1)
                .map(|index| heads[index])
                .or_else(|| heads.get(after).copied())
        })
        .collect()
}

/// Whether the at-commands step before the tool prerun may drop this message: a user message in
/// plain text that is blank or starts with a word beginning with `@`, which may be a command whose
/// replacement consumes the rest of it. Text before the first command always survives.
fn may_vanish_before_prerun(message: &ChatMessage) -> bool {
    if message.role != "user" || matches!(message.content, ChatContent::Multimodal(_)) {
        return false;
    }
    let text = message.content.content_text_only();
    text.split_whitespace()
        .next()
        .is_none_or(|word| word.starts_with('@'))
}

/// How many of `messages` (in wire order) reach the tool prerun once a trailing run of user
/// messages that may all vanish is dropped.
fn kept_before_prerun<'a>(
    messages: impl DoubleEndedIterator<Item = &'a ChatMessage> + ExactSizeIterator,
) -> usize {
    let total = messages.len();
    let mut users = 0;
    let mut all_vanish = true;
    for message in messages.rev() {
        if message.role != "user" {
            break;
        }
        users += 1;
        all_vanish &= may_vanish_before_prerun(message);
    }
    if users > 0 && all_vanish {
        total - users
    } else {
        total
    }
}

/// Typed call bundles of a view. A result belongs to the nearest earlier assistant message that
/// declares its call id (else the nearest later one), counted twice: among all assistant messages
/// (raw owner) and among those that reach the wire (wire owner, which decides pairing). A bundle's
/// window ends at the next user, assistant, system, event, plan or goal message that reaches the
/// wire, and a call is answered by a `tool` or `diff` result inside it. Messages that never reach
/// the wire go with their owners but neither answer a call nor separate turns; such a result
/// outside its raw owner's window is a crossed bundle too.
fn analyse(view: &[(Origin, &ChatMessage)]) -> Analysis {
    let visible: Vec<bool> = view
        .iter()
        .map(|(_, message)| reaches_wire(message))
        .collect();
    let raw_owners = owners_with(view, declared_calls);
    let wire_owners = owners_with(view, head_calls);
    let mut window_end = vec![view.len(); view.len()];
    let mut next = view.len();
    for position in (0..view.len()).rev() {
        window_end[position] = next;
        if visible[position] && is_separator(&view[position].1.role) {
            next = position;
        }
    }
    let mut answers_of: HashMap<(usize, &str), Vec<usize>> = HashMap::new();
    for (position, owner) in wire_owners.iter().enumerate() {
        if let (Some(head), true) = (owner, visible[position]) {
            answers_of
                .entry((*head, view[position].1.tool_call_id.as_str()))
                .or_default()
                .push(position);
        }
    }
    let ownership = view
        .iter()
        .enumerate()
        .filter(|(_, (origin, message))| is_result(message) && !matches!(origin, Origin::New(..)))
        .map(|(position, (origin, _))| {
            let owner = |owners: &[Option<usize>]| owners[position].map(|head| view[head].0);
            (
                position,
                Flaw::Owner(*origin, owner(&raw_owners), owner(&wire_owners)),
            )
        })
        .collect();
    let mut pairing = Vec::new();
    for (position, (origin, message)) in view.iter().enumerate() {
        let Some(calls) = head_calls(message) else {
            continue;
        };
        let mut seen = HashSet::new();
        for call in calls {
            if !seen.insert(call.id.as_str()) {
                continue;
            }
            let answers = answers_of
                .get(&(position, call.id.as_str()))
                .map(Vec::as_slice)
                .unwrap_or_default();
            let in_window = |result: usize| result > position && result < window_end[position];
            for &result in answers.iter().filter(|&&result| !in_window(result)) {
                pairing.push((position, Flaw::Crossed(*origin, view[result].0)));
            }
            let primaries: Vec<usize> = answers
                .iter()
                .copied()
                .filter(|&result| is_primary_result(view[result].1))
                .collect();
            if !primaries.iter().any(|&result| in_window(result)) {
                pairing.push((position, Flaw::Unanswered(*origin, call.id.clone())));
            }
            if primaries.len() > 1 {
                pairing.push((position, Flaw::Duplicated(*origin, call.id.clone())));
            }
        }
    }
    for (position, owner) in raw_owners.iter().enumerate() {
        if let (Some(head), false) = (*owner, visible[position]) {
            if position < head || position >= window_end[head] {
                pairing.push((head, Flaw::Crossed(view[head].0, view[position].0)));
            }
        }
    }

    let on_wire: Vec<usize> = (0..view.len())
        .filter(|&position| visible[position])
        .collect();
    let kept = &on_wire[..kept_before_prerun(on_wire.iter().map(|&position| view[position].1))];
    let tail = kept
        .iter()
        .rposition(|&position| is_separator(&view[position].1.role))
        .filter(|&at| head_calls(view[kept[at]].1).is_some())
        .map(|at| Tail {
            position: kept[at],
            head: view[kept[at]].0,
            members: kept[at..]
                .iter()
                .map(|&position| {
                    let bytes =
                        serde_json::to_vec(view[position].1).expect("chat messages serialize");
                    (view[position].0, Sha256::digest(bytes).into())
                })
                .collect(),
        });
    let start = (0..view.len())
        .find(|&position| visible[position])
        .filter(|&position| {
            !matches!(
                view[position].1.role.as_str(),
                "system" | "user" | EVENT_ROLE | PLAN_ROLE | GOAL_ROLE
            )
        })
        .map(|position| (position, Flaw::Start(view[position].0)));

    let mut owned: HashMap<Origin, Vec<Origin>> = HashMap::new();
    let mut owners = HashMap::new();
    for position in (0..view.len()).filter(|&position| is_result(view[position].1)) {
        let (wire, raw) = (wire_owners[position], raw_owners[position]);
        for head in wire.into_iter().chain(raw.filter(|raw| Some(*raw) != wire)) {
            owned
                .entry(view[head].0)
                .or_default()
                .push(view[position].0);
        }
        owners.insert(
            view[position].0,
            (raw.map(|head| view[head].0), wire.map(|head| view[head].0)),
        );
    }
    Analysis {
        ownership,
        pairing,
        tail,
        start,
        owned,
        owners,
    }
}

fn plain_assistant(message: &ChatMessage) -> bool {
    message.tool_calls.as_ref().is_none_or(Vec::is_empty)
        && message.thinking_blocks.as_ref().is_none_or(Vec::is_empty)
        && message.server_content_blocks.is_empty()
        && message.citations.is_empty()
        && message
            .reasoning_content
            .as_ref()
            .is_none_or(String::is_empty)
        && !message.extra.keys().any(|key| key.starts_with('_'))
}

fn has_non_text_parts(content: &ChatContent) -> bool {
    match content {
        ChatContent::SimpleText(_) => false,
        ChatContent::Multimodal(parts) => parts.iter().any(|part| part.m_type != "text"),
        ChatContent::ContextFiles(_) => true,
    }
}

fn edit_refusal(message: &ChatMessage, fragment: bool) -> Option<&'static str> {
    if is_never_model_visible(message) {
        return Some("it is never shown to the model");
    }
    if message.extra.contains_key("privacy_shell")
        || message.extra.contains_key("privacy_observation")
    {
        return Some("a privacy decision covers its content");
    }
    if message.extra.contains_key("privacy_ref") {
        return Some("its privacy records are unresolved");
    }
    match message.role.as_str() {
        "user" | "system" => {}
        "assistant" if !plain_assistant(message) => {
            return Some(
                "only plain text assistant messages are editable; this one carries tool calls, thinking, reasoning or provider blocks, remove it with its results instead",
            )
        }
        "assistant" => {}
        "tool" if message.extra.contains_key("edit_result") => {
            return Some("it is the result of a file edit")
        }
        "tool" if message.extra.contains_key("python") => {
            return Some("it is the result of a python run")
        }
        "tool" => {}
        _ => return Some("only user, assistant, system and plain tool messages are editable"),
    }
    if fragment && !matches!(message.content, ChatContent::SimpleText(_)) {
        return Some("find and replace works on plain text messages only");
    }
    if !fragment && has_non_text_parts(&message.content) {
        return Some("it has parts that are not text");
    }
    None
}

fn removal_refusal(message: &ChatMessage) -> Option<&'static str> {
    match message.role.as_str() {
        "system" | "developer" => {
            Some("system and developer messages cannot be removed; edit the system message instead")
        }
        EVENT_ROLE | PLAN_ROLE | GOAL_ROLE => {
            Some("goal, plan and event messages belong to the goal machinery")
        }
        COMPRESSION_REPORT_ROLE | "summarization" => Some("it belongs to compressed history"),
        _ => None,
    }
}

/// Occurrences of `needle` in `text`, overlapping ones included, counted up to two.
fn occurrences(text: &str, needle: &str) -> (usize, Option<usize>) {
    let step = needle.chars().next().map_or(1, char::len_utf8);
    let mut first = None;
    let mut from = 0;
    let mut count = 0;
    while let Some(found) = text[from..].find(needle) {
        count += 1;
        first.get_or_insert(from + found);
        if count > 1 {
            break;
        }
        from += found + step;
    }
    (count, first)
}

fn plain_text(content: &ChatContent) -> Option<&str> {
    match content {
        ChatContent::SimpleText(text) => Some(text),
        _ => None,
    }
}

fn current_text(message: &ChatMessage) -> String {
    message.content.content_text_only()
}

fn affected_refusal(affected: usize) -> PatchRefusal {
    PatchRefusal::Refused(format!(
        "the patch affects {affected} messages; the limit is {MAX_AFFECTED_MESSAGES}, split the patch"
    ))
}

fn resolve(
    stored: &[ChatMessage],
    start: usize,
    base_view: &[(Origin, &ChatMessage)],
    base_analysis: &Analysis,
    ops: &[PatchOp],
    env: &PatchEnv,
) -> Result<Resolved, PatchRefusal> {
    let mut by_id: HashMap<&str, usize> = HashMap::new();
    for (index, message) in stored.iter().enumerate().skip(start) {
        if !message.message_id.is_empty() {
            by_id.insert(message.message_id.as_str(), index);
        }
    }
    let compressed: HashSet<&str> = stored[..start]
        .iter()
        .map(|message| message.message_id.as_str())
        .chain(
            base_view
                .iter()
                .filter(|(origin, _)| matches!(origin, Origin::Payload(_)))
                .map(|(_, message)| message.message_id.as_str()),
        )
        .filter(|id| !id.is_empty())
        .collect();
    let locate = |op: usize, id: &str| -> Result<usize, PatchRefusal> {
        if let Some(index) = by_id.get(id) {
            return Ok(*index);
        }
        if compressed.contains(id) {
            return Err(PatchRefusal::Refused(format!(
                "{}: message {id} is inside compressed history; only messages after the last compression can change",
                op_label(op)
            )));
        }
        Err(PatchRefusal::Invalid(format!(
            "{}: message {id} is not in that chat",
            op_label(op)
        )))
    };
    let prompts: Vec<Origin> = base_view
        .iter()
        .filter(|(_, message)| matches!(message.role.as_str(), "system" | "developer"))
        .map(|(origin, _)| *origin)
        .collect();

    let mut claimed: HashMap<usize, usize> = HashMap::new();
    let claim = |claimed: &mut HashMap<usize, usize>, index: usize, op: usize| match claimed
        .insert(index, op)
    {
        Some(other) => Err(PatchRefusal::Invalid(format!(
            "{}: message #{index} is already changed by {}",
            op_label(op),
            op_label(other)
        ))),
        None => Ok(()),
    };
    let mut resolved = Resolved {
        edits: BTreeMap::new(),
        removals: BTreeMap::new(),
        inserts: Vec::new(),
        frozen_system_prompt: None,
        system_edit: None,
    };
    let mut cascade_heads: Vec<(usize, usize)> = Vec::new();
    let mut system_inserts = 0;

    for (op, patch_op) in ops.iter().enumerate() {
        match patch_op {
            PatchOp::Edit {
                message_id,
                text,
                find,
                replace,
            } => {
                let index = locate(op, message_id)?;
                claim(&mut claimed, index, op)?;
                let message = &stored[index];
                if let Some(why) = edit_refusal(message, find.is_some()) {
                    return Err(PatchRefusal::Refused(format!(
                        "{}: message #{index} ({}) cannot be edited: {why}",
                        op_label(op),
                        role_name(&message.role)
                    )));
                }
                let (new_text, fragment) = match (text, find, replace) {
                    (Some(text), _, _) => (text.clone(), None),
                    (None, Some(find), Some(replace)) => {
                        let old = plain_text(&message.content).unwrap_or_default();
                        let (count, first) = occurrences(old, find);
                        let Some(at) = first.filter(|_| count == 1) else {
                            return Err(PatchRefusal::Refused(format!(
                                "{}: find occurs {} in message #{index}; it must occur exactly once",
                                op_label(op),
                                match count {
                                    0 => "nowhere",
                                    _ => "more than once (overlapping occurrences count)",
                                }
                            )));
                        };
                        let assembled =
                            format!("{}{replace}{}", &old[..at], &old[at + find.len()..]);
                        let end = at + replace.len();
                        let before = (env.secret_spans)(old);
                        let gained = find != replace
                            && (env.secret_spans)(&assembled).into_iter().any(|span| {
                                let new_text = if replace.is_empty() {
                                    span.start < at && span.end > at
                                } else {
                                    span.start < end && span.end > at
                                };
                                let in_old = if span.start >= end {
                                    span.start - replace.len() + find.len()
                                        ..span.end - replace.len() + find.len()
                                } else {
                                    span.clone()
                                };
                                new_text || !before.contains(&in_old)
                            });
                        if gained {
                            return Err(PatchRefusal::Refused(format!(
                                "{}: the edited text of message #{index} would hold something that looks like a secret around the replaced fragment",
                                op_label(op)
                            )));
                        }
                        (assembled, Some((find.clone(), replace.clone())))
                    }
                    _ => unreachable!("checked by check_ops"),
                };
                if message.role != "tool" && new_text.trim().is_empty() {
                    return Err(PatchRefusal::Refused(format!(
                        "{}: the new text of message #{index} is empty",
                        op_label(op)
                    )));
                }
                let identical = plain_text(&message.content) == Some(new_text.as_str());
                if message.role == "system" && !identical {
                    if prompts != [Origin::Base(index)] {
                        return Err(PatchRefusal::Refused(format!(
                            "{}: that chat has other system or developer messages, so an edited system prompt might not reach its model",
                            op_label(op)
                        )));
                    }
                    resolved.system_edit = Some((index, op));
                    match env.frozen {
                        FrozenSystem::Absent => {}
                        FrozenSystem::Partial => {
                            return Err(PatchRefusal::Refused(format!(
                                "{}: that chat's frozen request prefix is incomplete, so a new system prompt would not reach its model",
                                op_label(op)
                            )))
                        }
                        FrozenSystem::Complete(frozen) if frozen != current_text(message) => {
                            return Err(PatchRefusal::Refused(format!(
                                "{}: that chat's frozen system prompt differs from its system message, so the edit would not reach its model",
                                op_label(op)
                            )))
                        }
                        FrozenSystem::Complete(_) => {
                            resolved.frozen_system_prompt = Some(new_text.clone());
                        }
                    }
                }
                resolved.edits.insert(
                    index,
                    Edit {
                        op,
                        text: new_text,
                        fragment,
                        identical,
                    },
                );
            }
            PatchOp::Remove {
                message_id,
                through,
                with_results,
            } => {
                let first = locate(op, message_id)?;
                let last = match through {
                    Some(through) => locate(op, through)?,
                    None => first,
                };
                if last < first {
                    return Err(PatchRefusal::Invalid(format!(
                        "{}: through (#{last}) comes before message_id (#{first})",
                        op_label(op)
                    )));
                }
                if resolved.removals.len() + last - first + 1 > MAX_AFFECTED_MESSAGES {
                    return Err(affected_refusal(resolved.removals.len() + last - first + 1));
                }
                if last - first + 1 > MAX_RANGE_MESSAGES {
                    return Err(PatchRefusal::Refused(format!(
                        "{}: the range holds {} messages; one remove takes at most {MAX_RANGE_MESSAGES}",
                        op_label(op),
                        last - first + 1
                    )));
                }
                for index in first..=last {
                    claim(&mut claimed, index, op)?;
                    if let Some(why) = removal_refusal(&stored[index]) {
                        return Err(PatchRefusal::Refused(format!(
                            "{}: message #{index} ({}) cannot be removed: {why}",
                            op_label(op),
                            role_name(&stored[index].role)
                        )));
                    }
                    let removal = if through.is_some() {
                        Removal::Range(op)
                    } else {
                        Removal::Target(op)
                    };
                    resolved.removals.insert(index, removal);
                    if *with_results {
                        cascade_heads.push((index, op));
                    }
                }
            }
            PatchOp::Insert {
                after,
                role,
                ..
            } => {
                let (gap, anchor) = match after.as_str() {
                    "start" if start > 0 => {
                        return Err(PatchRefusal::Refused(format!(
                            "{}: that chat's history is compressed; anchor the insert on a message id or use \"end\"",
                            op_label(op)
                        )))
                    }
                    "start" => (
                        usize::from(stored.first().is_some_and(|message| message.role == "system")),
                        None,
                    ),
                    "end" => (stored.len(), None),
                    id => {
                        let anchor = locate(op, id)?;
                        (anchor + 1, Some(anchor))
                    }
                };
                if *role == InsertRole::System {
                    system_inserts += 1;
                    let refusal = if after != "start" {
                        Some("a system message can only be inserted at the start")
                    } else if !prompts.is_empty() || system_inserts > 1 {
                        Some("that chat already has a system or developer message")
                    } else if env.frozen != FrozenSystem::Absent {
                        Some("that chat has a frozen system prompt, which would replace the inserted one")
                    } else {
                        None
                    };
                    if let Some(why) = refusal {
                        return Err(PatchRefusal::Refused(format!("{}: {why}", op_label(op))));
                    }
                }
                resolved.inserts.push(Insert {
                    op,
                    gap,
                    anchor,
                });
            }
        }
    }

    for (head, op) in cascade_heads {
        let Some(results) = base_analysis.owned.get(&Origin::Base(head)) else {
            continue;
        };
        for result in results {
            let Origin::Base(index) = *result else {
                return Err(PatchRefusal::Refused(format!(
                    "{}: a result of message #{head} is inside compressed history",
                    op_label(op)
                )));
            };
            if let Some(edit) = resolved.edits.get(&index) {
                return Err(PatchRefusal::Invalid(format!(
                    "{}: message #{index} is removed with its call and edited by {}",
                    op_label(op),
                    op_label(edit.op)
                )));
            }
            resolved.removals.entry(index).or_insert(Removal::Cascade {
                op,
                with: head,
            });
            if resolved.removals.len() > MAX_AFFECTED_MESSAGES {
                return Err(affected_refusal(resolved.removals.len()));
            }
        }
    }
    for insert in &resolved.inserts {
        if let Some(removal) = insert
            .anchor
            .and_then(|anchor| resolved.removals.get(&anchor))
        {
            return Err(PatchRefusal::Invalid(format!(
                "{}: its anchor #{} is removed by {}",
                op_label(insert.op),
                insert.anchor.unwrap_or_default(),
                op_label(removal.op())
            )));
        }
    }
    let inserted: usize = resolved
        .inserts
        .iter()
        .map(|insert| match &ops[insert.op] {
            PatchOp::Insert {
                tool_calls, ..
            } => 1 + tool_calls.as_ref().map_or(0, Vec::len),
            _ => 0,
        })
        .sum();
    let affected = resolved.changed_edits().count() + resolved.removals.len() + inserted;
    if affected > MAX_AFFECTED_MESSAGES {
        return Err(affected_refusal(affected));
    }
    Ok(resolved)
}

fn cut(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut end = max_bytes.saturating_sub('…'.len_utf8());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

/// The first `max_bytes` of the concatenated pieces, reading only as many pieces as fit.
fn bounded<'a>(pieces: impl IntoIterator<Item = Cow<'a, str>>, max_bytes: usize) -> String {
    let mut out = String::new();
    for piece in pieces {
        if out.len() + piece.len() <= max_bytes {
            out.push_str(&piece);
            continue;
        }
        let mut end = max_bytes - out.len();
        while !piece.is_char_boundary(end) {
            end -= 1;
        }
        out.push_str(&piece[..end]);
        out.push('…');
        return cut(&out, max_bytes);
    }
    out
}

fn separated<'a>(
    pieces: impl IntoIterator<Item = Cow<'a, str>>,
    separator: &'a str,
) -> impl Iterator<Item = Cow<'a, str>> {
    pieces
        .into_iter()
        .enumerate()
        .flat_map(move |(index, piece)| {
            (index > 0)
                .then_some(Cow::Borrowed(separator))
                .into_iter()
                .chain([piece])
        })
}

fn shown_text(message: &ChatMessage, max_bytes: usize) -> Value {
    let calls = message
        .tool_calls
        .as_ref()
        .filter(|calls| !calls.is_empty())
        .map(|calls| {
            let names = calls
                .iter()
                .map(|call| Cow::Borrowed(call.function.name.as_str()));
            let pieces = [Cow::Borrowed("\n[tool calls: ")]
                .into_iter()
                .chain(separated(names, ", "))
                .chain([Cow::Borrowed("]")]);
            bounded(pieces, max_bytes / 2)
        })
        .unwrap_or_default();
    let room = max_bytes.saturating_sub(calls.len());
    let text = match &message.content {
        ChatContent::SimpleText(text) => cut(text, room),
        ChatContent::Multimodal(parts) => bounded(
            separated(
                parts.iter().map(|part| match part.m_type.as_str() {
                    "text" => Cow::Borrowed(part.m_content.as_str()),
                    kind if kind.starts_with("image/") => Cow::Borrowed("[image]"),
                    kind => Cow::Owned(format!("[unsupported:{kind}]")),
                }),
                "\n\n",
            ),
            room,
        ),
        ChatContent::ContextFiles(files) => bounded(
            files.iter().enumerate().flat_map(|(index, file)| {
                [
                    Cow::Borrowed(if index > 0 { "\n\n" } else { "" }),
                    Cow::Owned(format!(
                        "{}:{}-{}\n",
                        file.file_name, file.line1, file.line2
                    )),
                    Cow::Borrowed(file.file_content.as_str()),
                ]
            }),
            room,
        ),
    };
    Value::String(cut(&format!("{text}{calls}"), max_bytes))
}

fn insert_place(insert: &Insert, ops: &[PatchOp]) -> String {
    match (&ops[insert.op], insert.anchor) {
        (_, Some(anchor)) => format!("after #{anchor}"),
        (
            PatchOp::Insert {
                after, ..
            },
            None,
        ) if after == "start" => "at the start".into(),
        _ => "at the end".into(),
    }
}

struct Card {
    items: Vec<PatchItem>,
    shown: Vec<usize>,
    size: usize,
}

impl Card {
    fn push(
        &mut self,
        label: String,
        old: Option<Value>,
        new: Option<Value>,
    ) -> Result<(), PatchRefusal> {
        let item = PatchItem {
            label,
            old,
            new,
        };
        self.size += serialized_len(&item, MAX_CARD_BYTES) + 1;
        if self.size > MAX_CARD_BYTES {
            return Err(PatchRefusal::Refused(format!(
                "the approval card would be over {MAX_CARD_BYTES} bytes; split the patch"
            )));
        }
        self.items.push(item);
        Ok(())
    }
}

fn card_items(
    stored: &[ChatMessage],
    resolved: &Resolved,
    ops: &[PatchOp],
    env: &PatchEnv,
) -> Result<(Vec<PatchItem>, Vec<usize>), PatchRefusal> {
    let mut card = Card {
        items: Vec::new(),
        shown: Vec::new(),
        size: CARD_ENVELOPE_BYTES + 1,
    };
    for (op, patch_op) in ops.iter().enumerate() {
        match patch_op {
            PatchOp::Edit {
                ..
            } => {
                let Some((index, edit)) = resolved.changed_edits().find(|(_, edit)| edit.op == op)
                else {
                    continue;
                };
                let message = &stored[*index];
                card.shown.push(*index);
                match &edit.fragment {
                    Some((find, replace)) => card.push(
                        format!("#{index} {}: replace text", role_name(&message.role)),
                        Some(Value::String(find.clone())),
                        Some(Value::String(replace.clone())),
                    )?,
                    None => card.push(
                        format!("#{index} {}: edit", role_name(&message.role)),
                        Some(shown_text(message, OLD_TEXT_BYTES)),
                        Some(Value::String(edit.text.clone())),
                    )?,
                }
                let dropped: Vec<&str> = DROPPED_TOOL_KEYS
                    .into_iter()
                    .filter(|key| message.role == "tool" && message.extra.contains_key(*key))
                    .collect();
                if !dropped.is_empty() {
                    card.push(
                        format!("#{index} tool: stored tool output dropped"),
                        Some(Value::String(dropped.join(", "))),
                        None,
                    )?;
                }
                if let (FrozenSystem::Complete(frozen), Some(_), "system") = (
                    env.frozen,
                    &resolved.frozen_system_prompt,
                    message.role.as_str(),
                ) {
                    let (old, new) = match &edit.fragment {
                        Some((find, replace)) => (find.clone(), replace.clone()),
                        None => (cut(frozen, OLD_TEXT_BYTES), edit.text.clone()),
                    };
                    card.push(
                        "system prompt (frozen copy)".into(),
                        Some(Value::String(old)),
                        Some(Value::String(new)),
                    )?;
                }
            }
            PatchOp::Remove {
                through, ..
            } => {
                let targets: Vec<usize> = resolved
                    .removals
                    .iter()
                    .filter(|(_, removal)| matches!(removal, Removal::Target(o) | Removal::Range(o) if *o == op))
                    .map(|(index, _)| *index)
                    .collect();
                let (Some(&first), Some(&last)) = (targets.first(), targets.last()) else {
                    continue;
                };
                if through.is_some() && last > first {
                    card.shown.extend([first, last]);
                    let mut roles: BTreeMap<&str, usize> = BTreeMap::new();
                    for index in &targets {
                        *roles.entry(role_name(&stored[*index].role)).or_default() += 1;
                    }
                    card.push(
                        format!("#{first}–#{last}: remove {} messages", targets.len()),
                        Some(json!({
                            "roles": roles,
                            "first": shown_text(&stored[first], RANGE_EDGE_BYTES),
                            "last": shown_text(&stored[last], RANGE_EDGE_BYTES),
                        })),
                        None,
                    )?;
                } else {
                    card.shown.push(first);
                    card.push(
                        format!("#{first} {}: remove", role_name(&stored[first].role)),
                        Some(shown_text(&stored[first], OLD_TEXT_BYTES)),
                        None,
                    )?;
                }
                for (index, removal) in &resolved.removals {
                    if let Removal::Cascade {
                        op: o,
                        with,
                    } = removal
                    {
                        if *o == op {
                            card.shown.push(*index);
                            card.push(
                                format!(
                                    "#{index} {}: removed with #{with}",
                                    role_name(&stored[*index].role)
                                ),
                                Some(shown_text(&stored[*index], CASCADE_TEXT_BYTES)),
                                None,
                            )?;
                        }
                    }
                }
            }
            PatchOp::Insert {
                role,
                text,
                tool_calls,
                ..
            } => {
                let Some(insert) = resolved.inserts.iter().find(|insert| insert.op == op) else {
                    continue;
                };
                let place = insert_place(insert, ops);
                card.push(
                    format!("{place}: insert {}", role.as_str()),
                    None,
                    Some(Value::String(text.clone())),
                )?;
                for call in tool_calls.iter().flatten() {
                    card.push(
                        format!("{place}: insert call {}", call.name),
                        None,
                        Some(call_card_value(call)),
                    )?;
                }
            }
        }
    }
    let removed: Vec<usize> = resolved.removals.keys().copied().collect();
    for row in removal_rows(stored, &removed) {
        card.push(row.label, row.old, row.new)?;
    }
    card.shown.sort_unstable();
    card.shown.dedup();
    Ok((card.items, card.shown))
}

/// The rows the card of a removal of `removed` (positions of `stored`, in order) shows besides the
/// removed text: the recorded usage and cost, and the file checkpoints that go with it (V16).
pub fn removal_rows(stored: &[ChatMessage], removed: &[usize]) -> Vec<PatchItem> {
    let mut rows = Vec::new();
    let (tokens, usd) = usage_of(stored, removed);
    if tokens > 0 || usd > 0.0 {
        rows.push(PatchItem {
            label: "recorded usage removed".into(),
            old: Some(Value::String(format!("{tokens} tokens, ${usd:.4}"))),
            new: None,
        });
    }
    let with_checkpoints = || {
        removed
            .iter()
            .filter(|index| !stored[**index].checkpoints.is_empty())
    };
    if with_checkpoints().next().is_some() {
        let lines = with_checkpoints().map(|index| {
            let hashes: Vec<&str> = stored[*index]
                .checkpoints
                .iter()
                .map(|checkpoint| {
                    checkpoint
                        .commit_hash
                        .get(..12)
                        .unwrap_or(&checkpoint.commit_hash)
                })
                .collect();
            Cow::Owned(format!("#{index}: {}", hashes.join(", ")))
        });
        rows.push(PatchItem {
            label: "file checkpoints removed".into(),
            old: Some(Value::String(bounded(
                separated(lines, "; "),
                OLD_TEXT_BYTES,
            ))),
            new: None,
        });
    }
    rows
}

/// Delivery ids of the removed messages (positions of `stored`): they must stay in that chat's
/// duplicate filter so a retry of those deliveries cannot land again.
pub fn removed_delivery_ids(stored: &[ChatMessage], removed: &[usize]) -> Vec<String> {
    removed
        .iter()
        .filter_map(|index| delivery_id_of_message(&stored[*index]).map(str::to_string))
        .collect()
}

fn removed_usage(stored: &[ChatMessage], resolved: &Resolved) -> (usize, f64) {
    usage_of(
        stored,
        &resolved.removals.keys().copied().collect::<Vec<_>>(),
    )
}

fn usage_of(stored: &[ChatMessage], removed: &[usize]) -> (usize, f64) {
    removed
        .iter()
        .filter_map(|index| stored[*index].usage.as_ref())
        .fold((0, 0.0), |(tokens, usd), usage| {
            (
                tokens + usage.total_tokens,
                usd + usage
                    .metering_usd
                    .as_ref()
                    .map_or(0.0, |metering| metering.total_usd),
            )
        })
}

fn new_message_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn stamp(message: &mut ChatMessage, delivery_id: Option<String>, env: &PatchEnv) {
    let mut delivery = PendingDelivery::with_id(
        delivery_id.unwrap_or_else(new_message_id),
        vec![message.clone()],
        PushMode::WhenIdle,
        env.stamp_source,
        false,
    );
    delivery.enqueued_at_ms = env.now_ms;
    if let Some(stamped) = delivery.stamp_messages().pop() {
        *message = stamped;
    }
}

fn carry(message: &mut ChatMessage, op: usize, env: &PatchEnv) -> Result<(), PatchRefusal> {
    (env.carry)(message).map_err(|reason| {
        PatchRefusal::Refused(format!(
            "{}: the privacy records of this run cannot travel with the message: {reason}",
            op_label(op)
        ))
    })
}

fn text_content(op: usize, text: &str) -> Result<ChatContent, PatchRefusal> {
    validate_content_with_attachments(&Value::String(text.to_string()), &[])
        .map_err(|reason| PatchRefusal::Refused(format!("{}: {reason}", op_label(op))))
}

fn edited_message(
    original: &ChatMessage,
    edit: &Edit,
    env: &PatchEnv,
) -> Result<ChatMessage, PatchRefusal> {
    let mut message = original.clone();
    message.content = text_content(edit.op, &edit.text)?;
    if message.role == "tool" {
        for key in DROPPED_TOOL_KEYS {
            message.extra.shift_remove(key);
        }
    }
    if message.role == "user" {
        stamp(
            &mut message,
            delivery_id_of_message(original).map(str::to_string),
            env,
        );
    }
    carry(&mut message, edit.op, env)?;
    Ok(message)
}

fn inserted_messages(
    op: usize,
    patch_op: &PatchOp,
    env: &PatchEnv,
) -> Result<(Vec<ChatMessage>, InsertedMessage), PatchRefusal> {
    let PatchOp::Insert {
        role,
        text,
        tool_calls,
        ..
    } = patch_op
    else {
        unreachable!("inserts come from insert ops");
    };
    let mut head = ChatMessage {
        message_id: new_message_id(),
        role: role.as_str().into(),
        content: text_content(op, text)?,
        ..Default::default()
    };
    let mut results = Vec::new();
    let mut tool_call_ids = Vec::new();
    if let Some(calls) = tool_calls.as_ref().filter(|calls| !calls.is_empty()) {
        let mut declared = Vec::new();
        for call in calls {
            let id = generate_valid_tool_id();
            declared.push(ChatToolCall {
                id: id.clone(),
                index: None,
                function: ChatToolFunction {
                    arguments: Value::Object(call.arguments.clone()).to_string(),
                    name: call.name.clone(),
                },
                tool_type: "function".into(),
                extra_content: None,
                started_at_ms: None,
                completed_at_ms: None,
            });
            results.push(ChatMessage {
                message_id: new_message_id(),
                role: "tool".into(),
                content: text_content(op, &call.result)?,
                tool_call_id: id.clone(),
                tool_failed: Some(call.failed),
                ..Default::default()
            });
            tool_call_ids.push(id);
        }
        head.tool_calls = Some(declared);
    }
    if *role == InsertRole::User {
        stamp(&mut head, None, env);
    }
    let mut messages = vec![head];
    messages.extend(results);
    for message in &mut messages {
        carry(message, op, env)?;
    }
    let ids = InsertedMessage {
        op,
        message_id: messages[0].message_id.clone(),
        tool_call_ids,
        result_ids: messages[1..]
            .iter()
            .map(|message| message.message_id.clone())
            .collect(),
    };
    Ok((messages, ids))
}

fn build(
    stored: &[ChatMessage],
    resolved: &Resolved,
    ops: &[PatchOp],
    env: &PatchEnv,
) -> Result<Built, PatchRefusal> {
    let mut by_gap: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for insert in &resolved.inserts {
        by_gap.entry(insert.gap).or_default().push(insert.op);
    }
    let mut built = Built {
        messages: Vec::with_capacity(stored.len()),
        origins: Vec::with_capacity(stored.len()),
        inserted: Vec::new(),
    };
    for gap in 0..=stored.len() {
        for op in by_gap.get(&gap).into_iter().flatten() {
            let (messages, ids) = inserted_messages(*op, &ops[*op], env)?;
            for (k, message) in messages.into_iter().enumerate() {
                built.messages.push(message);
                built.origins.push(Origin::New(*op, k));
            }
            built.inserted.push(ids);
        }
        if gap == stored.len() || resolved.removals.contains_key(&gap) {
            continue;
        }
        let message = match resolved.edits.get(&gap) {
            Some(edit) if !edit.identical => edited_message(&stored[gap], edit, env)?,
            _ => stored[gap].clone(),
        };
        built.messages.push(message);
        built.origins.push(Origin::Base(gap));
    }
    built.inserted.sort_by_key(|ids| ids.op);
    Ok(built)
}

fn role_name(role: &str) -> &str {
    match role {
        "system"
        | "developer"
        | "user"
        | "assistant"
        | "tool"
        | "diff"
        | "context_file"
        | "plain_text"
        | "cd_instruction"
        | "error"
        | "summarization"
        | EVENT_ROLE
        | PLAN_ROLE
        | GOAL_ROLE
        | COMPRESSION_REPORT_ROLE => role,
        _ => "other role",
    }
}

fn describe(origin: Origin, message: &ChatMessage) -> String {
    let role = role_name(&message.role);
    match origin {
        Origin::Base(index) => format!("#{index} ({role})"),
        Origin::Payload(index) => format!("message {index} of the compressed head ({role})"),
        Origin::New(op, _) => format!("the {role} message inserted by {}", op_label(op)),
    }
}

fn flaw_reason(
    flaw: &Flaw,
    position: usize,
    view: &[(Origin, &ChatMessage)],
    base_view: &[(Origin, &ChatMessage)],
    base_analysis: &Analysis,
    resolved: &Resolved,
) -> String {
    let removed_by = |origin: &Origin| match origin {
        Origin::Base(index) => resolved.removals.get(index).map(|removal| removal.op()),
        _ => None,
    };
    let in_view = |view: &[(Origin, &ChatMessage)], origin: &Origin| {
        view.iter()
            .find(|(candidate, _)| candidate == origin)
            .map(|(origin, message)| describe(*origin, message))
            .unwrap_or_default()
    };
    let (origin, message) = view[position];
    let subject = describe(origin, message);
    match flaw {
        Flaw::Owner(result, raw, wire) => {
            let (base_raw, base_wire) = base_analysis
                .owners
                .get(result)
                .copied()
                .unwrap_or_default();
            let lost =
                (wire.is_none() && base_wire.is_some()) || (raw.is_none() && base_raw.is_some());
            let removed = base_wire
                .or(base_raw)
                .and_then(|head| removed_by(&head).map(|op| (op, in_view(base_view, &head))));
            let new_head = wire
                .or(*raw)
                .map(|head| in_view(view, &head))
                .unwrap_or_default();
            match (lost, removed) {
                (true, Some((op, head))) => format!(
                    "{}: removing {head} would leave its result {subject} without its call; pass with_results=True or remove {subject} as well",
                    op_label(op)
                ),
                (true, None) => format!("the patch would leave {subject} without the tool call it answers"),
                (false, Some((op, old))) => format!(
                    "{}: removing {old} would make its result {subject} answer the tool call of {new_head}, which reuses its call id; remove {subject} as well",
                    op_label(op)
                ),
                (false, None) => format!("the patch would make {subject} answer the tool call of {new_head}"),
            }
        }
        Flaw::Unanswered(_, call_id) => {
            let removed = base_view.iter().find_map(|(origin, message)| {
                (is_primary_result(message) && message.tool_call_id == *call_id)
                    .then(|| removed_by(origin).map(|op| (op, describe(*origin, message))))
                    .flatten()
            });
            match removed {
                Some((op, result)) => format!(
                    "{}: removing {result} would leave a tool call of {subject} without its result; remove {subject} with with_results=True instead",
                    op_label(op)
                ),
                None => format!("the patch would leave a tool call of {subject} without a result"),
            }
        }
        Flaw::Crossed(..) => {
            let inserted = view[position + 1..]
                .iter()
                .find(|(_, message)| reaches_wire(message) && is_separator(&message.role))
                .and_then(|(origin, _)| match origin {
                    Origin::New(op, _) => Some(*op),
                    _ => None,
                });
            match inserted {
                Some(op) => format!(
                    "{}: the insert would separate the tool calls of {subject} from their results; anchor it after the last result",
                    op_label(op)
                ),
                None => format!("the patch would separate the tool calls of {subject} from their results"),
            }
        }
        Flaw::Duplicated(..) => {
            format!("the patch would leave a tool call of {subject} with more than one result")
        }
        Flaw::Start(..) => format!("the patch would break the history at {subject}"),
    }
}

fn validate(
    start: usize,
    base_view: &[(Origin, &ChatMessage)],
    base_analysis: &Analysis,
    resolved: &Resolved,
    built: &Built,
) -> Result<(), PatchRefusal> {
    let invalid = |error: String| {
        PatchRefusal::Refused(format!("the patched history would be invalid: {error}"))
    };
    let candidate = active_context(&built.messages).map_err(|error| invalid(error.to_string()))?;
    if candidate.report_index.map_or(0, |index| index + 1) != start {
        return Err(PatchRefusal::Refused(
            "the patch would move the compression boundary".into(),
        ));
    }
    let view = view_of(&candidate, |index| built.origins[index]);
    let last_on_wire = view.iter().rev().find(|(_, message)| reaches_wire(message));
    if last_on_wire.is_none() && base_view.iter().any(|(_, message)| reaches_wire(message)) {
        return Err(PatchRefusal::Refused(
            "the patch would remove every message of that chat".into(),
        ));
    }
    let analysis = analyse(&view);
    let known = base_analysis.flaws();
    if let Some((position, flaw)) = analysis
        .ownership
        .iter()
        .chain(&analysis.pairing)
        .find(|(_, flaw)| !known.contains(flaw))
    {
        return Err(PatchRefusal::Refused(flaw_reason(
            flaw,
            *position,
            &view,
            base_view,
            base_analysis,
            resolved,
        )));
    }
    let wire = apply_summarization_linearize(candidate.messages.clone()).map_err(invalid)?;
    let kept = kept_before_prerun(wire.iter());
    if tool_prerun_possible(&wire[..kept]) {
        let (origin, message) = view
            .iter()
            .filter(|(_, message)| reaches_wire(message))
            .nth(kept - 1)
            .expect("the projection keeps the messages that reach the wire");
        return Err(PatchRefusal::Refused(format!(
            "the patched history would end with {}, whose tool calls would run at that chat's next turn",
            describe(*origin, message)
        )));
    }
    if pending_tool_result_window(&built.messages, start) {
        let index =
            latest_tool_call_window(&built.messages, start, None).map_or(0, |(index, _)| index);
        return Err(PatchRefusal::Refused(format!(
            "the patched history would leave the tool calls of {} without results, and that chat would wait for them",
            describe(built.origins[index], &built.messages[index])
        )));
    }
    if let Some(tail) = &analysis.tail {
        let kept = base_analysis.tail.as_ref().is_some_and(|base| {
            let members: HashSet<&(Origin, [u8; 32])> = base.members.iter().collect();
            base.head == tail.head && tail.members.iter().all(|member| members.contains(member))
        });
        if !kept {
            let (origin, message) = view[tail.position];
            return Err(PatchRefusal::Refused(format!(
                "the patched history would end inside the tool call loop of {}; end it with a user message or a plain assistant message",
                describe(origin, message)
            )));
        }
    }
    let anchored = |view: &[(Origin, &ChatMessage)]| {
        view.iter().any(|(_, message)| {
            reaches_wire(message)
                && matches!(
                    message.role.as_str(),
                    "system" | "user" | EVENT_ROLE | PLAN_ROLE | GOAL_ROLE
                )
        })
    };
    if !anchored(&view) && anchored(base_view) {
        return Err(PatchRefusal::Refused(
            "the patched history would hold no system, user, event, plan or goal message".into(),
        ));
    }
    if let Some((position, _)) = analysis
        .start
        .as_ref()
        .filter(|(_, flaw)| !known.contains(flaw))
    {
        let (origin, message) = view[*position];
        return Err(PatchRefusal::Refused(format!(
            "the patched history would start with {}; the first message must be a system, user, event, plan or goal message",
            describe(origin, message)
        )));
    }
    if let Some((index, op)) = resolved.system_edit {
        if view.first().map(|(origin, _)| *origin) != Some(Origin::Base(index)) {
            return Err(PatchRefusal::Refused(format!(
                "{}: the edited system message must be the first message of that chat; otherwise a default system prompt is added in front of it",
                op_label(op)
            )));
        }
    }
    if let Some((position, (Origin::New(op, _), _))) =
        view.iter().enumerate().find(|(_, (origin, message))| {
            matches!(origin, Origin::New(..)) && message.role == "system"
        })
    {
        if position != 0 {
            return Err(PatchRefusal::Refused(format!(
                "{}: a system message can only be inserted as the first message",
                op_label(*op)
            )));
        }
    }
    Ok(())
}

fn assemble(
    stored: &[ChatMessage],
    resolved: Resolved,
    items: Vec<PatchItem>,
    shown: Vec<usize>,
    built: Built,
) -> PatchPlan {
    let edited: Vec<usize> = resolved.changed_edits().map(|(index, _)| *index).collect();
    let removed: Vec<usize> = resolved.removals.keys().copied().collect();
    let cascades: Vec<Cascade> = resolved
        .removals
        .iter()
        .filter_map(|(index, removal)| match removal {
            Removal::Cascade {
                with, ..
            } => Some(Cascade {
                position: *index,
                role: role_name(&stored[*index].role).to_string(),
                with: *with,
            }),
            _ => None,
        })
        .collect();
    let inserted_tool_results: usize = built.inserted.iter().map(|ids| ids.result_ids.len()).sum();
    let inserted = built.inserted.len() + inserted_tool_results;
    let removed_delivery_ids = removed_delivery_ids(stored, &removed);

    let mut repairs = Vec::new();
    if resolved.frozen_system_prompt.is_some() {
        repairs.push(Repair::FrozenSystemPrompt);
    }
    for index in &edited {
        if stored[*index].role == "tool"
            && DROPPED_TOOL_KEYS
                .iter()
                .any(|key| stored[*index].extra.contains_key(*key))
        {
            repairs.push(Repair::ToolMetadataDropped {
                position: *index,
            });
        }
    }
    let (tokens, usd) = removed_usage(stored, &resolved);
    if tokens > 0 || usd > 0.0 {
        repairs.push(Repair::UsageRemoved {
            tokens,
            usd,
        });
    }
    let checkpoints: usize = removed
        .iter()
        .map(|index| stored[*index].checkpoints.len())
        .sum();
    if checkpoints > 0 {
        repairs.push(Repair::CheckpointsRemoved {
            count: checkpoints,
        });
    }
    if !removed_delivery_ids.is_empty() {
        repairs.push(Repair::DeliveryIdsRemoved {
            count: removed_delivery_ids.len(),
        });
    }

    let destructive = !edited.is_empty() || !removed.is_empty();
    let mut warning = String::new();
    if destructive {
        warning.push_str("Edited and removed messages of that chat cannot be restored.");
        if resolved.system_edit.is_some() {
            warning.push_str(" Its system prompt changes.");
        }
    }
    if inserted > 0 {
        if !warning.is_empty() {
            warning.push(' ');
        }
        warning.push_str("Inserted messages will look to that chat's model like real conversation and real tool results.");
    }
    let affected = edited.len() + removed.len() + inserted;
    PatchPlan {
        messages: built.messages,
        frozen_system_prompt: resolved.frozen_system_prompt,
        items,
        shown,
        class: if destructive {
            PythonEngineClass::Destructive
        } else {
            PythonEngineClass::Risky
        },
        warning,
        summary: PatchSummary {
            edited,
            removed,
            cascades,
            inserted,
            inserted_tool_results,
            repairs,
            affected,
        },
        inserted: built.inserted,
        removed_delivery_ids,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history_limit::{
        remove_invalid_tool_calls_and_tool_calls_results, validate_chat_history,
    };
    use refact_core::active_context::{make_reconstruction_report, ReconstructionMetadata};
    use refact_core::chat_types::{ChatUsage, Checkpoint, ContextFile, MeteringUsd, MultimodalElement};

    const SOURCE: &str = "engine.chats.patch";
    const NOW: u64 = 1_800_000_000_000;
    const TEXTS: [&str; 7] = [
        "SYSTEM_TEXT",
        "USER_ONE",
        "ASSISTANT_CALLS",
        "TOOL_ONE",
        "ASSISTANT_TWO",
        "USER_TWO",
        "ASSISTANT_THREE",
    ];

    fn msg(id: &str, role: &str, text: &str) -> ChatMessage {
        ChatMessage {
            message_id: id.into(),
            role: role.into(),
            content: ChatContent::SimpleText(text.into()),
            ..Default::default()
        }
    }

    fn call(id: &str) -> ChatToolCall {
        ChatToolCall {
            id: id.into(),
            index: None,
            function: ChatToolFunction {
                arguments: "{}".into(),
                name: "shell".into(),
            },
            tool_type: "function".into(),
            extra_content: None,
            started_at_ms: None,
            completed_at_ms: None,
        }
    }

    fn assistant(id: &str, text: &str, calls: &[&str]) -> ChatMessage {
        let mut message = msg(id, "assistant", text);
        if !calls.is_empty() {
            message.tool_calls = Some(calls.iter().map(|id| call(id)).collect());
        }
        message
    }

    fn result(id: &str, role: &str, call_id: &str, text: &str) -> ChatMessage {
        let mut message = msg(id, role, text);
        message.tool_call_id = call_id.into();
        message
    }

    fn context_result(id: &str, call_id: &str) -> ChatMessage {
        ChatMessage {
            message_id: id.into(),
            role: "context_file".into(),
            content: ChatContent::ContextFiles(vec![ContextFile {
                file_name: "src/a.rs".into(),
                file_content: "fn a() {}".into(),
                line1: 1,
                line2: 1,
                ..Default::default()
            }]),
            tool_call_id: call_id.into(),
            ..Default::default()
        }
    }

    /// T16: a result that never reaches the wire still belongs to its raw bundle: a patch that
    /// newly moves one across a separator is refused, while old damage elsewhere stays allowed.
    #[test]
    fn a_hidden_context_result_cannot_newly_cross_a_separator() {
        let mut hidden = context_result("cf", "c1");
        hidden.extra.insert("_ui_only".into(), json!(true));
        let stored = vec![
            msg("u1", "user", TEXTS[1]),
            assistant("a1", TEXTS[2], &["c1"]),
            result("t1", "tool", "c1", TEXTS[3]),
            hidden.clone(),
            msg("u2", "user", TEXTS[5]),
            msg("a3", "assistant", TEXTS[6]),
        ];
        let reason = refused(plan(
            &stored,
            json!([{"op": "insert", "after": "t1", "role": "user", "text": "BETWEEN"}]),
        ));
        assert!(
            reason.contains("op 1: the insert would separate the tool calls of #1 (assistant)"),
            "{reason}"
        );
        planned(plan(
            &stored,
            json!([{"op": "insert", "after": "cf", "role": "user", "text": "AFTER"}]),
        ));
        let mut damaged = stored.clone();
        damaged.swap(3, 4);
        planned(plan(
            &damaged,
            json!([{"op": "edit", "message_id": "a3", "text": "ELSEWHERE"}]),
        ));
    }

    /// #0 system, #1 user, #2 assistant calling c1, #3 its result, #4 answer, #5 user, #6 answer.
    fn chat() -> Vec<ChatMessage> {
        vec![
            msg("s", "system", TEXTS[0]),
            msg("u1", "user", TEXTS[1]),
            assistant("a1", TEXTS[2], &["c1"]),
            result("t1", "tool", "c1", TEXTS[3]),
            msg("a2", "assistant", TEXTS[4]),
            msg("u2", "user", TEXTS[5]),
            msg("a3", "assistant", TEXTS[6]),
        ]
    }

    /// Whole matches of `NAME_TOKEN=value` where the name starts the text or follows whitespace.
    fn test_spans(text: &str) -> Vec<Range<usize>> {
        let mut spans = Vec::new();
        let mut from = 0;
        while let Some(found) = text[from..].find("TOKEN=") {
            let at = from + found;
            let value = at + "TOKEN=".len();
            let name_start = text[..at]
                .char_indices()
                .rev()
                .find(|(_, c)| !(c.is_ascii_alphanumeric() || *c == '_'))
                .map_or(0, |(i, c)| i + c.len_utf8());
            let delimiter = text[..name_start].char_indices().next_back();
            let end = text[value..]
                .find(char::is_whitespace)
                .map_or(text.len(), |i| value + i);
            if end > value && delimiter.is_none_or(|(_, c)| c.is_whitespace()) {
                spans.push(delimiter.map_or(0, |(i, _)| i)..end);
            }
            from = value;
        }
        spans
    }

    fn carry_marker(message: &mut ChatMessage) -> Result<(), String> {
        message.extra.insert("carried".into(), json!(true));
        Ok(())
    }

    fn plan_env(
        stored: &[ChatMessage],
        ops: Value,
        frozen: FrozenSystem,
    ) -> Result<PatchOutcome, PatchRefusal> {
        let ops: Vec<PatchOp> = serde_json::from_value(ops).expect("ops parse");
        let env = PatchEnv {
            frozen,
            stamp_source: SOURCE,
            now_ms: NOW,
            carry: &carry_marker,
            secret_spans: &test_spans,
        };
        plan_history_patch(stored, &ops, &env)
    }

    fn plan(stored: &[ChatMessage], ops: Value) -> Result<PatchOutcome, PatchRefusal> {
        plan_env(stored, ops, FrozenSystem::Absent)
    }

    fn planned(outcome: Result<PatchOutcome, PatchRefusal>) -> PatchPlan {
        match outcome {
            Ok(PatchOutcome::Planned(plan)) => plan,
            other => panic!("expected a plan, got {other:?}"),
        }
    }

    fn refused(outcome: Result<PatchOutcome, PatchRefusal>) -> String {
        match outcome {
            Err(PatchRefusal::Refused(reason)) => reason,
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    fn invalid(outcome: Result<PatchOutcome, PatchRefusal>) -> String {
        match outcome {
            Err(PatchRefusal::Invalid(reason)) => reason,
            other => panic!("expected an invalid call, got {other:?}"),
        }
    }

    fn ids(messages: &[ChatMessage]) -> Vec<&str> {
        messages
            .iter()
            .map(|message| message.message_id.as_str())
            .collect()
    }

    fn texts(messages: &[ChatMessage]) -> Vec<String> {
        messages
            .iter()
            .map(|message| message.content.content_text_only())
            .collect()
    }

    fn bytes(message: &ChatMessage) -> Vec<u8> {
        serde_json::to_vec(message).unwrap()
    }

    fn labels(plan: &PatchPlan) -> Vec<&str> {
        plan.items.iter().map(|item| item.label.as_str()).collect()
    }

    fn calls_insert(after: &str, text: &str) -> Value {
        json!({"op": "insert", "after": after, "role": "assistant", "text": text,
            "tool_calls": [{"name": "shell", "arguments": {}, "result": "ok"}]})
    }

    #[test]
    fn edit_remove_insert_build_the_expected_vector() {
        let plan = planned(plan(
            &chat(),
            json!([
                {"op": "edit", "message_id": "u1", "text": "NEW_USER_ONE"},
                {"op": "remove", "message_id": "a2"},
                {"op": "insert", "after": "u2", "role": "user", "text": "INSERTED_ONE"},
                {"op": "insert", "after": "u2", "role": "assistant", "text": "INSERTED_TWO"},
                {"op": "insert", "after": "start", "role": "user", "text": "AT_START"},
                {"op": "insert", "after": "end", "role": "user", "text": "AT_END"},
            ]),
        ));
        assert_eq!(
            texts(&plan.messages),
            [
                "SYSTEM_TEXT",
                "AT_START",
                "NEW_USER_ONE",
                "ASSISTANT_CALLS",
                "TOOL_ONE",
                "USER_TWO",
                "INSERTED_ONE",
                "INSERTED_TWO",
                "ASSISTANT_THREE",
                "AT_END"
            ]
        );
        assert_eq!(plan.messages[2].message_id, "u1");
        let inserted: Vec<(usize, &str)> = plan
            .inserted
            .iter()
            .map(|ids| (ids.op, ids.message_id.as_str()))
            .collect();
        assert_eq!(
            inserted,
            [
                (2, plan.messages[6].message_id.as_str()),
                (3, plan.messages[7].message_id.as_str()),
                (4, plan.messages[1].message_id.as_str()),
                (5, plan.messages[9].message_id.as_str()),
            ]
        );
        assert_eq!(plan.summary.edited, [1]);
        assert_eq!(plan.summary.removed, [4]);
        assert_eq!(plan.summary.inserted, 4);
        assert_eq!(plan.summary.affected, 6);
        assert_eq!(plan.class, PythonEngineClass::Destructive);
        assert_eq!(
            labels(&plan),
            [
                "#1 user: edit",
                "#4 assistant: remove",
                "after #5: insert user",
                "after #5: insert assistant",
                "at the start: insert user",
                "at the end: insert user"
            ]
        );
    }

    #[test]
    fn inserted_calls_get_valid_ids_and_contiguous_results() {
        let plan = planned(plan(
            &chat(),
            json!([{"op": "insert", "after": "u2", "role": "assistant", "text": "LOOKING",
            "tool_calls": [
                {"name": "shell", "arguments": {"command": "ls"}, "result": "a.txt"},
                {"name": "cat", "arguments": {}, "result": "missing", "failed": true},
            ]}]),
        ));
        let at = 6;
        assert_eq!(plan.messages[at].content.content_text_only(), "LOOKING");
        let calls = plan.messages[at].tool_calls.clone().unwrap();
        assert_eq!(calls.len(), 2);
        for (k, call) in calls.iter().enumerate() {
            let hex = call.id.strip_prefix("call_").unwrap();
            assert!(
                hex.len() == 24 && hex.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')),
                "{}",
                call.id
            );
            let result = &plan.messages[at + 1 + k];
            assert_eq!(
                (result.role.as_str(), result.tool_call_id.as_str()),
                ("tool", call.id.as_str())
            );
        }
        assert_ne!(calls[0].id, calls[1].id);
        assert_eq!(calls[0].function.name, "shell");
        assert_eq!(calls[0].tool_type, "function");
        assert_eq!(
            serde_json::from_str::<Value>(&calls[0].function.arguments).unwrap(),
            json!({"command": "ls"})
        );
        assert_eq!(plan.messages[at + 1].tool_failed, Some(false));
        assert_eq!(plan.messages[at + 2].tool_failed, Some(true));
        assert_eq!(plan.messages[at + 2].content.content_text_only(), "missing");
        assert_eq!(plan.messages[at + 3].message_id, "a3");
        let ids = &plan.inserted[0];
        assert_eq!(
            ids.tool_call_ids,
            calls.iter().map(|call| call.id.clone()).collect::<Vec<_>>()
        );
        assert_eq!(
            ids.result_ids,
            [
                plan.messages[at + 1].message_id.clone(),
                plan.messages[at + 2].message_id.clone()
            ]
        );
        assert_eq!(plan.summary.inserted, 3);
        assert_eq!(plan.summary.inserted_tool_results, 2);
        assert_eq!(plan.class, PythonEngineClass::Risky);
        let mut sanitized = plan.messages.clone();
        remove_invalid_tool_calls_and_tool_calls_results(&mut sanitized);
        assert_eq!(
            serde_json::to_vec(&sanitized).unwrap(),
            serde_json::to_vec(&plan.messages).unwrap()
        );
        validate_chat_history(&plan.messages).unwrap();
    }

    #[test]
    fn removing_a_result_alone_is_refused() {
        let reason = refused(plan(&chat(), json!([{"op": "remove", "message_id": "t1"}])));
        assert!(
            reason.contains("op 1: removing #3 (tool) would leave a tool call of #2 (assistant)")
                && reason.contains("with_results=True"),
            "{reason}"
        );
        let plan = planned(plan(
            &chat(),
            json!([{"op": "remove", "message_id": "a1", "with_results": true}]),
        ));
        assert_eq!(plan.summary.removed, [2, 3]);
        assert_eq!(ids(&plan.messages), ["s", "u1", "a2", "u2", "a3"]);
    }

    #[test]
    fn removing_a_call_without_with_results_is_refused() {
        let stored = vec![
            msg("s", "system", "S"),
            msg("u1", "user", "U1"),
            assistant("a1", "", &["c1", "c2"]),
            result("t1", "tool", "c1", "T1"),
            context_result("f1", "c1"),
            result("d2", "diff", "c2", "D2"),
            msg("a2", "assistant", "A2"),
            msg("u2", "user", "U2"),
        ];
        let reason = refused(plan(&stored, json!([{"op": "remove", "message_id": "a1"}])));
        assert!(
            reason.contains("op 1: removing #2 (assistant) would leave its result #3 (tool)")
                && reason.contains("with_results=True"),
            "{reason}"
        );
        let plan = planned(plan(
            &stored,
            json!([{"op": "remove", "message_id": "a1", "with_results": true}]),
        ));
        assert_eq!(plan.summary.removed, [2, 3, 4, 5]);
        let cascades: Vec<(usize, &str, usize)> = plan
            .summary
            .cascades
            .iter()
            .map(|cascade| (cascade.position, cascade.role.as_str(), cascade.with))
            .collect();
        assert_eq!(
            cascades,
            [(3, "tool", 2), (4, "context_file", 2), (5, "diff", 2)]
        );
        assert_eq!(ids(&plan.messages), ["s", "u1", "a2", "u2"]);
        assert_eq!(
            labels(&plan),
            [
                "#2 assistant: remove",
                "#3 tool: removed with #2",
                "#4 context_file: removed with #2",
                "#5 diff: removed with #2"
            ]
        );
    }

    #[test]
    fn context_file_results_go_with_their_call_and_are_never_orphaned() {
        let mut attachment = context_result("pc", "project_context");
        attachment.content = ChatContent::ContextFiles(vec![]);
        let stored = vec![
            msg("s", "system", "S"),
            attachment,
            msg("u1", "user", "U1"),
            assistant("a1", "", &["c1"]),
            context_result("f1", "c1"),
            result("t1", "tool", "c1", "T1"),
            msg("a2", "assistant", "A2"),
        ];
        let reason = refused(plan(
            &stored,
            json!([
                {"op": "remove", "message_id": "a1"},
                {"op": "remove", "message_id": "t1"},
            ]),
        ));
        assert!(
            reason
                .contains("op 1: removing #3 (assistant) would leave its result #4 (context_file)"),
            "{reason}"
        );
        let plan1 = planned(plan(
            &stored,
            json!([{"op": "remove", "message_id": "a1", "with_results": true}]),
        ));
        assert_eq!(plan1.summary.removed, [3, 4, 5]);
        let plan2 = planned(plan(&stored, json!([{"op": "remove", "message_id": "f1"}])));
        assert_eq!(plan2.summary.removed, [4]);
        planned(plan(
            &stored,
            json!([{"op": "edit", "message_id": "u1", "text": "E"}]),
        ));
    }

    #[test]
    fn a_range_that_holds_a_call_and_all_its_results_needs_no_with_results() {
        let plan1 = planned(plan(
            &chat(),
            json!([{"op": "remove", "message_id": "a1", "through": "t1"}]),
        ));
        assert_eq!(plan1.summary.removed, [2, 3]);
        assert!(plan1.summary.cascades.is_empty());
        assert_eq!(labels(&plan1), ["#2–#3: remove 2 messages"]);
        let old = plan1.items[0].old.clone().unwrap();
        assert_eq!(old["roles"], json!({"assistant": 1, "tool": 1}));
        assert_eq!(old["first"], json!("ASSISTANT_CALLS\n[tool calls: shell]"));
        assert_eq!(old["last"], json!("TOOL_ONE"));
        let reason = refused(plan(
            &chat(),
            json!([{"op": "remove", "message_id": "u1", "through": "a1"}]),
        ));
        assert!(reason.contains("with_results=True"), "{reason}");
        let plan2 = planned(plan(
            &chat(),
            json!([{"op": "remove", "message_id": "u1", "through": "a1", "with_results": true}]),
        ));
        assert_eq!(plan2.summary.removed, [1, 2, 3]);
        assert_eq!(
            plan2.summary.cascades,
            [Cascade {
                position: 3,
                role: "tool".into(),
                with: 2
            }]
        );
    }

    #[test]
    fn range_bounds_are_checked() {
        let reason = invalid(plan(
            &chat(),
            json!([{"op": "remove", "message_id": "a2", "through": "u1"}]),
        ));
        assert!(
            reason.contains("through (#1) comes before message_id (#4)"),
            "{reason}"
        );
        let mut stored = vec![msg("s", "system", "S")];
        for i in 0..1001 {
            let role = if i % 2 == 0 { "user" } else { "assistant" };
            stored.push(msg(&format!("m{i}"), role, "x"));
        }
        let reason = refused(plan(
            &stored,
            json!([{"op": "remove", "message_id": "m0", "through": "m1000"}]),
        ));
        assert!(reason.contains("1001 messages"), "{reason}");
        let plan = planned(plan(
            &stored,
            json!([{"op": "remove", "message_id": "m1", "through": "m1000"}]),
        ));
        assert_eq!(ids(&plan.messages), ["s", "m0"]);
    }

    #[test]
    fn a_patch_never_leaves_inserted_calls_as_the_current_turn() {
        let reason = refused(plan(&chat(), json!([calls_insert("end", "")])));
        assert!(
            reason.contains(
                "would end inside the tool call loop of the assistant message inserted by op 1"
            ),
            "{reason}"
        );
        planned(plan(
            &chat(),
            json!([{"op": "insert", "after": "end", "role": "user", "text": "MORE"}]),
        ));
        planned(plan(
            &chat(),
            json!([{"op": "insert", "after": "end", "role": "assistant", "text": "PLAIN"}]),
        ));
        planned(plan(
            &chat(),
            json!([
                calls_insert("end", ""),
                {"op": "insert", "after": "end", "role": "assistant", "text": "DONE"},
            ]),
        ));
    }

    #[test]
    fn a_history_that_ends_in_tool_results_keeps_them_only_untouched() {
        let stored = chat()[..4].to_vec();
        planned(plan(
            &stored,
            json!([{"op": "edit", "message_id": "u1", "text": "EDITED"}]),
        ));
        let reason = refused(plan(
            &stored,
            json!([{"op": "edit", "message_id": "t1", "text": "EDITED"}]),
        ));
        assert!(
            reason.contains("would end inside the tool call loop of #2 (assistant)"),
            "{reason}"
        );
    }

    #[test]
    fn a_patch_never_makes_the_history_start_with_an_answer() {
        let no_system = chat()[1..].to_vec();
        let reason = refused(plan(
            &no_system,
            json!([{"op": "remove", "message_id": "u1"}]),
        ));
        assert!(
            reason.contains("the patched history would start with #1 (assistant)"),
            "{reason}"
        );
        let damaged = chat()[2..].to_vec();
        planned(plan(
            &damaged,
            json!([{"op": "edit", "message_id": "u2", "text": "E"}]),
        ));
        let repaired = planned(plan(
            &damaged,
            json!([{"op": "insert", "after": "start", "role": "user", "text": "HELLO"}]),
        ));
        assert_eq!(repaired.messages[0].content.content_text_only(), "HELLO");
    }

    #[test]
    fn removal_that_exposes_a_closed_bundle_is_refused() {
        let reason = refused(plan(
            &chat(),
            json!([{"op": "remove", "message_id": "a2", "through": "a3"}]),
        ));
        assert!(
            reason.contains("would end inside the tool call loop of #2 (assistant)"),
            "{reason}"
        );
    }

    #[test]
    fn old_damage_does_not_block_a_patch_elsewhere() {
        let mut stored = chat();
        stored.insert(2, result("orphan", "tool", "gone", "OLD_ORPHAN"));
        planned(plan(
            &stored,
            json!([{"op": "edit", "message_id": "u2", "text": "EDITED"}]),
        ));
        let reason = refused(plan(&stored, json!([{"op": "remove", "message_id": "a1"}])));
        assert!(
            reason.contains("op 1: removing #3 (assistant) would leave its result #4 (tool)"),
            "{reason}"
        );
        planned(plan(
            &stored,
            json!([{"op": "remove", "message_id": "orphan"}]),
        ));
    }

    #[test]
    fn an_old_dangling_call_stays_until_a_patch_exposes_it() {
        let stored = vec![
            msg("s", "system", "S"),
            msg("u0", "user", "U0"),
            assistant("a0", "", &["c0"]),
            msg("u1", "user", "U1"),
            assistant("a1", "", &["c1"]),
            result("t1", "tool", "c1", "T1"),
            msg("a2", "assistant", "A2"),
            msg("u2", "user", "U2"),
        ];
        planned(plan(
            &stored,
            json!([{"op": "edit", "message_id": "u2", "text": "EDITED"}]),
        ));
        let reason = refused(plan(
            &stored,
            json!([{"op": "remove", "message_id": "a1", "through": "a2"}]),
        ));
        assert!(
            reason.contains("leave the tool calls of #2 (assistant) without results"),
            "{reason}"
        );
        let reason = refused(plan(
            &stored,
            json!([{"op": "remove", "message_id": "u1", "through": "u2"}]),
        ));
        assert!(
            reason.contains("would end with #2 (assistant), whose tool calls would run"),
            "{reason}"
        );
    }

    #[test]
    fn a_dangling_call_in_the_compressed_head_is_never_exposed() {
        let report = make_reconstruction_report(
            vec![
                msg("s", "system", "S"),
                msg("u0", "user", "U0"),
                assistant("a0", "", &["c0"]),
            ],
            ReconstructionMetadata::default(),
        )
        .unwrap();
        let stored = vec![
            msg("old", "user", "ARCHIVED"),
            report,
            msg("u1", "user", "U1"),
        ];
        planned(plan(
            &stored,
            json!([{"op": "insert", "after": "end", "role": "assistant", "text": "A"}]),
        ));
        let reason = refused(plan(&stored, json!([{"op": "remove", "message_id": "u1"}])));
        assert!(
            reason.contains(
                "would end with message 2 of the compressed head (assistant), whose tool calls would run"
            ),
            "{reason}"
        );
        for ops in [
            json!([{"op": "edit", "message_id": "u0", "text": "X"}]),
            json!([{"op": "remove", "message_id": "old"}]),
            json!([{"op": "insert", "after": "a0", "role": "user", "text": "X"}]),
        ] {
            let reason = refused(plan(&stored, ops));
            assert!(reason.contains("inside compressed history"), "{reason}");
        }
        let reason = refused(plan(
            &stored,
            json!([{"op": "insert", "after": "start", "role": "user", "text": "X"}]),
        ));
        assert!(reason.contains("history is compressed"), "{reason}");
    }

    #[test]
    fn id_less_messages_are_no_targets_but_go_with_ranges_and_calls() {
        let mut stored = chat();
        stored[3].message_id = String::new();
        stored.insert(5, msg("", "user", "NO_ID"));
        let reason = invalid(plan(
            &stored,
            json!([{"op": "edit", "message_id": "", "text": "x"}]),
        ));
        assert!(reason.contains("message_id is empty"), "{reason}");
        let plan1 = planned(plan(
            &stored,
            json!([{"op": "remove", "message_id": "a1", "with_results": true}]),
        ));
        assert_eq!(plan1.summary.removed, [2, 3]);
        let plan2 = planned(plan(
            &stored,
            json!([{"op": "remove", "message_id": "a2", "through": "u2"}]),
        ));
        assert_eq!(plan2.summary.removed, [4, 5, 6]);
        assert_eq!(ids(&plan2.messages), ["s", "u1", "a1", "", "a3"]);
    }

    #[test]
    fn duplicate_and_dislocated_results() {
        let mut stored = chat();
        stored.insert(4, result("t1b", "tool", "c1", "AGAIN"));
        planned(plan(
            &stored,
            json!([{"op": "edit", "message_id": "u2", "text": "E"}]),
        ));
        let plan1 = planned(plan(
            &stored,
            json!([{"op": "remove", "message_id": "t1b"}]),
        ));
        assert_eq!(plan1.summary.removed, [4]);
        let plan2 = planned(plan(
            &stored,
            json!([{"op": "remove", "message_id": "a1", "with_results": true}]),
        ));
        assert_eq!(plan2.summary.removed, [2, 3, 4]);

        let dislocated = vec![
            msg("s", "system", "S"),
            msg("u1", "user", "U1"),
            assistant("a1", "", &["c1"]),
            msg("u2", "user", "U2"),
            result("t1", "tool", "c1", "T1"),
            msg("u3", "user", "U3"),
            assistant("a4", "", &["c4"]),
            result("t4", "tool", "c4", "T4"),
            msg("a5", "assistant", "A5"),
        ];
        planned(plan(
            &dislocated,
            json!([{"op": "edit", "message_id": "u3", "text": "E"}]),
        ));
        let plan3 = planned(plan(
            &dislocated,
            json!([{"op": "remove", "message_id": "a1", "with_results": true}]),
        ));
        assert_eq!(plan3.summary.removed, [2, 4]);

        let reason = refused(plan(
            &chat(),
            json!([{"op": "insert", "after": "a1", "role": "user", "text": "X"}]),
        ));
        assert!(
            reason.contains(
                "op 1: the insert would separate the tool calls of #2 (assistant) from their results"
            ),
            "{reason}"
        );
        let reason = refused(plan(&chat(), json!([calls_insert("a1", "")])));
        assert!(
            reason.contains("op 1: the insert would separate"),
            "{reason}"
        );
    }

    #[test]
    fn control_roles_compressed_history_and_legacy_histories_are_never_touched() {
        let mut stored = chat();
        stored.insert(5, msg("e1", "event", "EVENT"));
        stored.insert(6, msg("g1", "goal", "GOAL"));
        stored.insert(7, msg("p1", "plan", "PLAN"));
        let mut report = msg("r1", "compression_report", "REPORT");
        report.extra.insert(
            "compression_report".into(),
            json!({"kind": "chat_compression_report"}),
        );
        stored.insert(8, report);
        for id in ["e1", "g1", "p1", "r1"] {
            let reason = refused(plan(
                &stored,
                json!([{"op": "edit", "message_id": id, "text": "X"}]),
            ));
            assert!(reason.contains("cannot be edited"), "{id}: {reason}");
            let reason = refused(plan(&stored, json!([{"op": "remove", "message_id": id}])));
            assert!(reason.contains("cannot be removed"), "{id}: {reason}");
        }
        let reason = refused(plan(
            &stored,
            json!([{"op": "remove", "message_id": "a2", "through": "u2"}]),
        ));
        assert!(
            reason.contains("message #5 (event) cannot be removed"),
            "{reason}"
        );
        planned(plan(
            &stored,
            json!([{"op": "insert", "after": "e1", "role": "user", "text": "X"}]),
        ));

        let mut legacy = chat();
        legacy[4].summarized_range = Some((1, 3));
        assert_eq!(
            plan(
                &legacy,
                json!([{"op": "edit", "message_id": "u2", "text": "E"}])
            )
            .unwrap_err(),
            PatchRefusal::NeedsRebuild(
                "that chat's history needs an explicit rebuild; run chats.compact first".into()
            )
        );
    }

    #[test]
    fn only_plain_text_assistant_messages_are_editable() {
        let variants: [(&str, fn(&mut ChatMessage)); 6] = [
            ("thinking", |m| {
                m.thinking_blocks = Some(vec![json!({"type": "thinking", "signature": "SIG"})])
            }),
            ("server blocks", |m| {
                m.server_content_blocks = vec![json!({"type": "server_tool_use"})]
            }),
            ("citations", |m| {
                m.citations = vec![json!({"url": "https://x"})]
            }),
            ("reasoning", |m| m.reasoning_content = Some("why".into())),
            ("text blocks", |m| {
                m.extra.insert(
                    "_anthropic_text_blocks".into(),
                    json!([{"index": 0, "text": "x"}]),
                );
            }),
            ("provider parts", |m| {
                m.extra.insert("_google_cloud_code_parts".into(), json!([]));
            }),
        ];
        for (name, mutate) in variants {
            let mut stored = chat();
            mutate(&mut stored[4]);
            let reason = refused(plan(
                &stored,
                json!([{"op": "edit", "message_id": "a2", "text": "X"}]),
            ));
            assert!(
                reason.contains("only plain text assistant messages are editable"),
                "{name}: {reason}"
            );
            planned(plan(&stored, json!([{"op": "remove", "message_id": "a2"}])));
        }
        let reason = refused(plan(
            &chat(),
            json!([{"op": "edit", "message_id": "a1", "text": "X"}]),
        ));
        assert!(
            reason.contains("only plain text assistant messages are editable"),
            "{reason}"
        );

        let mut stored = chat();
        stored[4].usage = Some(ChatUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
            ..Default::default()
        });
        stored[4].finish_reason = Some("stop".into());
        let plan = planned(plan(
            &stored,
            json!([{"op": "edit", "message_id": "a2", "text": "NEW"}]),
        ));
        let mut expected = stored[4].clone();
        expected.content = ChatContent::SimpleText("NEW".into());
        expected.extra.insert("carried".into(), json!(true));
        assert_eq!(bytes(&plan.messages[4]), bytes(&expected));
    }

    #[test]
    fn special_messages_are_not_editable_and_plain_tool_edits_drop_stale_metadata() {
        let variants: [(&str, fn(&mut ChatMessage)); 9] = [
            ("diff", |m| m.role = "diff".into()),
            ("edit result", |m| {
                m.extra.insert("edit_result".into(), json!({"version": 1}));
            }),
            ("python", |m| {
                m.extra.insert("python".into(), json!({"run": 1}));
            }),
            ("privacy shell", |m| {
                m.extra.insert("privacy_shell".into(), json!({}));
            }),
            ("privacy observation", |m| {
                m.extra.insert("privacy_observation".into(), json!({}));
            }),
            ("privacy ref", |m| {
                m.extra.insert("privacy_ref".into(), json!("ref"));
            }),
            ("ui only", |m| {
                m.extra.insert("_ui_only".into(), json!(true));
            }),
            ("context file", |m| m.role = "context_file".into()),
            ("cd instruction", |m| m.role = "cd_instruction".into()),
        ];
        for (name, mutate) in variants {
            let mut stored = chat();
            mutate(&mut stored[3]);
            let reason = refused(plan(
                &stored,
                json!([{"op": "edit", "message_id": "t1", "text": "X"}]),
            ));
            assert!(reason.contains("#3"), "{name}: {reason}");
            assert!(reason.contains("cannot be edited"), "{name}: {reason}");
        }
        let mut stored = chat();
        stored[5].role = "error".into();
        let reason = refused(plan(
            &stored,
            json!([{"op": "edit", "message_id": "u2", "text": "X"}]),
        ));
        assert!(reason.contains("never shown to the model"), "{reason}");

        let mut stored = chat();
        stored[3]
            .extra
            .insert("tool_output".into(), json!({"kind": "x"}));
        stored[3]
            .extra
            .insert("tool_enrichment".into(), json!({"refs": []}));
        stored[3]
            .extra
            .insert("privacy".into(), json!({"files": []}));
        let plan = planned(plan(
            &stored,
            json!([{"op": "edit", "message_id": "t1", "text": "NEW_TOOL"}]),
        ));
        let edited = &plan.messages[3];
        assert_eq!(edited.content.content_text_only(), "NEW_TOOL");
        assert!(!edited.extra.contains_key("tool_output"));
        assert!(!edited.extra.contains_key("tool_enrichment"));
        assert!(edited.extra.contains_key("privacy"));
        assert_eq!(edited.tool_call_id, "c1");
        assert_eq!(
            plan.items[1],
            PatchItem {
                label: "#3 tool: stored tool output dropped".into(),
                old: Some(json!("tool_output, tool_enrichment")),
                new: None,
            }
        );
        assert_eq!(
            plan.summary.repairs,
            [Repair::ToolMetadataDropped {
                position: 3
            }]
        );
    }

    #[test]
    fn find_and_replace() {
        let mut stored = chat();
        stored[5] = msg("u2", "user", "alpha beta gamma aaa");
        let plan1 = planned(plan(
            &stored,
            json!([{"op": "edit", "message_id": "u2", "find": "beta", "replace": "BETA"}]),
        ));
        assert_eq!(
            plan1.messages[5].content.content_text_only(),
            "alpha BETA gamma aaa"
        );
        assert_eq!(
            plan1.items,
            [PatchItem {
                label: "#5 user: replace text".into(),
                old: Some(json!("beta")),
                new: Some(json!("BETA")),
            }]
        );
        for (find, expected) in [
            ("delta", "find occurs nowhere"),
            ("a ", "more than once"),
            ("aa", "more than once"),
        ] {
            let reason = refused(plan(
                &stored,
                json!([{"op": "edit", "message_id": "u2", "find": find, "replace": "X"}]),
            ));
            assert!(reason.contains(expected), "{find}: {reason}");
        }
        let reason = invalid(plan(
            &stored,
            json!([{"op": "edit", "message_id": "u2", "find": "", "replace": "X"}]),
        ));
        assert!(reason.contains("find must not be empty"), "{reason}");

        let mut split = chat();
        split[5].content = ChatContent::Multimodal(vec![
            MultimodalElement {
                m_type: "text".into(),
                m_content: "one".into(),
            },
            MultimodalElement {
                m_type: "text".into(),
                m_content: "two".into(),
            },
        ]);
        let reason = refused(plan(
            &split,
            json!([{"op": "edit", "message_id": "u2", "find": "one\n\ntwo", "replace": "X"}]),
        ));
        assert!(reason.contains("plain text messages only"), "{reason}");

        let mut splice = chat();
        splice[5] = msg("u2", "user", "export X_VAR abc now");
        let reason = refused(plan(
            &splice,
            json!([{"op": "edit", "message_id": "u2", "find": "VAR ", "replace": "TOKEN="}]),
        ));
        assert!(
            reason.contains("looks like a secret around the replaced fragment"),
            "{reason}"
        );
        let reason = refused(plan(
            &splice,
            json!([{"op": "edit", "message_id": "u2", "find": "abc", "replace": "TOKEN=xyz"}]),
        ));
        assert!(reason.contains("looks like a secret"), "{reason}");

        let mut anchored = chat();
        anchored[5] = msg("u2", "user", "prefix/TOKEN=alpha");
        let reason = refused(plan(
            &anchored,
            json!([{"op": "edit", "message_id": "u2", "find": "/", "replace": " "}]),
        ));
        assert!(
            reason.contains("looks like a secret around the replaced fragment"),
            "{reason}"
        );
        anchored[5] = msg("u2", "user", "prefix!TOKEN=alpha");
        planned(plan(
            &anchored,
            json!([{"op": "edit", "message_id": "u2", "find": "!", "replace": "/"}]),
        ));

        let mut value = chat();
        value[5] = msg("u2", "user", "X_TOKEN=aaaa");
        let reason = refused(plan(
            &value,
            json!([{"op": "edit", "message_id": "u2", "find": "aaaa", "replace": "bbbb"}]),
        ));
        assert!(
            reason.contains("looks like a secret around the replaced fragment"),
            "{reason}"
        );

        let mut after = chat();
        after[5] = msg("u2", "user", "some more X_TOKEN=abc");
        let shifted = planned(plan(
            &after,
            json!([{"op": "edit", "message_id": "u2", "find": "more", "replace": "much longer"}]),
        ));
        assert_eq!(
            shifted.messages[5].content.content_text_only(),
            "some much longer X_TOKEN=abc"
        );

        let mut existing = chat();
        existing[5] = msg("u2", "user", "X_TOKEN=abc and more text");
        let plan2 = planned(plan(
            &existing,
            json!([{"op": "edit", "message_id": "u2", "find": "more", "replace": "less"}]),
        ));
        assert_eq!(
            plan2.messages[5].content.content_text_only(),
            "X_TOKEN=abc and less text"
        );
    }

    #[test]
    fn a_full_text_edit_of_a_message_with_other_parts_is_refused() {
        let mut stored = chat();
        stored[5].content = ChatContent::Multimodal(vec![
            MultimodalElement {
                m_type: "text".into(),
                m_content: "look".into(),
            },
            MultimodalElement {
                m_type: "image/png".into(),
                m_content: "iVBORw0KGgo=".into(),
            },
        ]);
        let reason = refused(plan(
            &stored,
            json!([{"op": "edit", "message_id": "u2", "text": "X"}]),
        ));
        assert!(reason.contains("parts that are not text"), "{reason}");
        let mut text_parts = chat();
        text_parts[5].content = ChatContent::Multimodal(vec![
            MultimodalElement {
                m_type: "text".into(),
                m_content: "one".into(),
            },
            MultimodalElement {
                m_type: "text".into(),
                m_content: "two".into(),
            },
        ]);
        let plan = planned(plan(
            &text_parts,
            json!([{"op": "edit", "message_id": "u2", "text": "X"}]),
        ));
        assert!(matches!(&plan.messages[5].content, ChatContent::SimpleText(text) if text == "X"));
    }

    #[test]
    fn system_messages() {
        let reason = refused(plan(&chat(), json!([{"op": "remove", "message_id": "s"}])));
        assert!(
            reason.contains("cannot be removed: system and developer messages cannot be removed"),
            "{reason}"
        );
        let reason = refused(plan(
            &chat(),
            json!([{"op": "insert", "after": "start", "role": "system", "text": "X"}]),
        ));
        assert!(
            reason.contains("already has a system or developer message"),
            "{reason}"
        );

        let no_system = chat()[1..].to_vec();
        let reason = refused(plan(
            &no_system,
            json!([{"op": "insert", "after": "u1", "role": "system", "text": "X"}]),
        ));
        assert!(reason.contains("only be inserted at the start"), "{reason}");
        let plan1 = planned(plan(
            &no_system,
            json!([{"op": "insert", "after": "start", "role": "system", "text": "SYS"}]),
        ));
        assert_eq!(
            (
                plan1.messages[0].role.as_str(),
                plan1.messages[0].content.content_text_only()
            ),
            ("system", "SYS".to_string())
        );
        let reason = refused(plan(
            &no_system,
            json!([
                {"op": "insert", "after": "start", "role": "system", "text": "ONE"},
                {"op": "insert", "after": "start", "role": "system", "text": "TWO"},
            ]),
        ));
        assert!(
            reason.contains("already has a system or developer message"),
            "{reason}"
        );
        let reason = refused(plan(
            &no_system,
            json!([
                {"op": "insert", "after": "start", "role": "user", "text": "FIRST"},
                {"op": "insert", "after": "start", "role": "system", "text": "SYS"},
            ]),
        ));
        assert!(
            reason.contains("op 2: a system message can only be inserted as the first message"),
            "{reason}"
        );
        for frozen in [FrozenSystem::Complete("SYS"), FrozenSystem::Partial] {
            let reason = refused(plan_env(
                &no_system,
                json!([{"op": "insert", "after": "start", "role": "system", "text": "SYS"}]),
                frozen,
            ));
            assert!(reason.contains("frozen system prompt"), "{reason}");
        }

        let edit = json!([{"op": "edit", "message_id": "s", "text": "NEW_SYSTEM"}]);
        let synced = planned(plan_env(
            &chat(),
            edit.clone(),
            FrozenSystem::Complete("SYSTEM_TEXT"),
        ));
        assert_eq!(synced.frozen_system_prompt.as_deref(), Some("NEW_SYSTEM"));
        assert_eq!(synced.messages[0].content.content_text_only(), "NEW_SYSTEM");
        assert!(synced.items.contains(&PatchItem {
            label: "system prompt (frozen copy)".into(),
            old: Some(json!("SYSTEM_TEXT")),
            new: Some(json!("NEW_SYSTEM")),
        }));
        assert!(synced.warning.contains("Its system prompt changes."));
        assert!(synced.summary.repairs.contains(&Repair::FrozenSystemPrompt));
        let unfrozen = planned(plan(&chat(), edit.clone()));
        assert_eq!(unfrozen.frozen_system_prompt, None);
        assert_eq!(labels(&unfrozen), ["#0 system: edit"]);
        let reason = refused(plan_env(&chat(), edit.clone(), FrozenSystem::Partial));
        assert!(
            reason.contains("frozen request prefix is incomplete"),
            "{reason}"
        );
        let reason = refused(plan_env(&chat(), edit, FrozenSystem::Complete("OTHER")));
        assert!(
            reason.contains("differs from its system message"),
            "{reason}"
        );

        let mut two = chat();
        two.insert(1, msg("s2", "system", "SECOND"));
        let reason = refused(plan(
            &two,
            json!([{"op": "edit", "message_id": "s2", "text": "X"}]),
        ));
        assert!(
            reason.contains("that chat has other system or developer messages"),
            "{reason}"
        );
    }

    #[test]
    fn op_count_limit() {
        let ops = |n: usize| {
            Value::Array(
                (0..n)
                    .map(|i| json!({"op": "insert", "after": "end", "role": "user", "text": format!("m{i}")}))
                    .collect(),
            )
        };
        let reason = invalid(plan(&chat(), ops(51)));
        assert!(
            reason.contains("51 operations in one call; the limit is 50"),
            "{reason}"
        );
        planned(plan(&chat(), ops(50)));
        let reason = invalid(plan(&chat(), json!([])));
        assert!(reason.contains("at least one operation"), "{reason}");
    }

    #[test]
    fn text_size_limits() {
        let insert =
            |text: String| json!({"op": "insert", "after": "end", "role": "user", "text": text});
        planned(plan(
            &chat(),
            json!([insert("x".repeat(MAX_TEXT_BYTES - 2))]),
        ));
        let reason = refused(plan(
            &chat(),
            json!([insert("x".repeat(MAX_TEXT_BYTES - 1))]),
        ));
        assert!(
            reason.contains("op 1: the new text is 16385 bytes; the limit is 16384 bytes"),
            "{reason}"
        );
        let reason = refused(plan(
            &chat(),
            json!([{"op": "edit", "message_id": "u2", "find": "x".repeat(MAX_TEXT_BYTES), "replace": "y"}]),
        ));
        assert!(reason.contains("find is 16386 bytes"), "{reason}");
        let reason = refused(plan(
            &chat(),
            json!([{"op": "insert", "after": "u2", "role": "assistant", "text": "",
                "tool_calls": [{"name": "shell", "arguments": {"a": "z".repeat(9000)}, "result": "r".repeat(9000)}]}]),
        ));
        assert!(
            reason.contains("a tool call with its arguments and result is"),
            "{reason}"
        );
        let text = "y".repeat(15 * 1024);
        planned(plan(&chat(), Value::Array(vec![insert(text.clone()); 4])));
        let reason = refused(plan(&chat(), Value::Array(vec![insert(text); 5])));
        assert!(
            reason.contains("the new text of this call is over 65536 bytes"),
            "{reason}"
        );
    }

    #[test]
    fn tool_call_count_limit() {
        let calls: Vec<Value> = (0..17)
            .map(|_| json!({"name": "shell", "arguments": {}, "result": "ok"}))
            .collect();
        let reason = refused(plan(
            &chat(),
            json!([{"op": "insert", "after": "u2", "role": "assistant", "text": "", "tool_calls": calls}]),
        ));
        assert!(
            reason.contains("17 tool calls in one message; the limit is 16"),
            "{reason}"
        );
    }

    #[test]
    fn affected_message_limit() {
        let mut stored = vec![msg("s", "system", "S")];
        for i in 0..2100 {
            let role = if i % 2 == 0 { "user" } else { "assistant" };
            stored.push(msg(&format!("m{i}"), role, "x"));
        }
        let ranges = json!([
            {"op": "remove", "message_id": "m1", "through": "m1000"},
            {"op": "remove", "message_id": "m1001", "through": "m2000"},
        ]);
        let plan1 = planned(plan(&stored, ranges.clone()));
        assert_eq!(plan1.summary.affected, 2000);
        let mut over = ranges.as_array().unwrap().clone();
        over.push(json!({"op": "remove", "message_id": "m2001"}));
        let reason = refused(plan(&stored, Value::Array(over)));
        assert!(
            reason.contains("the patch affects 2001 messages; the limit is 2000"),
            "{reason}"
        );
    }

    #[test]
    fn card_size_limit() {
        let mut stored = vec![msg("s", "system", "S")];
        let mut ops = Vec::new();
        for i in 0..40 {
            stored.push(msg(&format!("u{i}"), "user", "U"));
            let calls: Vec<String> = (0..15).map(|k| format!("c{i}_{k}")).collect();
            let call_refs: Vec<&str> = calls.iter().map(String::as_str).collect();
            stored.push(assistant(&format!("a{i}"), &"q".repeat(5000), &call_refs));
            for call_id in &calls {
                stored.push(result(
                    &format!("t{call_id}"),
                    "tool",
                    call_id,
                    &"r".repeat(400),
                ));
            }
            stored.push(msg(&format!("z{i}"), "assistant", "Z"));
            ops.push(json!({"op": "remove", "message_id": format!("a{i}"), "with_results": true}));
        }
        stored.push(msg("last", "user", "L"));
        let reason = refused(plan(&stored, Value::Array(ops.clone())));
        assert!(
            reason.contains("the approval card would be over 262144 bytes"),
            "{reason}"
        );
        planned(plan(&stored, Value::Array(ops[..20].to_vec())));
    }

    #[test]
    fn history_size_limit() {
        let mut stored = chat();
        stored[5] = msg("u2", "user", &"h".repeat(MAX_HISTORY_BYTES));
        let reason = refused(plan(
            &stored,
            json!([{"op": "edit", "message_id": "u1", "text": "E"}]),
        ));
        assert!(reason.contains("over 64 MiB; compact it first"), "{reason}");
    }

    #[test]
    fn user_messages_the_patch_writes_carry_the_script_stamp() {
        let mut stored = chat();
        stored[5].extra.insert(
            "delivery".into(),
            json!({"id": "d-1", "source": "scheduler", "push": "append", "at_ms": 1}),
        );
        let plan = planned(plan(
            &stored,
            json!([
                {"op": "edit", "message_id": "s", "text": "S1"},
                {"op": "edit", "message_id": "u1", "text": "E1"},
                {"op": "edit", "message_id": "u2", "find": "TWO", "replace": "2"},
                {"op": "edit", "message_id": "a2", "text": "A2"},
                {"op": "insert", "after": "end", "role": "user", "text": "NEW"},
                {"op": "insert", "after": "u2", "role": "assistant", "text": "A"},
            ]),
        ));
        let stamp = |message: &ChatMessage| message.extra.get("delivery").cloned();
        let edited = stamp(&plan.messages[1]).unwrap();
        assert_eq!(edited["source"], json!(SOURCE));
        assert_eq!(edited["at_ms"], json!(NOW));
        assert_eq!(edited["push"], json!("when_idle"));
        assert!(edited["id"].as_str().is_some_and(|id| !id.is_empty()));
        let kept = stamp(&plan.messages[5]).unwrap();
        assert_eq!(kept["id"], json!("d-1"));
        assert_eq!(kept["source"], json!(SOURCE));
        assert_eq!(delivery_id_of_message(&plan.messages[5]), Some("d-1"));
        let inserted = stamp(plan.messages.last().unwrap()).unwrap();
        assert_eq!(inserted["source"], json!(SOURCE));
        assert_ne!(inserted["id"], edited["id"]);
        assert!(stamp(&plan.messages[4]).is_none());
        assert!(stamp(&plan.messages[6]).is_none());
        assert!(stamp(&plan.messages[0]).is_none());
    }

    #[test]
    fn untouched_messages_are_byte_identical() {
        let mut stored = chat();
        stored[2].thinking_blocks = Some(vec![
            json!({"type": "thinking", "thinking": "T", "signature": "SIG"}),
        ]);
        stored[2].usage = Some(ChatUsage {
            total_tokens: 7,
            ..Default::default()
        });
        stored[3]
            .extra
            .insert("tool_output".into(), json!({"k": "v"}));
        stored[1].checkpoints = vec![Checkpoint {
            workspace_folder: "/w".into(),
            commit_hash: "abc".into(),
        }];
        stored[4].extra.insert(
            "_anthropic_text_blocks".into(),
            json!([{"index": 0, "text": "x"}]),
        );
        let plan = planned(plan(
            &stored,
            json!([
                {"op": "edit", "message_id": "u2", "text": "E"},
                {"op": "remove", "message_id": "a2"},
                {"op": "insert", "after": "a3", "role": "user", "text": "N"},
            ]),
        ));
        assert_eq!(plan.messages.len(), stored.len());
        for message in stored
            .iter()
            .filter(|message| !["u2", "a2"].contains(&message.message_id.as_str()))
        {
            let candidate = plan
                .messages
                .iter()
                .find(|candidate| candidate.message_id == message.message_id)
                .unwrap();
            assert_eq!(bytes(candidate), bytes(message), "{}", message.message_id);
        }
    }

    #[test]
    fn class_warning_and_items() {
        let risky = planned(plan(
            &chat(),
            json!([
                {"op": "insert", "after": "u2", "role": "user", "text": "X"},
                calls_insert("u2", "Y"),
            ]),
        ));
        assert_eq!(risky.class, PythonEngineClass::Risky);
        assert_eq!(
            risky.warning,
            "Inserted messages will look to that chat's model like real conversation and real tool results."
        );
        assert_eq!(
            risky.items,
            [
                PatchItem {
                    label: "after #5: insert user".into(),
                    old: None,
                    new: Some(json!("X")),
                },
                PatchItem {
                    label: "after #5: insert assistant".into(),
                    old: None,
                    new: Some(json!("Y")),
                },
                PatchItem {
                    label: "after #5: insert call shell".into(),
                    old: None,
                    new: Some(json!({"arguments": {}, "result": "ok", "failed": false})),
                },
            ]
        );

        let mut stored = chat();
        stored[5] = msg("u2", "user", &"é".repeat(5000));
        let destructive = planned(plan(
            &stored,
            json!([{"op": "edit", "message_id": "u2", "text": "short"}]),
        ));
        assert_eq!(destructive.class, PythonEngineClass::Destructive);
        assert_eq!(
            destructive.warning,
            "Edited and removed messages of that chat cannot be restored."
        );
        let old = destructive.items[0].old.as_ref().unwrap().as_str().unwrap();
        assert!(
            old.len() <= OLD_TEXT_BYTES && old.ends_with('…'),
            "{}",
            old.len()
        );
        assert_eq!(destructive.items[0].new, Some(json!("short")));

        let both = planned(plan(
            &chat(),
            json!([
                {"op": "remove", "message_id": "a3"},
                {"op": "insert", "after": "end", "role": "user", "text": "X"},
            ]),
        ));
        assert_eq!(
            both.warning,
            "Edited and removed messages of that chat cannot be restored. Inserted messages will look to that chat's model like real conversation and real tool results."
        );
    }

    #[test]
    fn the_card_names_every_position_whose_text_it_shows() {
        let mut stored = chat();
        stored.insert(5, msg("m", "assistant", "MIDDLE"));
        let shown = planned(plan(
            &stored,
            json!([
                {"op": "edit", "message_id": "u1", "find": "ONE", "replace": "1"},
                {"op": "remove", "message_id": "a1", "with_results": true},
                {"op": "remove", "message_id": "a2", "through": "u2"},
                {"op": "insert", "after": "end", "role": "user", "text": "X"},
            ]),
        ));
        assert_eq!(shown.shown, [1, 2, 3, 4, 6]);
        let single = planned(plan(
            &chat(),
            json!([{"op": "remove", "message_id": "u2", "through": "u2"}]),
        ));
        assert_eq!(labels(&single), ["#5 user: remove"]);
        assert_eq!(single.shown, [5]);
        assert_eq!(single.summary.removed, [5]);
    }

    #[test]
    fn summaries_and_refusals_hold_no_message_text() {
        let stored = chat();
        let patch = planned(plan(
            &stored,
            json!([
                {"op": "edit", "message_id": "u1", "text": "NEW_TEXT_X"},
                {"op": "remove", "message_id": "a1", "with_results": true},
                {"op": "insert", "after": "end", "role": "user", "text": "INSERTED_X"},
            ]),
        ));
        let summary = format!(
            "{}{:?}{:?}{:?}{}{:?}",
            serde_json::to_string(&patch.summary).unwrap(),
            patch.summary,
            patch.inserted,
            patch.removed_delivery_ids,
            patch.warning,
            patch
        );
        for text in TEXTS.iter().chain(&["NEW_TEXT_X", "INSERTED_X"]) {
            assert!(!summary.contains(text), "{text} in {summary}");
        }
        for ops in [
            json!([{"op": "remove", "message_id": "t1"}]),
            json!([{"op": "remove", "message_id": "a1"}]),
            json!([{"op": "edit", "message_id": "a1", "text": "X"}]),
            json!([{"op": "edit", "message_id": "u2", "find": "ABSENT", "replace": "X"}]),
            json!([{"op": "insert", "after": "a1", "role": "user", "text": "X"}]),
            json!([{"op": "remove", "message_id": "a2", "through": "a3"}]),
        ] {
            let reason = plan(&stored, ops).unwrap_err();
            for text in TEXTS {
                assert!(!reason.reason().contains(text), "{text} in {reason:?}");
            }
        }
    }

    #[test]
    fn an_identical_edit_is_nothing_to_change() {
        for ops in [
            json!([{"op": "edit", "message_id": "u1", "text": "USER_ONE"}]),
            json!([{"op": "edit", "message_id": "u1", "find": "ONE", "replace": "ONE"}]),
        ] {
            assert!(matches!(
                plan(&chat(), ops),
                Ok(PatchOutcome::NothingToChange)
            ));
        }
        let plan = planned(plan(
            &chat(),
            json!([
                {"op": "edit", "message_id": "u1", "text": "USER_ONE"},
                {"op": "edit", "message_id": "u2", "text": "E"},
            ]),
        ));
        assert_eq!(plan.summary.edited, [5]);
        assert_eq!(labels(&plan), ["#5 user: edit"]);
        assert_eq!(bytes(&plan.messages[1]), bytes(&chat()[1]));
    }

    #[test]
    fn malformed_ops_and_conflicting_targets_are_invalid() {
        for (ops, expected) in [
            (
                json!([
                    {"op": "edit", "message_id": "u1", "text": "X"},
                    {"op": "remove", "message_id": "u1"},
                ]),
                "op 2: message #1 is already changed by op 1",
            ),
            (
                json!([
                    {"op": "remove", "message_id": "u1", "through": "a1", "with_results": true},
                    {"op": "edit", "message_id": "t1", "text": "X"},
                ]),
                "message #3 is removed with its call and edited by op 2",
            ),
            (
                json!([
                    {"op": "remove", "message_id": "a2"},
                    {"op": "insert", "after": "a2", "role": "user", "text": "X"},
                ]),
                "op 2: its anchor #4 is removed by op 1",
            ),
            (
                json!([
                    {"op": "insert", "after": "a2", "role": "user", "text": "X"},
                    {"op": "remove", "message_id": "a2"},
                ]),
                "op 1: its anchor #4 is removed by op 2",
            ),
            (
                json!([{"op": "edit", "message_id": "nope", "text": "X"}]),
                "op 1: message nope is not in that chat",
            ),
            (
                json!([{"op": "edit", "message_id": "u1", "text": "X", "find": "a", "replace": "b"}]),
                "an edit takes either text, or find and replace",
            ),
            (
                json!([{"op": "insert", "after": "end", "role": "user", "text": "X",
                    "tool_calls": [{"name": "shell", "arguments": {}, "result": "ok"}]}]),
                "only an assistant message can carry tool calls",
            ),
            (
                json!([{"op": "insert", "after": "end", "role": "user", "text": "  "}]),
                "an inserted user message needs text",
            ),
            (
                json!([{"op": "insert", "after": "end", "role": "assistant", "text": "",
                    "tool_calls": [{"name": "bad name", "arguments": {}, "result": "ok"}]}]),
                "tool call 1 needs a name",
            ),
        ] {
            let reason = invalid(plan(&chat(), ops));
            assert!(reason.contains(expected), "{expected}: {reason}");
        }
    }

    #[test]
    fn new_ids_are_fresh_and_ops_cannot_carry_ids_or_extra() {
        let base: HashSet<String> = chat()
            .into_iter()
            .map(|message| message.message_id)
            .collect();
        let plan = planned(plan(
            &chat(),
            json!([
                calls_insert("u2", "A"),
                {"op": "insert", "after": "end", "role": "user", "text": "U"},
            ]),
        ));
        let mut seen = HashSet::new();
        for message in &plan.messages {
            assert!(seen.insert(message.message_id.clone()), "duplicate id");
            if !base.contains(&message.message_id) {
                assert!(uuid::Uuid::parse_str(&message.message_id).is_ok());
            }
        }
        assert_eq!(seen.len(), base.len() + 3);
        for bad in [
            json!({"op": "insert", "after": "end", "role": "user", "text": "X", "message_id": "m"}),
            json!({"op": "insert", "after": "end", "role": "user", "text": "X", "extra": {}}),
            json!({"op": "insert", "after": "end", "role": "tool", "text": "X"}),
            json!({"op": "insert", "after": "end", "role": "assistant", "text": "",
                "tool_calls": [{"id": "c", "name": "shell", "arguments": {}, "result": "ok"}]}),
            json!({"op": "edit", "message_id": "u1", "text": "X", "role": "user"}),
            json!({"op": "remove", "message_id": "u1", "tool_call_id": "c1"}),
        ] {
            assert!(
                serde_json::from_value::<PatchOp>(bad.clone()).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn run_records_are_carried_into_every_new_and_edited_message() {
        let plan = planned(plan(
            &chat(),
            json!([
                {"op": "edit", "message_id": "u1", "text": "E"},
                calls_insert("u2", "A"),
                {"op": "insert", "after": "end", "role": "user", "text": "U"},
            ]),
        ));
        let base: HashSet<String> = chat()
            .into_iter()
            .map(|message| message.message_id)
            .collect();
        for message in &plan.messages {
            let carried = message.extra.get("carried") == Some(&json!(true));
            let changed = message.message_id == "u1" || !base.contains(&message.message_id);
            assert_eq!(carried, changed, "{}", message.message_id);
        }
        let failing = |_: &mut ChatMessage| -> Result<(), String> {
            Err("a record is unreadable".to_string())
        };
        let ops: Vec<PatchOp> =
            serde_json::from_value(json!([{"op": "edit", "message_id": "u1", "text": "E"}]))
                .unwrap();
        let env = PatchEnv {
            frozen: FrozenSystem::Absent,
            stamp_source: SOURCE,
            now_ms: NOW,
            carry: &failing,
            secret_spans: &test_spans,
        };
        let reason = refused(plan_history_patch(&chat(), &ops, &env));
        assert!(reason.contains("a record is unreadable"), "{reason}");
    }

    #[test]
    fn removal_cards_show_usage_and_checkpoints_and_keep_delivery_ids() {
        let mut stored = chat();
        stored[2].usage = Some(ChatUsage {
            total_tokens: 1200,
            metering_usd: Some(MeteringUsd {
                total_usd: 0.25,
                ..Default::default()
            }),
            ..Default::default()
        });
        stored[4].usage = Some(ChatUsage {
            total_tokens: 300,
            ..Default::default()
        });
        stored[5].checkpoints = vec![Checkpoint {
            workspace_folder: "/w".into(),
            commit_hash: "0123456789abcdef".into(),
        }];
        stored[5].extra.insert(
            "delivery".into(),
            json!({"id": "d-7", "source": "scheduler", "push": "append", "at_ms": 1}),
        );
        let plan = planned(plan(
            &stored,
            json!([
                {"op": "remove", "message_id": "a1", "with_results": true},
                {"op": "remove", "message_id": "a2", "through": "u2"},
            ]),
        ));
        assert_eq!(plan.summary.removed, [2, 3, 4, 5]);
        assert!(plan.items.contains(&PatchItem {
            label: "recorded usage removed".into(),
            old: Some(json!("1500 tokens, $0.2500")),
            new: None,
        }));
        assert!(plan.items.contains(&PatchItem {
            label: "file checkpoints removed".into(),
            old: Some(json!("#5: 0123456789ab")),
            new: None,
        }));
        assert_eq!(plan.removed_delivery_ids, ["d-7"]);
        assert_eq!(
            plan.summary.repairs,
            [
                Repair::UsageRemoved {
                    tokens: 1500,
                    usd: 0.25
                },
                Repair::CheckpointsRemoved {
                    count: 1
                },
                Repair::DeliveryIdsRemoved {
                    count: 1
                },
            ]
        );
    }

    #[test]
    fn tool_prerun_calls_follow_the_prerun_rule() {
        let run = |messages: &[ChatMessage], allowed: &[&str]| -> Vec<String> {
            tool_prerun_calls(messages, |name| allowed.contains(&name))
                .into_iter()
                .map(|call| call.id)
                .collect()
        };
        let mut calling = assistant("a", "", &["c1", "c2"]);
        calling.tool_calls.as_mut().unwrap()[1].function.name = "cat".into();
        let open = vec![msg("u", "user", "U"), calling];
        assert_eq!(run(&open, &["shell", "cat"]), ["c1", "c2"]);
        assert!(run(&open, &["shell"]).is_empty());
        let mut answered = vec![result("t", "tool", "c2", "T")];
        answered.extend(open.clone());
        assert_eq!(run(&answered, &["shell"]), ["c1"]);
        let mut by_diff = vec![result("d", "diff", "c2", "D")];
        by_diff.extend(open.clone());
        assert_eq!(run(&by_diff, &["shell"]), ["c1"]);
        let mut by_context = vec![context_result("f", "c2")];
        by_context.extend(open.clone());
        assert!(run(&by_context, &["shell"]).is_empty());
        let mut closed = open.clone();
        closed.push(result("t1", "tool", "c1", "T"));
        assert!(run(&closed, &["shell", "cat"]).is_empty());
        let mut later = open.clone();
        later.push(msg("u2", "user", "U2"));
        assert!(run(&later, &["shell", "cat"]).is_empty());
        assert!(run(&[], &["shell"]).is_empty());
    }

    #[test]
    fn pending_tool_result_window_follows_the_session_rule() {
        assert!(!pending_tool_result_window(&chat(), 0));
        let open = vec![
            msg("s", "system", "S"),
            msg("u1", "user", "U1"),
            assistant("a1", "", &["c1"]),
        ];
        assert!(pending_tool_result_window(&open, 0));
        assert!(!pending_tool_result_window(&open, 3));
        let mut crossed = open.clone();
        crossed.push(msg("u2", "user", "U2"));
        crossed.push(result("t1", "tool", "c1", "T1"));
        assert!(pending_tool_result_window(&crossed, 0));
        let mut through_error = open.clone();
        through_error.push(msg("x", "error", "E"));
        through_error.push(context_result("f1", "c1"));
        through_error.push(result("t1", "tool", "c1", "T1"));
        assert!(!pending_tool_result_window(&through_error, 0));
        let mut after_event = open.clone();
        after_event.push(msg("e", "event", "E"));
        after_event.push(result("t1", "tool", "c1", "T1"));
        assert!(pending_tool_result_window(&after_event, 0));
        assert_eq!(
            latest_tool_call_window(&chat(), 0, Some("c1")),
            Some((2, vec!["c1".to_string()]))
        );
        assert_eq!(latest_tool_call_window(&chat(), 0, Some("zz")), None);
        assert_eq!(latest_tool_call_window(&chat(), 3, None), None);
        assert_eq!(
            tool_results_after(&chat(), 2),
            HashSet::from(["c1".to_string()])
        );
    }

    fn paused_event(id: &str) -> ChatMessage {
        let mut event = msg(id, "event", "PAUSED");
        event.extra.insert(
            "event".into(),
            json!({"subkind": "goal_pursuit", "source": "goal", "payload": {"kind": "paused"}}),
        );
        event
    }

    fn hidden(mut message: ChatMessage) -> ChatMessage {
        message.extra.insert("_ui_only".into(), json!(true));
        message
    }

    fn on_view(messages: &[ChatMessage]) -> Vec<Flaw> {
        let view: Vec<(Origin, &ChatMessage)> = messages
            .iter()
            .enumerate()
            .map(|(index, message)| (Origin::Base(index), message))
            .collect();
        analyse(&view)
            .pairing
            .into_iter()
            .map(|(_, flaw)| flaw)
            .collect()
    }

    #[test]
    fn a_paused_goal_event_never_hides_an_unanswered_call() {
        let report = make_reconstruction_report(
            vec![
                msg("s", "system", "S"),
                msg("u0", "user", "U0"),
                assistant("a0", "", &["c0"]),
            ],
            ReconstructionMetadata::default(),
        )
        .unwrap();
        let compressed = vec![report, paused_event("e"), msg("u1", "user", "U1")];
        let reason = refused(plan(
            &compressed,
            json!([{"op": "remove", "message_id": "u1"}]),
        ));
        assert!(
            reason.contains(
                "would end with message 2 of the compressed head (assistant), whose tool calls would run"
            ),
            "{reason}"
        );
        let raw = vec![
            msg("s", "system", "S"),
            msg("u1", "user", "U1"),
            assistant("a1", "", &["c1"]),
            hidden(result("h1", "tool", "c1", "H1")),
            paused_event("e"),
            msg("u2", "user", "U2"),
        ];
        let reason = refused(plan(&raw, json!([{"op": "remove", "message_id": "u2"}])));
        assert!(
            reason.contains("would end with #2 (assistant), whose tool calls would run"),
            "{reason}"
        );
        planned(plan(
            &raw,
            json!([{"op": "edit", "message_id": "u2", "text": "E"}]),
        ));
    }

    #[test]
    fn a_paused_goal_event_never_hides_a_closed_tool_loop() {
        let stored = vec![
            msg("s", "system", "S"),
            msg("u1", "user", "U1"),
            assistant("a1", "", &["c1"]),
            result("t1", "tool", "c1", "T1"),
            paused_event("e"),
            msg("u2", "user", "U2"),
        ];
        let reason = refused(plan(&stored, json!([{"op": "remove", "message_id": "u2"}])));
        assert!(
            reason.contains("would end inside the tool call loop of #2 (assistant)"),
            "{reason}"
        );
        let mut shown = stored.clone();
        shown[4] = msg("e", "event", "SHOWN");
        planned(plan(&shown, json!([{"op": "remove", "message_id": "u2"}])));
    }

    #[test]
    fn a_new_separation_inside_an_already_damaged_bundle_is_refused() {
        let stored = vec![
            msg("s", "system", "S"),
            msg("u1", "user", "U1"),
            context_result("f1", "c1"),
            assistant("a1", "", &["c1"]),
            result("t1", "tool", "c1", "T1"),
            assistant("a2", "", &["c2"]),
            result("t2", "tool", "c2", "T2"),
            msg("u2", "user", "U2"),
        ];
        planned(plan(
            &stored,
            json!([{"op": "edit", "message_id": "u2", "text": "E"}]),
        ));
        let reason = refused(plan(
            &stored,
            json!([{"op": "insert", "after": "a1", "role": "user", "text": "X"}]),
        ));
        assert!(
            reason.contains(
                "op 1: the insert would separate the tool calls of #3 (assistant) from their results"
            ),
            "{reason}"
        );
    }

    #[test]
    fn a_result_never_moves_to_another_call_that_reuses_its_id() {
        let stored = vec![
            msg("s", "system", "S"),
            msg("u1", "user", "U1"),
            assistant("a1", "", &["c"]),
            result("r1", "tool", "c", "R1"),
            assistant("a2", "", &["c"]),
            result("r2", "tool", "c", "R2"),
            msg("u2", "user", "U2"),
        ];
        let reason = refused(plan(
            &stored,
            json!([{"op": "remove", "message_id": "r1", "through": "a2"}]),
        ));
        assert!(
            reason.contains(
                "op 1: removing #4 (assistant) would make its result #5 (tool) answer the tool call of #2 (assistant)"
            ),
            "{reason}"
        );
        let own = planned(plan(
            &stored,
            json!([{"op": "remove", "message_id": "a1", "with_results": true}]),
        ));
        assert_eq!(own.summary.removed, [2, 3]);
        let later = planned(plan(
            &stored,
            json!([{"op": "remove", "message_id": "a2", "with_results": true}]),
        ));
        assert_eq!(later.summary.removed, [4, 5]);
    }

    #[test]
    fn hidden_results_are_never_orphaned() {
        let stored = vec![
            msg("s", "system", "S"),
            msg("u1", "user", "U1"),
            assistant("a1", "", &["c1"]),
            result("t1", "tool", "c1", "T1"),
            hidden(result("h1", "tool", "c1", "H1")),
            msg("a2", "assistant", "A2"),
            msg("u2", "user", "U2"),
        ];
        let reason = refused(plan(
            &stored,
            json!([{"op": "remove", "message_id": "a1", "through": "t1"}]),
        ));
        assert!(
            reason.contains("op 1: removing #2 (assistant) would leave its result #4 (tool)"),
            "{reason}"
        );
        let plan = planned(plan(
            &stored,
            json!([{"op": "remove", "message_id": "a1", "through": "t1", "with_results": true}]),
        ));
        assert_eq!(plan.summary.removed, [2, 3, 4]);
    }

    #[test]
    fn results_placed_before_their_call_go_with_it() {
        let stored = vec![
            msg("s", "system", "S"),
            msg("u1", "user", "U1"),
            result("t0", "tool", "c1", "EARLY"),
            assistant("a1", "", &["c1"]),
            msg("a2", "assistant", "A2"),
            msg("u2", "user", "U2"),
        ];
        let plan = planned(plan(
            &stored,
            json!([{"op": "remove", "message_id": "a1", "with_results": true}]),
        ));
        assert_eq!(plan.summary.removed, [2, 3]);
    }

    #[test]
    fn a_context_file_never_counts_as_the_result_of_a_call() {
        let stored = vec![
            msg("s", "system", "S"),
            msg("u1", "user", "U1"),
            assistant("a1", "", &["c1"]),
            context_result("f1", "c1"),
            result("t1", "tool", "c1", "T1"),
            msg("a2", "assistant", "A2"),
        ];
        let reason = refused(plan(&stored, json!([{"op": "remove", "message_id": "t1"}])));
        assert!(
            reason.contains("op 1: removing #4 (tool) would leave a tool call of #2 (assistant)"),
            "{reason}"
        );
    }

    #[test]
    fn cascades_count_against_the_affected_limit() {
        let build = |calls: usize| {
            let ids: Vec<String> = (0..calls).map(|k| format!("c{k}")).collect();
            let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
            let mut stored = vec![
                msg("s", "system", "S"),
                msg("u1", "user", "U1"),
                assistant("a1", "", &refs),
            ];
            stored.extend(
                ids.iter()
                    .map(|id| result(&format!("t{id}"), "tool", id, "R")),
            );
            stored.push(msg("a2", "assistant", "A2"));
            stored
        };
        let remove = json!([{"op": "remove", "message_id": "a1", "with_results": true}]);
        assert_eq!(
            planned(plan(&build(1999), remove.clone())).summary.affected,
            2000
        );
        let reason = refused(plan(&build(2000), remove));
        assert!(
            reason.contains("the patch affects 2001 messages; the limit is 2000"),
            "{reason}"
        );
    }

    #[test]
    fn inserted_tool_calls_count_against_the_new_text_limit() {
        let insert = json!({"op": "insert", "after": "u2", "role": "assistant", "text": "",
            "tool_calls": [{"name": "shell", "arguments": {}, "result": "r".repeat(13 * 1024)}]});
        planned(plan(&chat(), Value::Array(vec![insert.clone(); 4])));
        let reason = refused(plan(&chat(), Value::Array(vec![insert; 5])));
        assert!(
            reason.contains("the new text of this call is over 65536 bytes"),
            "{reason}"
        );
    }

    #[test]
    fn limits_measure_the_serialized_text() {
        let insert =
            |text: String| json!({"op": "insert", "after": "end", "role": "user", "text": text});
        let reason = refused(plan(&chat(), json!([insert("\"".repeat(9000))])));
        assert!(reason.contains("the new text is 18002 bytes"), "{reason}");
        planned(plan(
            &chat(),
            Value::Array(vec![insert("\"".repeat(8000)); 4]),
        ));
        let reason = refused(plan(
            &chat(),
            Value::Array(vec![insert("\"".repeat(8000)); 5]),
        ));
        assert!(
            reason.contains("the new text of this call is over 65536 bytes"),
            "{reason}"
        );
    }

    #[test]
    fn every_new_text_is_checked_for_secrets() {
        for ops in [
            json!([{"op": "insert", "after": "end", "role": "user", "text": "set X TOKEN=abc"}]),
            json!([{"op": "edit", "message_id": "u2", "text": "TOKEN=abc"}]),
            json!([{"op": "insert", "after": "u2", "role": "assistant", "text": "",
                "tool_calls": [{"name": "shell", "arguments": {}, "result": "TOKEN=abc"}]}]),
            json!([{"op": "insert", "after": "u2", "role": "assistant", "text": "",
                "tool_calls": [{"name": "shell", "arguments": {"cmd": "X TOKEN=abc"}, "result": "ok"}]}]),
        ] {
            let reason = refused(plan(&chat(), ops.clone()));
            assert!(reason.contains("looks like a secret"), "{ops}: {reason}");
        }
    }

    #[test]
    fn hidden_and_developer_prompts_block_system_edits() {
        let mut hidden_first = chat();
        hidden_first.insert(0, hidden(msg("sh", "system", "HIDDEN")));
        let reason = refused(plan(
            &hidden_first,
            json!([{"op": "edit", "message_id": "s", "text": "X"}]),
        ));
        assert!(
            reason.contains("that chat has other system or developer messages"),
            "{reason}"
        );
        let mut developer = chat();
        developer.insert(5, msg("d", "developer", "DEV"));
        let reason = refused(plan(
            &developer,
            json!([{"op": "edit", "message_id": "s", "text": "X"}]),
        ));
        assert!(
            reason.contains("that chat has other system or developer messages"),
            "{reason}"
        );
        let reason = refused(plan(
            &developer,
            json!([{"op": "remove", "message_id": "d"}]),
        ));
        assert!(
            reason.contains("system and developer messages cannot be removed"),
            "{reason}"
        );
        let no_system: Vec<ChatMessage> = developer[1..].to_vec();
        let reason = refused(plan(
            &no_system,
            json!([{"op": "insert", "after": "start", "role": "system", "text": "S"}]),
        ));
        assert!(
            reason.contains("already has a system or developer message"),
            "{reason}"
        );
    }

    #[test]
    fn the_compression_report_itself_is_never_a_target() {
        let report = make_reconstruction_report(
            vec![msg("s", "system", "S"), msg("u0", "user", "U0")],
            ReconstructionMetadata::default(),
        )
        .unwrap();
        let report_id = report.message_id.clone();
        let stored = vec![report, msg("u1", "user", "U1")];
        for ops in [
            json!([{"op": "edit", "message_id": report_id, "text": "X"}]),
            json!([{"op": "remove", "message_id": report_id}]),
        ] {
            let reason = refused(plan(&stored, ops));
            assert!(reason.contains("inside compressed history"), "{reason}");
        }
    }

    #[test]
    fn stored_role_strings_never_reach_a_reason_or_the_summary() {
        let mut stored = chat();
        stored.insert(5, msg("x", "PRIVATE_SENTINEL", "PRIVATE_SENTINEL"));
        let reason = refused(plan(
            &stored,
            json!([{"op": "edit", "message_id": "x", "text": "E"}]),
        ));
        assert!(
            reason.contains("#5 (other role) cannot be edited"),
            "{reason}"
        );
        let plan = planned(plan(&stored, json!([{"op": "remove", "message_id": "x"}])));
        assert_eq!(labels(&plan), ["#5 other role: remove"]);
        assert!(!format!("{:?}", plan.summary).contains("PRIVATE_SENTINEL"));
    }

    #[test]
    fn the_card_limit_counts_the_whole_serialized_card() {
        let stored: Vec<ChatMessage> = (0..50)
            .map(|i| msg(&format!("u{i}"), "user", &"a".repeat(4096)))
            .collect();
        let edits = |n: usize| {
            Value::Array(
                (0..n)
                    .map(|i| {
                        let len = if i == 49 { 1000 } else { 1129 };
                        json!({"op": "edit", "message_id": format!("u{i}"), "text": "b".repeat(len)})
                    })
                    .collect(),
            )
        };
        let reason = refused(plan(&stored, edits(50)));
        assert!(
            reason.contains("the approval card would be over 262144 bytes"),
            "{reason}"
        );
        planned(plan(&stored, edits(45)));
    }

    #[test]
    fn bundle_windows_follow_the_wire() {
        let calls = |between: ChatMessage| {
            vec![
                assistant("a", "", &["c"]),
                between,
                result("t", "tool", "c", "T"),
            ]
        };
        assert!(on_view(&calls(msg("x", "error", "E"))).is_empty());
        assert!(on_view(&calls(paused_event("e"))).is_empty());
        assert_eq!(
            on_view(&calls(msg("e", "event", "E"))),
            [
                Flaw::Crossed(Origin::Base(0), Origin::Base(2)),
                Flaw::Unanswered(Origin::Base(0), "c".into())
            ]
        );
        assert_eq!(
            on_view(&[
                assistant("a", "", &["c"]),
                hidden(result("h", "tool", "c", "H"))
            ]),
            [Flaw::Unanswered(Origin::Base(0), "c".into())]
        );
    }

    #[test]
    fn the_prerun_check_holds_for_every_set_of_allowed_tools() {
        let mut twice = assistant("a", "", &["c", "c"]);
        twice.tool_calls.as_mut().unwrap()[1].function.name = "cat".into();
        let tail = vec![msg("u", "user", "U"), twice.clone()];
        assert!(tool_prerun_calls(&tail, |_| true).is_empty());
        assert_eq!(tool_prerun_calls(&tail, |name| name == "cat").len(), 1);
        assert!(tool_prerun_possible(&tail));
        assert!(tool_prerun_possible(&[assistant("a", "", &["c1", "c2"])]));
        assert!(!tool_prerun_possible(&[
            assistant("a", "", &["c1"]),
            result("t", "tool", "c1", "T")
        ]));
        assert!(!tool_prerun_possible(&[msg("u", "user", "U")]));
        let report = make_reconstruction_report(
            vec![msg("s", "system", "S"), msg("u0", "user", "U0"), twice],
            ReconstructionMetadata::default(),
        )
        .unwrap();
        let stored = vec![report, msg("x", "error", "E")];
        let reason = refused(plan(&stored, json!([{"op": "remove", "message_id": "x"}])));
        assert!(reason.contains("whose tool calls would run"), "{reason}");
    }

    #[test]
    fn the_session_window_does_not_stop_at_a_system_message() {
        let stored = vec![
            msg("s", "system", "S"),
            msg("u1", "user", "U1"),
            assistant("a1", "", &["c1"]),
            msg("s2", "system", "S2"),
            result("t1", "tool", "c1", "T1"),
        ];
        assert!(!pending_tool_result_window(&stored, 0));
        assert_eq!(
            tool_results_after(&stored, 2),
            HashSet::from(["c1".to_string()])
        );
    }

    #[test]
    fn a_call_needs_a_result_inside_its_window_even_when_an_old_one_is_misplaced() {
        let stored = vec![
            msg("s", "system", "S"),
            msg("u1", "user", "U1"),
            assistant("a1", "", &["c1"]),
            result("good", "tool", "c1", "GOOD"),
            msg("u2", "user", "U2"),
            result("late", "tool", "c1", "LATE"),
            assistant("b", "", &["d"]),
            result("td", "tool", "d", "TD"),
            msg("u3", "user", "U3"),
        ];
        planned(plan(
            &stored,
            json!([{"op": "edit", "message_id": "u3", "text": "E"}]),
        ));
        let reason = refused(plan(
            &stored,
            json!([{"op": "remove", "message_id": "good"}]),
        ));
        assert!(
            reason.contains("op 1: removing #3 (tool) would leave a tool call of #2 (assistant) without its result"),
            "{reason}"
        );
        planned(plan(
            &stored,
            json!([{"op": "remove", "message_id": "late"}]),
        ));
    }

    #[test]
    fn hidden_assistants_keep_their_results() {
        let stored = vec![
            msg("s", "system", "S"),
            msg("u1", "user", "U1"),
            hidden(assistant("a1", "", &["c1"])),
            hidden(result("t1", "tool", "c1", "T1")),
            msg("u2", "user", "U2"),
            msg("a2", "assistant", "A2"),
        ];
        let reason = refused(plan(&stored, json!([{"op": "remove", "message_id": "a1"}])));
        assert!(
            reason.contains(
                "op 1: removing #2 (assistant) would leave its result #3 (tool) without its call"
            ),
            "{reason}"
        );
        let plan = planned(plan(
            &stored,
            json!([{"op": "remove", "message_id": "a1", "with_results": true}]),
        ));
        assert_eq!(plan.summary.removed, [2, 3]);
    }

    #[test]
    fn user_messages_the_at_commands_step_drops_never_protect_a_call() {
        let report = make_reconstruction_report(
            vec![
                msg("s", "system", "S"),
                msg("u0", "user", "U0"),
                assistant("a0", "", &["c0"]),
            ],
            ReconstructionMetadata::default(),
        )
        .unwrap();
        let stopper = vec![
            report.clone(),
            msg("u1", "user", "U1"),
            msg("w", "user", "write to alice@example.test"),
        ];
        planned(plan(
            &stopper,
            json!([{"op": "remove", "message_id": "u1"}]),
        ));
        for stopper in ["text @mention", "prefix @file src/a.rs"] {
            let stored = vec![
                report.clone(),
                msg("u1", "user", "U1"),
                msg("w", "user", stopper),
            ];
            planned(plan(&stored, json!([{"op": "remove", "message_id": "u1"}])));
        }
        for vanishing in ["   ", "@file src/a.rs", "@web rust please"] {
            let stored = vec![
                report.clone(),
                msg("u1", "user", "U1"),
                msg("w", "user", vanishing),
            ];
            let reason = refused(plan(&stored, json!([{"op": "remove", "message_id": "u1"}])));
            assert!(
                reason.contains(
                    "would end with message 2 of the compressed head (assistant), whose tool calls would run"
                ),
                "{vanishing}: {reason}"
            );
            planned(plan(
                &stored,
                json!([{"op": "edit", "message_id": "u1", "text": "E"}]),
            ));
        }
        let looped = vec![
            msg("s", "system", "S"),
            msg("u1", "user", "U1"),
            assistant("a1", "", &["c1"]),
            result("t1", "tool", "c1", "T1"),
            msg("u2", "user", "U2"),
            msg("w", "user", " "),
        ];
        let reason = refused(plan(&looped, json!([{"op": "remove", "message_id": "u2"}])));
        assert!(
            reason.contains("would end inside the tool call loop of #2 (assistant)"),
            "{reason}"
        );
        planned(plan(
            &looped,
            json!([{"op": "edit", "message_id": "u2", "text": "E"}]),
        ));
    }

    #[test]
    fn an_edited_system_message_must_come_first() {
        let stored = vec![
            msg("u1", "user", "U1"),
            msg("s", "system", "OLD"),
            msg("u2", "user", "U2"),
        ];
        let reason = refused(plan(
            &stored,
            json!([{"op": "edit", "message_id": "s", "text": "NEW"}]),
        ));
        assert!(
            reason
                .contains("op 1: the edited system message must be the first message of that chat"),
            "{reason}"
        );
        let plan = planned(plan(
            &stored,
            json!([
                {"op": "remove", "message_id": "u1"},
                {"op": "edit", "message_id": "s", "text": "NEW"},
            ]),
        ));
        assert_eq!(texts(&plan.messages), ["NEW", "U2"]);
    }

    #[test]
    fn a_hidden_assistant_never_hides_what_a_visible_call_loses() {
        let stored = vec![
            msg("s", "system", "S"),
            msg("u1", "user", "U1"),
            assistant("a1", "", &["c"]),
            hidden(assistant("h", "", &["c"])),
            result("r", "tool", "c", "R"),
            assistant("b", "", &["d"]),
            result("td", "tool", "d", "TD"),
            msg("u2", "user", "U2"),
        ];
        let reason = refused(plan(&stored, json!([{"op": "remove", "message_id": "r"}])));
        assert!(
            reason.contains("op 1: removing #4 (tool) would leave a tool call of #2 (assistant) without its result"),
            "{reason}"
        );
        let reason = refused(plan(&stored, json!([{"op": "remove", "message_id": "a1"}])));
        assert!(
            reason.contains("op 1: removing #2 (assistant) would leave its result #4 (tool)"),
            "{reason}"
        );
        let plan = planned(plan(
            &stored,
            json!([{"op": "remove", "message_id": "a1", "with_results": true}]),
        ));
        assert_eq!(plan.summary.removed, [2, 4]);
    }

    #[test]
    fn the_card_limit_leaves_room_for_the_request_envelope() {
        let stored: Vec<ChatMessage> = (0..50)
            .map(|i| msg(&format!("u{i}"), "user", &"a".repeat(4096)))
            .collect();
        let edits = Value::Array(
            (0..50)
                .map(|i| {
                    let len = if i == 49 { 1105 } else { 1102 };
                    json!({"op": "edit", "message_id": format!("u{i}"), "text": "b".repeat(len)})
                })
                .collect(),
        );
        let reason = refused(plan(&stored, edits));
        assert!(
            reason.contains("the approval card would be over 262144 bytes"),
            "{reason}"
        );
    }

    #[test]
    fn removing_a_duplicate_inside_the_final_tool_loop_is_a_repair() {
        let stored = vec![
            msg("s", "system", "S"),
            msg("u1", "user", "U1"),
            assistant("a1", "", &["c1"]),
            result("dup", "tool", "c1", "DUP"),
            result("keep", "tool", "c1", "KEEP"),
        ];
        let plan = planned(plan(
            &stored,
            json!([{"op": "remove", "message_id": "dup"}]),
        ));
        assert_eq!(plan.summary.removed, [3]);
        let reason = refused(plan_env(
            &stored,
            json!([{"op": "edit", "message_id": "keep", "text": "CHANGED"}]),
            FrozenSystem::Absent,
        ));
        assert!(
            reason.contains("would end inside the tool call loop of #2 (assistant)"),
            "{reason}"
        );
    }

    #[test]
    fn a_patch_never_leaves_a_history_without_a_prompt_anchor() {
        let stored = vec![msg("a", "assistant", "A"), msg("u", "user", "U")];
        let reason = refused(plan(&stored, json!([{"op": "remove", "message_id": "u"}])));
        assert!(
            reason.contains("would hold no system, user, event, plan or goal message"),
            "{reason}"
        );
    }

    #[test]
    fn a_fragment_edit_of_a_long_system_prompt_shows_only_the_fragment() {
        let long = format!("{} MARKER {}", "p".repeat(10_000), "q".repeat(10_000));
        let mut stored = chat();
        stored[0] = msg("s", "system", &long);
        let plan = planned(plan_env(
            &stored,
            json!([{"op": "edit", "message_id": "s", "find": "MARKER", "replace": "CHANGED"}]),
            FrozenSystem::Complete(&long),
        ));
        assert!(plan.items.contains(&PatchItem {
            label: "system prompt (frozen copy)".into(),
            old: Some(json!("MARKER")),
            new: Some(json!("CHANGED")),
        }));
        assert_eq!(
            plan.frozen_system_prompt.as_deref(),
            Some(long.replace("MARKER", "CHANGED").as_str())
        );
    }

    #[test]
    fn card_excerpts_are_cut_from_the_start_of_any_content() {
        let mut stored = chat();
        let mut file = context_result("f", "");
        file.content = ChatContent::ContextFiles(vec![ContextFile {
            file_name: "src/big.rs".into(),
            file_content: "x".repeat(50_000),
            line1: 1,
            line2: 9,
            ..Default::default()
        }]);
        stored.insert(5, file);
        stored[3].content = ChatContent::Multimodal(vec![
            MultimodalElement {
                m_type: "text".into(),
                m_content: "y".repeat(50_000),
            },
            MultimodalElement {
                m_type: "image/png".into(),
                m_content: "iVBORw0KGgo=".into(),
            },
        ]);
        let plan = planned(plan(
            &stored,
            json!([
                {"op": "remove", "message_id": "f"},
                {"op": "remove", "message_id": "a1", "with_results": true},
            ]),
        ));
        let file_row = plan.items[0].old.as_ref().unwrap().as_str().unwrap();
        assert!(file_row.starts_with("src/big.rs:1-9\nxxx") && file_row.ends_with('…'));
        assert!(file_row.len() <= OLD_TEXT_BYTES, "{}", file_row.len());
        let result_row = plan.items[2].old.as_ref().unwrap().as_str().unwrap();
        assert!(result_row.starts_with("yyy") && result_row.ends_with('…'));
        assert!(
            result_row.len() <= CASCADE_TEXT_BYTES,
            "{}",
            result_row.len()
        );
    }
}
