use axum::response::Result;
use axum::Extension;
use hyper::{Body, Response, StatusCode};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use serde_json::{json, Value};
use tokio::sync::RwLock as ARwLock;
use tokio::sync::Mutex as AMutex;
use strsim::jaro_winkler;
use itertools::Itertools;
use tokenizers::Tokenizer;
use tracing::info;

use crate::ext::config_dirs::{CommandSource, get_ext_dirs};
use crate::ext::slash_commands::{SlashCommand, load_slash_commands};
use crate::ext::skills::{SkillIndex, load_skill_indices};
use crate::integrations::mcp::mcp_prompts::mcp_prompts_as_slash_commands;

use crate::at_commands::execute_at::run_at_commands_locally;
use crate::indexing_utils::wait_for_indexing_if_needed;
use crate::postprocessing::pp_context_files::postprocess_context_files;
use crate::tokens;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_commands::execute_at::{execute_at_commands_in_query, parse_words_from_line};
use crate::call_validation::{ChatMeta, PostprocessSettings, SubchatParameters};
use crate::caps::resolve_chat_model;
use crate::custom_error::ScratchError;
use crate::global_context::try_load_caps_quickly_if_not_present;
use crate::global_context::GlobalContext;
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum, deserialize_messages_from_post};
use crate::at_commands::at_commands::filter_only_context_file_from_context_tool;
use crate::chat::get_or_create_session_with_trajectory;
use crate::scratchpads::scratchpad_utils::HasRagResults;

#[derive(Serialize, Deserialize, Clone)]
struct CommandCompletionPost {
    query: String,
    cursor: i64,
    top_n: usize,
}
#[derive(Serialize, Deserialize, Clone)]
pub struct CompletionDetail {
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
    pub source: String,
    pub kind: String,
}

