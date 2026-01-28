use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use serde_json::{Value, json};
use tokio::sync::Mutex as AMutex;
use tokio::sync::RwLock as ARwLock;
use async_trait::async_trait;
use axum::http::StatusCode;
use std::collections::HashMap;

use crate::subchat::{run_subchat, run_subchat_once, resolve_subchat_params, resolve_subchat_model, resolve_subchat_config_with_parent};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};
use crate::call_validation::{
    ChatMessage, ChatContent, ChatUsage, ContextEnum, SubchatParameters, ContextFile,
    PostprocessSettings,
};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::caps::resolve_chat_model;
use crate::custom_error::ScratchError;
use crate::files_correction::correct_to_nearest_filename;
use crate::files_in_workspace::get_file_text_from_memory_or_disk;
use crate::global_context::{GlobalContext, try_load_caps_quickly_if_not_present};
use crate::postprocessing::pp_context_files::postprocess_context_files;
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::tokens::count_text_tokens_with_fallback;
use crate::memories::{memories_add_enriched, EnrichmentParams};

pub struct ToolStrategicPlanning {
    pub config_path: String,
}

const MAX_FILES: usize = 30;
const GATHER_FILES_MAX_STEPS: usize = 10;
static TOKENS_EXTRA_BUDGET_PERCENT: f32 = 0.06;

static GATHER_FILES_SYSTEM_PROMPT: &str = r#"You are a focused sub-agent that identifies relevant files for strategic planning.

Your task:
1. Analyze the conversation to understand the problem
2. Use the available tools to explore the codebase and find all relevant files
3. Read key files to understand their purpose

Consider:
- Files explicitly mentioned in the conversation
- Files that would need to be modified to solve the problem
- Related test files
- Configuration files that might be affected
- Dependencies and imports
- Similar patterns from past solutions (use knowledge tool)

After your investigation, output your final recommendations in this EXACT format:

RELEVANT_FILES:
path/to/file1.ext
path/to/file2.ext
END_FILES

Include up to 30 most important files, prioritized by relevance to solving the problem.
Only include files that actually exist and that you've verified."#;

static GATHER_FILES_RETRY_PROMPT: &str = r#"Your response was not in the required format. Please output the list of relevant files in this EXACT format:

RELEVANT_FILES:
path/to/file1.ext
path/to/file2.ext
END_FILES

Include only the files you found during your investigation."#;

static SOLVER_PROMPT: &str = r#"Your task is to identify and solve the problem by the given conversation and context files.
The solution must be robust and complete and addressing all corner cases.
Also make a couple of alternative ways to solve the problem, if the initial solution doesn't work."#;

static GUARDRAILS_PROMPT: &str = r#"💿 Now confirm the plan with the user"#;

static ENTERTAINMENT_MESSAGES: &[&str] = &[
    "1/4: 📋 Gathering context from files...",
    "2/4: 💡 Formulating solution approaches...",
    "3/4: 📝 Drafting the strategic plan...",
    "4/4: 🔄 Refining the solution...",
];

static GATHER_FILES_TOOLS: &[&str] = &[
    "tree",
    "cat",
    "search_pattern",
    "search_symbol_definition",
    "search_semantic",
    "knowledge",
];

async fn send_files_gathered_message(
    subchat_tx: &Arc<AMutex<tokio::sync::mpsc::UnboundedSender<serde_json::Value>>>,
    tool_call_id: &str,
    files: &[PathBuf],
) {
    let file_names: Vec<String> = files.iter().map(|p| p.to_string_lossy().to_string()).collect();
    let files_preview = if file_names.len() <= 3 {
        file_names.join(", ")
    } else {
        format!("{}, …", file_names[..3].join(", "))
    };
    let message_text = format!("📁 {} files: {}", file_names.len(), files_preview);
    let msg = json!({
        "tool_call_id": tool_call_id,
        "subchat_id": message_text,
        "add_message": {
            "role": "assistant",
            "content": message_text
        }
    });
    let _ = subchat_tx.lock().await.send(msg);
}

async fn send_entertainment_message(
    subchat_tx: &Arc<AMutex<tokio::sync::mpsc::UnboundedSender<serde_json::Value>>>,
    tool_call_id: &str,
    message_idx: usize,
) {
    let message_text = ENTERTAINMENT_MESSAGES[message_idx % ENTERTAINMENT_MESSAGES.len()];
    let entertainment_msg = json!({
        "tool_call_id": tool_call_id,
        "subchat_id": message_text,
        "add_message": {
            "role": "assistant",
            "content": message_text
        }
    });
    let _ = subchat_tx.lock().await.send(entertainment_msg);
}

