use std::collections::HashMap;
use std::path::PathBuf;
use std::string::ToString;
use std::sync::Arc;
use serde_json::{Value, json};
use tokio::sync::Mutex as AMutex;
use tokio::sync::RwLock as ARwLock;
use async_trait::async_trait;
use axum::http::StatusCode;
use crate::subchat::{run_subchat_once, resolve_subchat_params, resolve_subchat_model};
use crate::tools::tools_description::{
    Tool, ToolDesc, ToolParam, ToolSource, ToolSourceType,
};
use crate::call_validation::{
    ChatMessage, ChatContent, ChatUsage, ContextEnum, SubchatParameters, ContextFile,
    PostprocessSettings,
};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_commands::at_file::{file_repair_candidates, return_one_candidate_or_a_good_error};
use crate::caps::resolve_chat_model;
use crate::custom_error::ScratchError;
use crate::files_correction::{
    canonicalize_normalized_path, get_project_dirs_with_code_workdir, preprocess_path_for_normalization,
};
use crate::files_in_workspace::get_file_text_from_memory_or_disk;
use crate::global_context::{GlobalContext, try_load_caps_quickly_if_not_present};
use crate::postprocessing::pp_context_files::postprocess_context_files;
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::tokens::count_text_tokens_with_fallback;
use crate::memories::{memories_add_enriched, EnrichmentParams};

pub struct ToolStrategicPlanning {
    pub config_path: String,
}

static TOKENS_EXTRA_BUDGET_PERCENT: f32 = 0.06;

static SOLVER_PROMPT: &str = r#"Your task is to identify and solve the problem by the given conversation and context files.
The solution must be robust and complete and adressing all corner cases.
Also make a couple of alternative ways to solve the problem, if the initial solution doesn't work."#;

static GUARDRAILS_PROMPT: &str = r#"💿 Now confirm the plan with the user"#;

static ENTERTAINMENT_MESSAGES: &[&str] = &[
    "1/7: 🧠 Strategic planning in progress...",
    "2/7: 📋 Analyzing the problem and context...",
    "3/7: 🔍 Reviewing relevant files...",
    "4/7: 💡 Formulating solution approaches...",
    "5/7: 📝 Drafting the strategic plan...",
    "6/7: ⏳ Still working... Almost there!",
    "7/7: 🔄 Refining the solution...",
];

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
    tracing::info!("strategic_planning: sending entertainment message: tool_call_id={}, subchat_id={}", tool_call_id, message_text);
    match subchat_tx.lock().await.send(entertainment_msg) {
        Ok(_) => tracing::info!("strategic_planning: entertainment message sent successfully"),
        Err(e) => tracing::error!("strategic_planning: failed to send entertainment message: {}", e),
    }
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