#[derive(Serialize, Deserialize, Clone)]
struct CommandCompletionResponse {
    completions: Vec<String>,
    replace: (i64, i64),
    is_cmd_executable: bool,
    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    completion_details: HashMap<String, CompletionDetail>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SlashCommandInfo {
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
    pub source: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub user_invocable: bool,
    pub source: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SlashCommandsListResponse {
    pub commands: Vec<SlashCommandInfo>,
    pub skills: Vec<SkillInfo>,
}

const SLASH_CACHE_TTL: Duration = Duration::from_secs(5);

struct SlashCacheEntry {
    commands: Vec<SlashCommand>,
    skills: Vec<SkillIndex>,
    loaded_at: Instant,
    generation: u64,
}

static SLASH_CACHE: OnceLock<tokio::sync::RwLock<Option<SlashCacheEntry>>> = OnceLock::new();

pub async fn invalidate_slash_cache() {
    if let Some(lock) = SLASH_CACHE.get() {
        *lock.write().await = None;
    }
}

async fn load_slash_commands_and_skills(
    gcx: Arc<ARwLock<GlobalContext>>,
) -> (Vec<SlashCommand>, Vec<SkillIndex>) {
    let current_gen = gcx
        .read()
        .await
        .ext_cache_generation
        .load(Ordering::Relaxed);
    let lock = SLASH_CACHE.get_or_init(|| tokio::sync::RwLock::new(None));
    {
        let read = lock.read().await;
        if let Some(entry) = &*read {
            if entry.loaded_at.elapsed() < SLASH_CACHE_TTL && entry.generation == current_gen {
                return (entry.commands.clone(), entry.skills.clone());
            }
        }
    }
    let ext_dirs = get_ext_dirs(gcx.clone()).await;
    let mut commands = load_slash_commands(&ext_dirs).await;
    let mcp_commands = mcp_prompts_as_slash_commands(gcx).await;
    commands.extend(mcp_commands);
    let skills = load_skill_indices(&ext_dirs).await;
    let mut write = lock.write().await;
    *write = Some(SlashCacheEntry {
        commands: commands.clone(),
        skills: skills.clone(),
        loaded_at: Instant::now(),
        generation: current_gen,
    });
    (commands, skills)
}

fn source_label(source: &CommandSource) -> String {
    match source {
        CommandSource::GlobalClaude => "global_claude".to_string(),
        CommandSource::GlobalRefact => "global_refact".to_string(),
        CommandSource::ProjectClaude(_) => "project_claude".to_string(),
        CommandSource::ProjectRefact(_) => "project_refact".to_string(),
        CommandSource::InstalledPlugin(name) => format!("plugin:{}", name),
    }
}

pub fn slash_completions_for_prefix(
    commands: &[SlashCommand],
    skills: &[SkillIndex],
    prefix: &str,
) -> (Vec<String>, HashMap<String, CompletionDetail>) {
    if !prefix.starts_with('/') {
        return (Vec::new(), HashMap::new());
    }
    let mut completions: Vec<String> = Vec::new();
    let mut details: HashMap<String, CompletionDetail> = HashMap::new();
    for cmd in commands {
        let name_with_slash = format!("/{}", cmd.name);
        if name_with_slash.starts_with(prefix) {
            details.insert(
                name_with_slash.clone(),
                CompletionDetail {
                    description: cmd.description.clone(),
                    argument_hint: if cmd.argument_hint.is_empty() {
                        None
                    } else {
                        Some(cmd.argument_hint.clone())
                    },
                    source: source_label(&cmd.source),
                    kind: "cmd".to_string(),
                },
            );
            completions.push(name_with_slash);
        }
    }
    for skill in skills {
        if !skill.user_invocable {
            continue;
        }
        let name_with_slash = format!("/{}", skill.name);
        if name_with_slash.starts_with(prefix) && !details.contains_key(&name_with_slash) {
            details.insert(
                name_with_slash.clone(),
                CompletionDetail {
                    description: skill.description.clone(),
                    argument_hint: None,
                    source: source_label(&skill.source),
                    kind: "skill".to_string(),
                },
            );
            completions.push(name_with_slash);
        }
    }
    completions.sort();
    completions.dedup();
    (completions, details)
}

#[derive(Serialize, Deserialize, Clone)]
struct CommandPreviewPost {
    #[serde(default)]
    pub messages: Vec<Value>,
    #[serde(default)]
    model: String,
    #[serde(default)]
    provider: String,
    #[serde(default)]
    pub meta: ChatMeta,
}

#[derive(Serialize, Deserialize, Clone)]
struct Highlight {
    kind: String,
    pos1: i64,
    pos2: i64,
    ok: bool,
    reason: String,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct CommandExecutePost {
    pub messages: Vec<ChatMessage>,
    pub n_ctx: usize,
    pub maxgen: usize,
    pub subchat_tool_parameters: IndexMap<String, SubchatParameters>, // tool_name: {model, allowed_context, temperature}
    pub postprocess_parameters: PostprocessSettings,
    pub model_name: String,
    pub chat_id: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CommandExecuteResponse {
    pub messages: Vec<ChatMessage>,
    pub undroppable_msg_number: usize,
    pub any_context_produced: bool,
    pub messages_to_stream_back: Vec<serde_json::Value>,
}

pub async fn handle_v1_command_completion(
    Extension(global_context): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<CommandCompletionPost>(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;
    let top_n = post.top_n;

    let fake_n_ctx = 4096;
    let ccx: Arc<AMutex<AtCommandsContext>> = Arc::new(AMutex::new(
        AtCommandsContext::new(
            global_context.clone(),
            fake_n_ctx,
            top_n,
            true,
            vec![],
            "".to_string(),
            None,
            "".to_string(),
            None,
            None,
        )
        .await,
    ));

    let at_commands = ccx.lock().await.at_commands.clone();
    let at_command_names = at_commands.keys().map(|x| x.clone()).collect::<Vec<_>>();

    let mut completions: Vec<String> = vec![];
    let mut pos1 = -1;
    let mut pos2 = -1;
    let mut is_cmd_executable = false;
    let mut completion_details: HashMap<String, CompletionDetail> = HashMap::new();

    if let Ok((query_line_val, cursor_rel, cursor_line_start)) =
        get_line_with_cursor(&post.query, post.cursor)
    {
        let query_line_val_at_cursor = query_line_val
            .chars()
            .take(cursor_rel as usize)
            .collect::<String>();
        let args = query_line_args(
            &query_line_val_at_cursor,
            cursor_rel,
            cursor_line_start,
            &at_command_names,
        );
        info!("args: {:?}", args);
        let focused_slash = args.iter().find(|a| a.focused && a.value.starts_with('/'));
        if let Some(focused) = focused_slash {
            let (slash_cmds, skills) = load_slash_commands_and_skills(global_context.clone()).await;
            let (raw_completions, details) =
                slash_completions_for_prefix(&slash_cmds, &skills, &focused.value);
            is_cmd_executable = raw_completions.iter().any(|c| c == &focused.value);
            pos1 = focused.pos1;
            pos2 = focused.pos2;
            completion_details = details;
            completions = raw_completions;
        } else {
            (completions, is_cmd_executable, pos1, pos2) =
                command_completion(ccx.clone(), args, post.cursor).await;
        }
    }
    let completions: Vec<_> = completions
        .into_iter()
        .unique()
        .map(|x| {
            if x.starts_with('/') {
                x
            } else {
                format!("{} ", x)
            }
        })
        .collect();

    let response = CommandCompletionResponse {
        completions,
        replace: (pos1, pos2),
        is_cmd_executable,
        completion_details,
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(serde_json::to_string(&response).unwrap()))
        .unwrap())
}

async fn count_tokens(
    tokenizer_arc: Option<Arc<Tokenizer>>,
    messages: &Vec<ChatMessage>,
) -> Result<u64, ScratchError> {
    let mut accum: u64 = 0;

    for message in messages {
        accum += message
            .content
            .count_tokens(tokenizer_arc.clone(), &None)
            .map_err(|e| {
                ScratchError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("v1_chat_token_counter: count_tokens failed: {}", e),
                )
            })? as u64;
    }
    Ok(accum)
}

pub async fn handle_v1_command_preview(
    Extension(global_context): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    let post = serde_json::from_slice::<CommandPreviewPost>(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;
    let mut messages = deserialize_messages_from_post(&post.messages)?;

    let last_message = messages.pop();
    let mut query = if let Some(last_message) = &last_message {
        match &last_message.content {
            ChatContent::SimpleText(query) => query.clone(),
            ChatContent::Multimodal(elements) => {
                let mut query = String::new();
                for element in elements {
                    if element.is_text() {
                        // use last text, but expected to be only one
                        query = element.m_content.clone();
                    }
                }
                query
            }
            ChatContent::ContextFiles(_) => {
                // Context files don't contain user query text
                String::new()
            }
        }
    } else {
        String::new()
    };

    let caps =
        crate::global_context::try_load_caps_quickly_if_not_present(global_context.clone(), 0)
            .await?;
    let model_rec = resolve_chat_model(caps, &post.model)
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    let tokenizer_arc =
        match tokens::cached_tokenizer(global_context.clone(), &model_rec.base).await {
            Ok(x) => x,
            Err(e) => {
                tracing::error!(e);
                return Err(ScratchError::new(StatusCode::BAD_REQUEST, e));
            }
        };

    let ccx = Arc::new(AMutex::new(
        AtCommandsContext::new(
            global_context.clone(),
            model_rec.base.n_ctx,
            crate::constants::CHAT_TOP_N,
            true,
            messages,
            "".to_string(),
            None,
            model_rec.base.id.clone(),
            None,
            None,
        )
        .await,
    ));

    let (messages_for_postprocessing, vec_highlights) =
        execute_at_commands_in_query(ccx.clone(), &mut query).await;

    let mut preview: Vec<ChatMessage> = vec![];
    for exec_result in messages_for_postprocessing.iter() {
        if let ContextEnum::ChatMessage(raw_msg) = exec_result {
            preview.push(raw_msg.clone());
        }
    }

    let mut context_files =
        filter_only_context_file_from_context_tool(&messages_for_postprocessing);

    if !context_files.is_empty() {
        let (gcx, mut pp_settings) = {
            let ccx_locked = ccx.lock().await;
            (
                ccx_locked.global_context.clone(),
                ccx_locked.postprocess_parameters.clone(),
            )
        };

        pp_settings.max_files_n = pp_settings.max_files_n.max(1);
        pp_settings.use_ast_based_pp = false;

        let tokens_limit = (model_rec.base.n_ctx / 4).max(256);
        let (post_processed_files, _notes) = postprocess_context_files(
            gcx.clone(),
            &mut context_files,
            tokenizer_arc.clone(),
            tokens_limit,
            false,
            &pp_settings,
        )
        .await;

        if !post_processed_files.is_empty() {
            let message = ChatMessage {
                role: "context_file".to_string(),
                content: ChatContent::ContextFiles(post_processed_files),
                ..Default::default()
            };
            preview.push(message);
        }
    }

    let mut highlights = vec![];
    for h in vec_highlights {
        highlights.push(Highlight {
            kind: h.kind.clone(),
            pos1: h.pos1 as i64,
            pos2: h.pos2 as i64,
            ok: h.ok,
            reason: h.reason.unwrap_or_default(),
        })
    }

    let messages_to_count = if let Some(mut last_message) = last_message {
        match &mut last_message.content {
            ChatContent::SimpleText(_) => {
                last_message.content = ChatContent::SimpleText(query.clone());
            }
            ChatContent::Multimodal(elements) => {
                for elem in elements {
                    if elem.is_text() {
                        elem.m_content = query.clone();
                    }
                }
            }
            ChatContent::ContextFiles(_) => {
                // Context files are not user queries, leave unchanged
            }
        };
        itertools::concat(vec![preview.clone(), vec![last_message]])
    } else {
        preview.clone()
    };
    let tokens_number = count_tokens(tokenizer_arc.clone(), &messages_to_count).await?;

    Ok(Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(
            serde_json::to_string_pretty(
                &json!({"messages": preview, "model": model_rec.base.id, "highlight": highlights,
                "current_context": tokens_number, "number_context": model_rec.base.n_ctx}),
            )
            .unwrap(),
        ))
        .unwrap())
}

pub async fn handle_v1_at_command_execute(
    Extension(global_context): Extension<Arc<ARwLock<GlobalContext>>>,
    body_bytes: hyper::body::Bytes,
) -> Result<Response<Body>, ScratchError> {
    wait_for_indexing_if_needed(global_context.clone()).await;

    let post = serde_json::from_slice::<CommandExecutePost>(&body_bytes).map_err(|e| {
        ScratchError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("JSON problem: {}", e),
        )
    })?;

    let caps = try_load_caps_quickly_if_not_present(global_context.clone(), 0).await?;
    let model_rec = resolve_chat_model(caps, &post.model_name)
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let tokenizer = tokens::cached_tokenizer(global_context.clone(), &model_rec.base)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;

    let effective_n_ctx = post.n_ctx.min(model_rec.base.n_ctx).max(1);

    let mut ccx = AtCommandsContext::new(
        global_context.clone(),
        effective_n_ctx,
        crate::constants::CHAT_TOP_N,
        false,
        post.messages.clone(),
        post.chat_id.clone(),
        None,
        model_rec.base.id.clone(),
        None,
        None,
    )
    .await;
    ccx.subchat_tool_parameters = post.subchat_tool_parameters.clone();
    ccx.postprocess_parameters = post.postprocess_parameters.clone();
    let ccx_arc = Arc::new(AMutex::new(ccx));

    let mut has_rag_results = HasRagResults::new();
    let (messages, any_context_produced) = run_at_commands_locally(
        ccx_arc.clone(),
        tokenizer.clone(),
        post.maxgen,
        post.messages.clone(),
        &mut has_rag_results,
    )
    .await;
    let messages_to_stream_back = has_rag_results.in_json;

    if !post.chat_id.is_empty() && any_context_produced {
        let sessions = global_context.read().await.chat_sessions.clone();
        let session_arc =
            get_or_create_session_with_trajectory(global_context.clone(), &sessions, &post.chat_id)
                .await;
        let mut session = session_arc.lock().await;
        let original_len = post.messages.len();
        for msg in messages.iter().skip(original_len) {
            session.add_message(msg.clone());
        }
    }

    let undroppable_msg_number = messages
        .iter()
        .rposition(|msg| msg.role == "user")
        .unwrap_or(0);

    let response = CommandExecuteResponse {
        messages,
        messages_to_stream_back,
        undroppable_msg_number,
        any_context_produced,
    };

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string_pretty(&response).unwrap()))
        .unwrap())
}