fn spawn_entertainment_task(
    subchat_tx: Arc<AMutex<tokio::sync::mpsc::UnboundedSender<serde_json::Value>>>,
    tool_call_id: String,
    cancel_token: tokio_util::sync::CancellationToken,
) {
    tokio::spawn(async move {
        let mut message_idx = 0usize;
        loop {
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    break;
                }
                _ = tokio::time::sleep(tokio::time::Duration::from_secs(10)) => {
                    send_entertainment_message(&subchat_tx, &tool_call_id, message_idx).await;
                    message_idx += 1;
                }
            }
        }
    });
}

fn parse_relevant_files_from_response(response: &str) -> Vec<String> {
    let mut files = Vec::new();
    let mut in_files_block = false;

    for line in response.lines() {
        let trimmed = line.trim();
        if trimmed == "RELEVANT_FILES:" {
            in_files_block = true;
            continue;
        }
        if trimmed == "END_FILES" {
            break;
        }
        if in_files_block && !trimmed.is_empty() && !trimmed.starts_with('#') && !trimmed.starts_with("//") {
            files.push(trimmed.to_string());
        }
    }

    files.truncate(MAX_FILES);
    files
}

fn get_last_assistant_content(messages: &[ChatMessage]) -> String {
    messages
        .iter()
        .rev()
        .find(|m| m.role == "assistant")
        .map(|m| m.content.to_text_with_image_placeholders())
        .unwrap_or_default()
}

async fn gather_relevant_files(
    gcx: Arc<ARwLock<GlobalContext>>,
    ccx: Arc<AMutex<AtCommandsContext>>,
    external_messages: Vec<ChatMessage>,
    tool_call_id: String,
) -> Result<(Vec<PathBuf>, ChatUsage), String> {
    let (parent_chat_id, parent_root_chat_id, parent_subchat_tx, parent_abort_flag) = {
        let ccx_lock = ccx.lock().await;
        (
            ccx_lock.chat_id.clone(),
            ccx_lock.root_chat_id.clone(),
            ccx_lock.subchat_tx.clone(),
            ccx_lock.abort_flag.clone(),
        )
    };

    let tools: Vec<String> = GATHER_FILES_TOOLS.iter().map(|s| s.to_string()).collect();

    let config = resolve_subchat_config_with_parent(
        gcx.clone(),
        "strategic_planning_gather_files",
        true,
        None,
        Some("Strategic Planning: Gathering Files".to_string()),
        Some(parent_chat_id),
        Some("gather_files".to_string()),
        Some(parent_root_chat_id),
        Some(tools),
        GATHER_FILES_MAX_STEPS,
        false,
        None,
        Some(tool_call_id.clone()),
        Some(parent_subchat_tx.clone()),
        Some(parent_abort_flag),
    )
    .await?;

    let mut messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: ChatContent::SimpleText(GATHER_FILES_SYSTEM_PROMPT.to_string()),
            ..Default::default()
        },
    ];

    for msg in external_messages.iter() {
        if msg.role == "user" || msg.role == "assistant" || msg.role == "tool" {
            messages.push(msg.clone());
        }
    }

    messages.push(ChatMessage {
        role: "user".to_string(),
        content: ChatContent::SimpleText(
            "Based on the conversation above, identify all relevant files for solving this problem.".to_string()
        ),
        ..Default::default()
    });

    tracing::info!("strategic_planning: starting file-gathering subagent");
    let result = run_subchat(gcx.clone(), messages.clone(), config).await?;

    let response = get_last_assistant_content(&result.messages);
    let mut files = parse_relevant_files_from_response(&response);

    if files.is_empty() {
        tracing::info!("strategic_planning: file list not properly formatted, requesting retry");

        let mut retry_messages = result.messages.clone();
        retry_messages.push(ChatMessage {
            role: "user".to_string(),
            content: ChatContent::SimpleText(GATHER_FILES_RETRY_PROMPT.to_string()),
            ..Default::default()
        });

        let retry_result = run_subchat_once(gcx.clone(), "strategic_planning_gather_files", retry_messages).await?;
        let retry_response = get_last_assistant_content(&retry_result.messages);
        files = parse_relevant_files_from_response(&retry_response);

        if files.is_empty() {
            return Err("File-gathering subagent failed to provide a valid file list".to_string());
        }
    }

    tracing::info!("strategic_planning: gathered {} files", files.len());

    let mut valid_paths = Vec::new();
    let mut seen = HashSet::new();
    for file_str in files {
        let candidates = correct_to_nearest_filename(gcx.clone(), &file_str, false, 1).await;
        if let Some(corrected) = candidates.first() {
            let path = PathBuf::from(corrected);
            if !seen.contains(&path) {
                seen.insert(path.clone());
                valid_paths.push(path);
            }
        } else {
            tracing::warn!("strategic_planning: skipping invalid path: {}", file_str);
        }
    }

    if valid_paths.is_empty() {
        return Err("No valid files found from the gathered list".to_string());
    }

    send_files_gathered_message(&parent_subchat_tx, &tool_call_id, &valid_paths).await;

    Ok((valid_paths, result.usage))
}

