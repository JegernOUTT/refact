use std::path::PathBuf;
use std::sync::Arc;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;
use tokio::sync::RwLock as ARwLock;
use async_trait::async_trait;
use axum::http::StatusCode;
use std::collections::HashMap;

use crate::subchat::{run_subchat_once_with_parent, resolve_subchat_params, resolve_subchat_model};
use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};
use crate::tools::tool_helpers::{load_code_subagent_config, CodeSubagentConfig};
use crate::tools::subagent_phases::{gather_files_phase, GatherFilesParams};
use crate::call_validation::{
    ChatMessage, ChatContent, ContextEnum, SubchatParameters, ContextFile, PostprocessSettings,
};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::caps::resolve_chat_model;
use crate::custom_error::ScratchError;
use crate::files_in_workspace::get_file_text_from_memory_or_disk;
use crate::global_context::{GlobalContext, try_load_caps_quickly_if_not_present};
use crate::postprocessing::pp_context_files::postprocess_context_files;
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::tokens::count_text_tokens_with_fallback;

pub struct ToolCodeReview {
    pub config_path: String,
}

static TOKENS_EXTRA_BUDGET_PERCENT: f32 = 0.06;

fn get_gather_files_params(config: &CodeSubagentConfig) -> GatherFilesParams<'_> {
    GatherFilesParams {
        default_subagent_id: "code_review_gather_files",
        title: "Code Review: Gathering Files",
        default_system_prompt: config.gather_system_prompt.as_deref().unwrap_or(""),
        user_instruction:
            "Based on the conversation above, identify all relevant files that need to be reviewed.",
    }
}

async fn make_review_prompt(
    gcx: Arc<ARwLock<GlobalContext>>,
    subchat_params: &SubchatParameters,
    important_paths: &[PathBuf],
    previous_messages: &[ChatMessage],
    config: &CodeSubagentConfig,
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

    let tokens_extra_budget =
        (subchat_params.subchat_n_ctx as f32 * TOKENS_EXTRA_BUDGET_PERCENT) as usize;
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

    let reviewer_prompt = config
        .reviewer_prompt
        .clone()
        .ok_or("reviewer_prompt not configured for code_review")?;

    let mut tokens_budget: i64 = (subchat_params.subchat_n_ctx - required_tokens) as i64;
    let final_message = reviewer_prompt.clone();
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
                tracing::warn!("code_review: failed to read file '{:?}'", p);
            }
        }
    }

    for message in previous_messages.iter().rev() {
        let message_row = match message.role.as_str() {
            "system" => continue,
            "user" => format!(
                "👤:\n{}\n\n",
                &message.content.to_text_with_image_placeholders()
            ),
            "assistant" => format!(
                "🤖:\n{}\n\n",
                &message.content.to_text_with_image_placeholders()
            ),
            "tool" => format!(
                "📎:\n{}\n\n",
                &message.content.to_text_with_image_placeholders()
            ),
            _ => continue,
        };
        let left_tokens =
            tokens_budget - count_text_tokens_with_fallback(tokenizer.clone(), &message_row) as i64;
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
        Ok(format!(
            "{final_message}\n\n# Conversation\n{context}\n\n# Files to Review\n{files_context}"
        ))
    } else {
        Ok(format!("{final_message}\n\n# Conversation\n{context}"))
    }
}

async fn execute_code_review(
    gcx: Arc<ARwLock<GlobalContext>>,
    ccx: Arc<AMutex<AtCommandsContext>>,
    important_paths: Vec<PathBuf>,
    external_messages: Vec<ChatMessage>,
    tool_call_id: String,
    config: &CodeSubagentConfig,
) -> Result<(String, serde_json::Map<String, serde_json::Value>), String> {
    let (subchat_tx, abort_flag, parent_depth, parent_task_meta, parent_worktree) = {
        let ccx_lock = ccx.lock().await;
        (
            ccx_lock.subchat_tx.clone(),
            ccx_lock.abort_flag.clone(),
            ccx_lock.subchat_depth,
            ccx_lock.task_meta.clone(),
            ccx_lock.execution_scope_worktree(),
        )
    };

    let subchat_params = resolve_subchat_params(gcx.clone(), "code_review").await?;

    let prompt = make_review_prompt(
        gcx.clone(),
        &subchat_params,
        &important_paths,
        &external_messages,
        config,
    )
    .await?;

    let history: Vec<ChatMessage> = vec![ChatMessage::new("user".to_string(), prompt)];

    let result = run_subchat_once_with_parent(
        gcx.clone(),
        "code_review",
        history,
        tool_call_id.clone(),
        subchat_tx,
        abort_flag,
        parent_depth,
        parent_task_meta,
        parent_worktree,
    )
    .await?;
    let review_response = result
        .messages
        .last()
        .cloned()
        .ok_or("No response from code review")?;

    let filenames: Vec<String> = important_paths
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();

    let files_section = format!(
        "# Files Reviewed ({})\n{}\n\n",
        filenames.len(),
        filenames
            .iter()
            .map(|f| format!("- {}", f))
            .collect::<Vec<_>>()
            .join("\n")
    );

    let review_content = format!(
        "{}# Code Review\n{}",
        files_section,
        review_response.content.to_text_with_image_placeholders()
    );
    let metering = result.metering;

    Ok((review_content, metering))
}

#[async_trait]
impl Tool for ToolCodeReview {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "code_review".to_string(),
            display_name: "Code Review".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Perform a thorough code review. Automatically identifies relevant files and checks for bugs, integration issues, missing tests, code style, and consistency.".to_string(),
            input_schema: json_schema_from_params(&[], &[]),
            output_schema: None,
            annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        _args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let gcx = ccx.lock().await.global_context.clone();

        let config = load_code_subagent_config(gcx.clone(), "code_review", None).await?;

        let external_messages = {
            let ccx_lock = ccx.lock().await;
            ccx_lock.messages.clone()
        };

        let gather_params = get_gather_files_params(&config);

        tracing::info!("code_review: phase 1 - gathering relevant files");
        let important_paths = gather_files_phase(
            gcx.clone(),
            ccx.clone(),
            external_messages.clone(),
            tool_call_id.clone(),
            &config,
            &gather_params,
        )
        .await?;

        tracing::info!(
            "code_review: phase 2 - performing review on {} files",
            important_paths.len()
        );

        let (final_message, metering) = execute_code_review(
            gcx,
            ccx.clone(),
            important_paths,
            external_messages,
            tool_call_id.clone(),
            &config,
        )
        .await?;

        let guardrails_prompt = config
            .guardrails_prompt
            .clone()
            .ok_or("guardrails_prompt not configured for code_review")?;

        Ok((
            false,
            vec![
                ContextEnum::ChatMessage(ChatMessage {
                    role: "tool".to_string(),
                    content: ChatContent::SimpleText(final_message),
                    tool_calls: None,
                    tool_call_id: tool_call_id.clone(),
                    usage: None,
                    extra: metering,
                    output_filter: Some(OutputFilter::no_limits()),
                    ..Default::default()
                }),
                ContextEnum::ChatMessage(ChatMessage {
                    role: "cd_instruction".to_string(),
                    content: ChatContent::SimpleText(guardrails_prompt),
                    ..Default::default()
                }),
            ],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}