fn get_line_with_cursor(query: &String, cursor: i64) -> Result<(String, i64, i64), ScratchError> {
    let mut cursor_rel = cursor;
    for line in query.lines() {
        let line_length = line.len() as i64;
        if cursor_rel <= line_length {
            return Ok((line.to_string(), cursor_rel, cursor - cursor_rel));
        }
        cursor_rel -= line_length + 1; // +1 to account for the newline character
    }
    return Err(ScratchError::new(
        StatusCode::EXPECTATION_FAILED,
        "incorrect cursor provided".to_string(),
    ));
}

async fn command_completion(
    ccx: Arc<AMutex<AtCommandsContext>>,
    args: Vec<QueryLineArg>,
    cursor_abs: i64,
) -> (Vec<String>, bool, i64, i64) {
    // returns ([possible, completions], good_as_it_is)
    let mut args = args;
    let at_commands = ccx.lock().await.at_commands.clone();
    let at_command_names = at_commands.keys().map(|x| x.clone()).collect::<Vec<_>>();

    let q_cmd_with_index = args
        .iter()
        .enumerate()
        .find_map(|(index, x)| x.value.starts_with("@").then(|| (x, index)));
    let (q_cmd, q_cmd_idx) = match q_cmd_with_index {
        Some((x, idx)) => (x.clone(), idx),
        None => return (vec![], false, -1, -1),
    };

    let cmd = match at_command_names
        .iter()
        .find(|x| x == &&q_cmd.value)
        .and_then(|x| at_commands.get(x))
    {
        Some(x) => x,
        None => {
            return if !q_cmd.focused {
                (vec![], false, -1, -1)
            } else {
                (
                    command_completion_options(ccx.clone(), &q_cmd.value).await,
                    false,
                    q_cmd.pos1,
                    q_cmd.pos2,
                )
            }
        }
    };
    args = args
        .iter()
        .skip(q_cmd_idx + 1)
        .map(|x| x.clone())
        .collect::<Vec<_>>();
    let cmd_params_cnt = cmd.params().len();
    args.truncate(cmd_params_cnt);

    let can_execute = args.len() == cmd.params().len();

    for (arg, param) in args.iter().zip(cmd.params()) {
        let is_valid = param.is_value_valid(ccx.clone(), &arg.value).await;
        if !is_valid {
            return if arg.focused {
                (
                    param.param_completion(ccx.clone(), &arg.value).await,
                    can_execute,
                    arg.pos1,
                    arg.pos2,
                )
            } else {
                (vec![], false, -1, -1)
            };
        }
        if is_valid && arg.focused && param.param_completion_valid() {
            return (
                param.param_completion(ccx.clone(), &arg.value).await,
                can_execute,
                arg.pos1,
                arg.pos2,
            );
        }
    }

    if can_execute {
        return (vec![], true, -1, -1);
    }

    // if command is not focused, and the argument is empty we should make suggestions
    if !q_cmd.focused {
        match cmd.params().get(args.len()) {
            Some(param) => {
                return (
                    param.param_completion(ccx.clone(), &"".to_string()).await,
                    false,
                    cursor_abs,
                    cursor_abs,
                );
            }
            None => {}
        }
    }

    (vec![], false, -1, -1)
}