async fn _make_prompt(
    ccx: Arc<AMutex<AtCommandsContext>>,
    subchat_params: &SubchatParameters,
    problem_statement: &String,
    important_paths: &Vec<PathBuf>,
    previous_messages: &Vec<ChatMessage>,
) -> Result<String, String> {
    let gcx = ccx.lock().await.global_context.clone();
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
            "Bad subchat budget for strategic_planning: max_new_tokens({}) + tokens_for_rag({}) + extra({}) = {} >= n_ctx({})",
            subchat_params.subchat_max_new_tokens,
            subchat_params.subchat_tokens_for_rag,
            tokens_extra_budget,
            required_tokens,
            subchat_params.subchat_n_ctx
        ));
    }
    let mut tokens_budget: i64 = (subchat_params.subchat_n_ctx - required_tokens) as i64;
    let final_message = problem_statement.to_string();
    tokens_budget -= count_text_tokens_with_fallback(tokenizer.clone(), &final_message) as i64;
    let mut context = "".to_string();
    let mut context_files = vec![];
    for p in important_paths.iter() {
        context_files.push(
            match get_file_text_from_memory_or_disk(gcx.clone(), &p).await {
                Ok(text) => {
                    let total_lines = text.lines().count();
                    tracing::info!("adding file '{:?}' to the context", p);
                    ContextFile {
                        file_name: p.to_string_lossy().to_string(),
                        file_content: "".to_string(),
                        line1: 1,
                        line2: total_lines.max(1),
                        file_rev: None,
                        symbols: vec![],
                        gradient_type: 4,
                        usefulness: 100.0,
                        skip_pp: false,
                    }
                }
                Err(_) => {
                    tracing::warn!("failed to read file '{:?}'. Skipping...", p);
                    continue;
                }
            },
        )
    }
    for message in previous_messages.iter().rev() {
        let message_row = match message.role.as_str() {
            "system" => {
                // just skipping it
                continue;
            }
            "user" => {
                format!("👤:\n{}\n\n", &message.content.content_text_only())
            }
            "assistant" => {
                format!("🤖:\n{}\n\n", &message.content.content_text_only())
            }
            "tool" => {
                format!("📎:\n{}\n\n", &message.content.content_text_only())
            }
            _ => {
                tracing::info!(
                    "skip adding message to the context: {}",
                    crate::nicer_logs::first_n_chars(&message.content.content_text_only(), 40)
                );
                continue;
            }
        };
        let left_tokens =
            tokens_budget - count_text_tokens_with_fallback(tokenizer.clone(), &message_row) as i64;
        if left_tokens < 0 {
            // we do not end here, maybe there are smaller useful messages at the beginning
            continue;
        } else {
            tokens_budget = left_tokens;
            context.insert_str(0, &message_row);
        }
    }
    if !context_files.is_empty() {
        let mut pp_settings = PostprocessSettings::new();
        pp_settings.max_files_n = context_files.len();
        let mut files_context = "".to_string();
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
            "{final_message}\n\n# Conversation\n{context}\n\n# Files context\n{files_context}"
        ))
    } else {
        Ok(format!("{final_message}\n\n# Conversation\n{context}"))
    }
}

async fn _execute_subchat_iteration(
    gcx: Arc<ARwLock<GlobalContext>>,
    history: Vec<ChatMessage>,
) -> Result<(Vec<ChatMessage>, ChatMessage, ChatUsage), String> {
    let result = run_subchat_once(gcx, "strategic_planning", history).await?;

    let reply = result.messages.last().cloned()
        .ok_or("No response from strategic planning")?;

    Ok((result.messages, reply, result.usage))
}