async fn make_planning_prompt(
    gcx: Arc<ARwLock<GlobalContext>>,
    subchat_params: &SubchatParameters,
    important_paths: &[PathBuf],
    previous_messages: &[ChatMessage],
) -> Result<String, String> {
    let caps = try_load_caps_quickly_if_not_present(gcx.clone(), 0)
        .await
        .map_err(|x| x.message)?;
    let model_id = resolve_subchat_model(gcx.clone(), subchat_params).await?;
    let model_rec = resolve_chat_model(caps, &model_id)?;
    let tokenizer = crate::tokens::cached_tokenizer(gcx.clone(), &model_rec.base)
        .await
        .map_err(|e| ScratchError::new(StatusCode::INTERNAL_SERVER_ERROR, e))
        .map_err(|x| x.message)?;

    let tokens_extra_budget = (subchat_params.subchat_n_ctx as f32 * TOKENS_EXTRA_BUDGET_PERCENT) as usize;
    let required_tokens = subchat_params.subchat_max_new_tokens
        + subchat_params.subchat_tokens_for_rag
        + tokens_extra_budget;

    if required_tokens >= subchat_params.subchat_n_ctx {
        return Err(format!(
            "Bad subchat budget: max_new_tokens({}) + tokens_for_rag({}) + extra({}) = {} >= n_ctx({})",
            subchat_params.subchat_max_new_tokens,
            subchat_params.subchat_tokens_for_rag,
            tokens_extra_budget,
            required_tokens,
            subchat_params.subchat_n_ctx
        ));
    }

    let mut tokens_budget: i64 = (subchat_params.subchat_n_ctx - required_tokens) as i64;
    let final_message = SOLVER_PROMPT.to_string();
    tokens_budget -= count_text_tokens_with_fallback(tokenizer.clone(), &final_message) as i64;

    let mut context = String::new();
    let mut context_files = vec![];

    for p in important_paths.iter() {
        match get_file_text_from_memory_or_disk(gcx.clone(), p).await {
            Ok(text) => {
                let total_lines = text.lines().count();
                context_files.push(ContextFile {
                    file_name: p.to_string_lossy().to_string(),
                    file_content: String::new(),
                    line1: 1,
                    line2: total_lines.max(1),
                    file_rev: None,
                    symbols: vec![],
                    gradient_type: 4,
                    usefulness: 100.0,
                    skip_pp: false,
                });
            }
            Err(_) => {
                tracing::warn!("strategic_planning: failed to read file '{:?}'", p);
            }
        }
    }

    for message in previous_messages.iter().rev() {
        let message_row = match message.role.as_str() {
            "system" => continue,
            "user" => format!("👤:\n{}\n\n", &message.content.to_text_with_image_placeholders()),
            "assistant" => format!("🤖:\n{}\n\n", &message.content.to_text_with_image_placeholders()),
            "tool" => format!("📎:\n{}\n\n", &message.content.to_text_with_image_placeholders()),
            _ => continue,
        };
        let left_tokens = tokens_budget - count_text_tokens_with_fallback(tokenizer.clone(), &message_row) as i64;
        if left_tokens >= 0 {
            tokens_budget = left_tokens;
            context.insert_str(0, &message_row);
        }
    }

    if !context_files.is_empty() {
        let mut pp_settings = PostprocessSettings::new();
        pp_settings.max_files_n = context_files.len();
        let mut files_context = String::new();
        let (pp_files, _notes) = postprocess_context_files(
            gcx.clone(),
            &mut context_files,
            tokenizer.clone(),
            subchat_params.subchat_tokens_for_rag + tokens_budget.max(0) as usize,
            false,
            &pp_settings,
        )
        .await;

        for context_file in pp_files {
            files_context.push_str(&format!(
                "📎 {}:{}-{}\n```\n{}```\n\n",
                context_file.file_name,
                context_file.line1,
                context_file.line2,
                context_file.file_content
            ));
        }
        Ok(format!("{final_message}\n\n# Conversation\n{context}\n\n# Files context\n{files_context}"))
    } else {
        Ok(format!("{final_message}\n\n# Conversation\n{context}"))
    }
}