async fn command_completion_options(
    ccx: Arc<AMutex<AtCommandsContext>>,
    q_cmd: &String,
) -> Vec<String> {
    let at_commands = ccx.lock().await.at_commands.clone();
    let at_command_names = at_commands.keys().map(|x| x.clone()).collect::<Vec<_>>();
    at_command_names
        .iter()
        .filter(|command| command.starts_with(q_cmd))
        .map(|command| (command.to_string(), jaro_winkler(&command, q_cmd)))
        .sorted_by(|(_, dist1), (_, dist2)| dist1.partial_cmp(dist2).unwrap())
        .rev()
        .take(5)
        .map(|(command, _)| command.clone())
        .collect()
}

pub fn query_line_args(
    line: &String,
    cursor_rel: i64,
    cursor_line_start: i64,
    at_command_names: &Vec<String>,
) -> Vec<QueryLineArg> {
    let mut args: Vec<QueryLineArg> = vec![];
    for (text, pos1, pos2) in parse_words_from_line(line).iter().rev().cloned() {
        if at_command_names.contains(&text)
            && args.iter().any(|x| {
                (x.value.contains("@") && x.focused) || at_command_names.contains(&x.value)
            })
        {
            break;
        }
        let mut x = QueryLineArg {
            value: text.clone(),
            pos1: pos1 as i64,
            pos2: pos2 as i64,
            focused: false,
        };
        x.focused = cursor_rel >= x.pos1 && cursor_rel <= x.pos2;
        x.pos1 += cursor_line_start;
        x.pos2 += cursor_line_start;
        args.push(x)
    }
    args.iter().rev().cloned().collect::<Vec<_>>()
}