async fn execute_strategic_planning(
    gcx: Arc<ARwLock<GlobalContext>>,
    ccx_subchat: Arc<AMutex<AtCommandsContext>>,
    important_paths: Vec<PathBuf>,
    external_messages: Vec<ChatMessage>,
    tool_call_id: String,
) -> Result<(String, ChatUsage), String> {
    let subchat_tx = ccx_subchat.lock().await.subchat_tx.clone();

    send_entertainment_message(&subchat_tx, &tool_call_id, 0).await;
    let cancel_token = tokio_util::sync::CancellationToken::new();
    spawn_entertainment_task(subchat_tx, tool_call_id.clone(), cancel_token.clone());

    let subchat_params = resolve_subchat_params(gcx.clone(), "strategic_planning").await?;

    let ccx_for_prompt = {
        let ccx_lock = ccx_subchat.lock().await;
        Arc::new(AMutex::new(AtCommandsContext::new(
            ccx_lock.global_context.clone(),
            subchat_params.subchat_n_ctx,
            0,
            false,
            external_messages.clone(),
            ccx_lock.chat_id.clone(),
            ccx_lock.should_execute_remotely,
            ccx_lock.current_model.clone(),
            ccx_lock.task_meta.clone(), None,
        ).await))
    };

    let prompt = _make_prompt(
        ccx_for_prompt,
        &subchat_params,
        &SOLVER_PROMPT.to_string(),
        &important_paths,
        &external_messages,
    )
    .await?;
    let history: Vec<ChatMessage> = vec![ChatMessage::new("user".to_string(), prompt)];

    tracing::info!("FIRST ITERATION: Get the initial solution");
    let result = _execute_subchat_iteration(gcx.clone(), history.clone()).await;

    cancel_token.cancel();

    let (_, initial_solution, usage_collector) = result?;
    let solution_content = format!(
        "# Solution\n{}",
        initial_solution.content.content_text_only()
    );
    tracing::info!(
        "strategic planning response (combined):\n{}",
        solution_content
    );

    let filenames: Vec<String> = important_paths
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    let enrichment_params = EnrichmentParams {
        base_tags: vec!["planning".to_string(), "strategic".to_string()],
        base_filenames: filenames,
        base_kind: "decision".to_string(),
        base_title: Some("Strategic Plan".to_string()),
    };

    let memory_note = match memories_add_enriched(ccx_subchat.clone(), &solution_content, enrichment_params).await {
        Ok(path) => {
            tracing::info!(
                "Created enriched memory from strategic planning: {:?}",
                path
            );
            format!(
                "\n\n---\n📝 **This plan has been saved to the knowledge base:** `{}`",
                path.display()
            )
        }
        Err(e) => {
            tracing::warn!(
                "Failed to create enriched memory from strategic planning: {}",
                e
            );
            String::new()
        }
    };
    let final_message = format!("{}{}", solution_content, memory_note);

    Ok((final_message, usage_collector))
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
            description: "Strategically plan a solution for a complex problem or create a comprehensive approach.".to_string(),
            parameters: vec![
                ToolParam {
                    name: "important_paths".to_string(),
                    param_type: "string".to_string(),
                    description: "Comma-separated list of all filenames which are required to be considered for resolving the problem. More files - better, include them even if you are not sure.".to_string(),
                }
            ],
            parameters_required: vec!["important_paths".to_string()],
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (gcx, code_workdir) = {
            let ccx_locked = ccx.lock().await;
            (ccx_locked.global_context.clone(), ccx_locked.code_workdir.clone())
        };
        let project_dirs = get_project_dirs_with_code_workdir(gcx.clone(), &code_workdir).await;
        let important_paths = match args.get("important_paths") {
            Some(Value::String(s)) => {
                let mut paths = vec![];
                for s in s.split(",") {
                    let s_raw = s.trim().to_string();
                    let candidates_file =
                        file_repair_candidates(gcx.clone(), &s_raw, 3, false).await;
                    paths.push(
                        match return_one_candidate_or_a_good_error(
                            gcx.clone(),
                            &s_raw,
                            &candidates_file,
                            &project_dirs,
                            false,
                        )
                        .await
                        {
                            Ok(f) => canonicalize_normalized_path(PathBuf::from(
                                preprocess_path_for_normalization(f.trim().to_string()),
                            )),
                            Err(_) => {
                                tracing::info!("cannot find a good file candidate for `{s_raw}`");
                                continue;
                            }
                        },
                    )
                }
                paths
            }
            Some(v) => return Err(format!("argument `important_paths` is not a string: {:?}", v)),
            None => return Err("Missing argument `important_paths`".to_string()),
        };

        if important_paths.is_empty() {
            return Err("No valid files resolved from `important_paths`. Please provide existing file paths.".to_string());
        }

        let external_messages = {
            let ccx_lock = ccx.lock().await;
            ccx_lock.messages.clone()
        };

        tracing::info!("Starting strategic planning for {} files", important_paths.len());

        let (final_message, usage_collector) = execute_strategic_planning(
            gcx,
            ccx.clone(),
            important_paths.clone(),
            external_messages,
            tool_call_id.clone(),
        ).await?;

        Ok((false, vec![
            ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(final_message),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                usage: Some(usage_collector),
                output_filter: Some(OutputFilter::no_limits()),
                ..Default::default()
            }),
            ContextEnum::ChatMessage(ChatMessage {
                role: "cd_instruction".to_string(),
                content: ChatContent::SimpleText(GUARDRAILS_PROMPT.to_string()),
                ..Default::default()
            }),
        ]))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}