async fn execute_strategic_planning(
    gcx: Arc<ARwLock<GlobalContext>>,
    ccx: Arc<AMutex<AtCommandsContext>>,
    important_paths: Vec<PathBuf>,
    external_messages: Vec<ChatMessage>,
    tool_call_id: String,
) -> Result<(String, ChatUsage, serde_json::Map<String, serde_json::Value>), String> {
    let subchat_tx = ccx.lock().await.subchat_tx.clone();

    send_entertainment_message(&subchat_tx, &tool_call_id, 0).await;
    let cancel_token = tokio_util::sync::CancellationToken::new();
    spawn_entertainment_task(subchat_tx, tool_call_id.clone(), cancel_token.clone());

    let subchat_params = resolve_subchat_params(gcx.clone(), "strategic_planning").await?;

    let prompt = make_planning_prompt(
        gcx.clone(),
        &subchat_params,
        &important_paths,
        &external_messages,
    )
    .await?;

    let history: Vec<ChatMessage> = vec![ChatMessage::new("user".to_string(), prompt)];

    let result = run_subchat_once(gcx.clone(), "strategic_planning", history).await;

    cancel_token.cancel();

    let result = result?;
    let initial_solution = result
        .messages
        .last()
        .cloned()
        .ok_or("No response from strategic planning")?;

    let filenames: Vec<String> = important_paths
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    let files_section = format!(
        "# Files Analyzed ({})\n{}\n\n",
        filenames.len(),
        filenames.iter().map(|f| format!("- {}", f)).collect::<Vec<_>>().join("\n")
    );

    let solution_content = format!("{}# Solution\n{}", files_section, initial_solution.content.to_text_with_image_placeholders());

    let enrichment_params = EnrichmentParams {
        base_tags: vec!["planning".to_string(), "strategic".to_string()],
        base_filenames: filenames,
        base_kind: "decision".to_string(),
        base_title: Some("Strategic Plan".to_string()),
    };

    let memory_note = match memories_add_enriched(ccx.clone(), &solution_content, enrichment_params).await {
        Ok(path) => {
            format!("\n\n---\n📝 **This plan has been saved to the knowledge base:** `{}`", path.display())
        }
        Err(e) => {
            tracing::warn!("strategic_planning: failed to save memory: {}", e);
            String::new()
        }
    };

    let final_message = format!("{}{}", solution_content, memory_note);
    let metering = result.metering;

    Ok((final_message, result.usage, metering))
}

#[async_trait]
impl Tool for ToolStrategicPlanning {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "strategic_planning".to_string(),
            display_name: "Strategic Planning".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            agentic: true,
            experimental: false,
            description: "Strategically plan a solution for a complex problem or create a comprehensive approach. Automatically identifies relevant files from the codebase.".to_string(),
            parameters: vec![],
            parameters_required: vec![],
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        _args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let gcx = ccx.lock().await.global_context.clone();

        let external_messages = {
            let ccx_lock = ccx.lock().await;
            ccx_lock.messages.clone()
        };

        tracing::info!("strategic_planning: phase 1 - gathering relevant files");
        let (important_paths, gather_usage) = gather_relevant_files(
            gcx.clone(),
            ccx.clone(),
            external_messages.clone(),
            tool_call_id.clone(),
        )
        .await?;

        tracing::info!(
            "strategic_planning: phase 2 - creating plan with {} files",
            important_paths.len()
        );

        let (final_message, plan_usage, metering) = execute_strategic_planning(
            gcx,
            ccx.clone(),
            important_paths,
            external_messages,
            tool_call_id.clone(),
        )
        .await?;

        let combined_usage = ChatUsage {
            prompt_tokens: gather_usage.prompt_tokens + plan_usage.prompt_tokens,
            completion_tokens: gather_usage.completion_tokens + plan_usage.completion_tokens,
            total_tokens: gather_usage.total_tokens + plan_usage.total_tokens,
            ..Default::default()
        };

        Ok((
            false,
            vec![
                ContextEnum::ChatMessage(ChatMessage {
                    role: "tool".to_string(),
                    content: ChatContent::SimpleText(final_message),
                    tool_calls: None,
                    tool_call_id: tool_call_id.clone(),
                    usage: Some(combined_usage),
                    extra: metering,
                    output_filter: Some(OutputFilter::no_limits()),
                    ..Default::default()
                }),
                ContextEnum::ChatMessage(ChatMessage {
                    role: "cd_instruction".to_string(),
                    content: ChatContent::SimpleText(GUARDRAILS_PROMPT.to_string()),
                    ..Default::default()
                }),
            ],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}