#[derive(Debug, Clone)]
pub struct QueryLineArg {
    pub value: String,
    pub pos1: i64,
    pub pos2: i64,
    pub focused: bool,
}

pub async fn handle_v1_slash_commands(
    Extension(global_context): Extension<Arc<ARwLock<GlobalContext>>>,
) -> Result<Response<Body>, ScratchError> {
    let (commands, skills) = load_slash_commands_and_skills(global_context).await;
    let response = SlashCommandsListResponse {
        commands: commands
            .iter()
            .map(|cmd| SlashCommandInfo {
                name: cmd.name.clone(),
                description: cmd.description.clone(),
                argument_hint: if cmd.argument_hint.is_empty() {
                    None
                } else {
                    Some(cmd.argument_hint.clone())
                },
                source: source_label(&cmd.source),
            })
            .collect(),
        skills: skills
            .iter()
            .map(|skill| SkillInfo {
                name: skill.name.clone(),
                description: skill.description.clone(),
                user_invocable: skill.user_invocable,
                source: source_label(&skill.source),
            })
            .collect(),
    };
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(serde_json::to_string(&response).unwrap()))
        .unwrap())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ext::config_dirs::CommandSource;
    use crate::ext::skills::SkillIndex;
    use crate::ext::slash_commands::SlashCommand;

    fn make_slash_command(name: &str, desc: &str, arg_hint: &str) -> SlashCommand {
        SlashCommand {
            name: name.to_string(),
            description: desc.to_string(),
            argument_hint: arg_hint.to_string(),
            allowed_tools: vec![],
            model: None,
            body: String::new(),
            source: CommandSource::GlobalRefact,
            file_path: std::path::PathBuf::new(),
        }
    }

    fn make_skill(name: &str, desc: &str, user_invocable: bool) -> SkillIndex {
        SkillIndex {
            name: name.to_string(),
            description: desc.to_string(),
            user_invocable,
            disable_model_invocation: false,
            source: CommandSource::GlobalRefact,
        }
    }

    #[test]
    fn test_at_command_slash_completion_prefix_filter() {
        let commands = vec![
            make_slash_command("optimize", "Optimize code", "[file]"),
            make_slash_command("options", "Show options", ""),
            make_slash_command("review", "Review code", ""),
        ];
        let (completions, details) = slash_completions_for_prefix(&commands, &[], "/opt");
        assert_eq!(completions.len(), 2);
        assert!(completions.contains(&"/optimize".to_string()));
        assert!(completions.contains(&"/options".to_string()));
        assert!(!completions.contains(&"/review".to_string()));
        assert!(details.contains_key("/optimize"));
        assert!(details.contains_key("/options"));
    }

    #[test]
    fn test_at_command_slash_completion_all_for_slash() {
        let commands = vec![
            make_slash_command("optimize", "Optimize", ""),
            make_slash_command("review", "Review", ""),
        ];
        let (completions, _) = slash_completions_for_prefix(&commands, &[], "/");
        assert_eq!(completions.len(), 2);
        assert!(completions.contains(&"/optimize".to_string()));
        assert!(completions.contains(&"/review".to_string()));
    }

    #[test]
    fn test_at_command_slash_completion_excludes_non_invocable_skills() {
        let skills = vec![
            make_skill("public-skill", "Public", true),
            make_skill("private-skill", "Private", false),
        ];
        let (completions, _) = slash_completions_for_prefix(&[], &skills, "/");
        assert_eq!(completions.len(), 1);
        assert!(completions.contains(&"/public-skill".to_string()));
        assert!(!completions.contains(&"/private-skill".to_string()));
    }

    #[test]
    fn test_at_command_slash_completion_details_populated() {
        let commands = vec![make_slash_command("format", "Format code", "<file-path>")];
        let (completions, details) = slash_completions_for_prefix(&commands, &[], "/");
        assert_eq!(completions.len(), 1);
        let detail = details.get("/format").unwrap();
        assert_eq!(detail.description, "Format code");
        assert_eq!(detail.argument_hint, Some("<file-path>".to_string()));
        assert_eq!(detail.source, "global_refact");
    }

    #[test]
    fn test_at_command_slash_completion_empty_prefix_no_match() {
        let commands = vec![make_slash_command("optimize", "Optimize", "")];
        let (completions, _) = slash_completions_for_prefix(&commands, &[], "");
        assert!(completions.is_empty());
    }

    #[test]
    fn test_at_command_slash_completion_at_prefix_no_match() {
        let commands = vec![make_slash_command("file", "File command", "")];
        let (completions, _) = slash_completions_for_prefix(&commands, &[], "@file");
        assert!(completions.is_empty());
    }

    #[test]
    fn test_at_command_slash_completion_skills_included() {
        let skills = vec![make_skill("code-explainer", "Explains code", true)];
        let (completions, details) = slash_completions_for_prefix(&[], &skills, "/");
        assert_eq!(completions.len(), 1);
        assert!(completions.contains(&"/code-explainer".to_string()));
        assert!(details.contains_key("/code-explainer"));
    }

    #[test]
    fn test_at_command_slash_completion_hint_none_when_empty() {
        let commands = vec![make_slash_command("no-hint", "No hint", "")];
        let (_, details) = slash_completions_for_prefix(&commands, &[], "/");
        let detail = details.get("/no-hint").unwrap();
        assert!(detail.argument_hint.is_none());
    }

    fn make_arg(value: &str, pos1: i64, pos2: i64, focused: bool) -> QueryLineArg {
        QueryLineArg {
            value: value.to_string(),
            pos1,
            pos2,
            focused,
        }
    }

    #[test]
    fn test_slash_completion_triggered_mid_text() {
        let args = vec![
            make_arg("some", 0, 4, false),
            make_arg("text", 5, 9, false),
            make_arg("/greet", 10, 16, true),
        ];
        let focused_slash = args.iter().find(|a| a.focused && a.value.starts_with('/'));
        assert!(
            focused_slash.is_some(),
            "Any focused slash token should trigger completions"
        );
    }

    #[test]
    fn test_completion_details_keys_match_completions() {
        let commands = vec![
            make_slash_command("format", "Format code", "<file>"),
            make_slash_command("review", "Review code", ""),
        ];
        let (raw_completions, details) = slash_completions_for_prefix(&commands, &[], "/");
        let completions: Vec<_> = raw_completions
            .into_iter()
            .unique()
            .map(|x| {
                if x.starts_with('/') {
                    x
                } else {
                    format!("{} ", x)
                }
            })
            .collect();
        for c in &completions {
            assert!(
                details.contains_key(c.as_str()),
                "No detail found for completion '{}'",
                c
            );
        }
        assert_eq!(completions, vec!["/format", "/review"]);
    }

    #[test]
    fn test_at_command_slash_commands_list_response_format() {
        let cmd = SlashCommandInfo {
            name: "optimize".to_string(),
            description: "Optimize code".to_string(),
            argument_hint: Some("[file]".to_string()),
            source: "project_refact".to_string(),
        };
        let skill = SkillInfo {
            name: "explainer".to_string(),
            description: "Explains code".to_string(),
            user_invocable: true,
            source: "global_refact".to_string(),
        };
        let resp = SlashCommandsListResponse {
            commands: vec![cmd],
            skills: vec![skill],
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"commands\""));
        assert!(json.contains("\"skills\""));
        assert!(json.contains("\"optimize\""));
        assert!(json.contains("\"explainer\""));
    }
}
