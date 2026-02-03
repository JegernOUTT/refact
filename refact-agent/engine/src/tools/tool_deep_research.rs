use std::collections::HashMap;
use std::sync::Arc;
use serde_json::{Value, json};
use tokio::sync::Mutex as AMutex;
use async_trait::async_trait;

use crate::subchat::run_subchat_once;
use crate::tools::tools_description::{
    Tool, ToolDesc, ToolParam, ToolSource, ToolSourceType, MatchConfirmDeny, MatchConfirmDenyResult,
};
use crate::call_validation::{ChatMessage, ChatContent, ChatUsage, ContextEnum};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::global_context::GlobalContext;
use crate::yaml_configs::customization_registry::get_subagent_config;
use tokio::sync::RwLock as ARwLock;
use crate::integrations::integr_abstract::IntegrationConfirmation;
use crate::memories::{memories_add_enriched, EnrichmentParams};
use crate::postprocessing::pp_command_output::OutputFilter;

const SUBAGENT_ID: &str = "deep_research";

pub struct ToolDeepResearch {
    pub config_path: String,
}

static ENTERTAINMENT_MESSAGES: &[&str] = &[
    "1/9: 🔬 Deep research in progress... This may take up to 30 minutes, please be patient!",
    "2/9: 🌐 Browsing the web and gathering relevant sources...",
    "3/9: 📚 Reading through documentation and articles...",
    "4/9: 🔍 Cross-referencing information from multiple sources...",
    "5/9: 🧠 Analyzing and synthesizing the findings...",
    "6/9: 📊 Organizing data and preparing insights...",
    "7/9: ✍️ Composing comprehensive report with citations...",
    "8/9: ⏳ Still working... Almost there!",
    "9/9: 🔄 Continuing deep research... Thank you for your patience!",
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
    tracing::info!(
        "deep_research: sending entertainment message: tool_call_id={}, subchat_id={}",
        tool_call_id,
        message_text
    );
    match subchat_tx.lock().await.send(entertainment_msg) {
        Ok(_) => tracing::info!("deep_research: entertainment message sent successfully"),
        Err(e) => tracing::error!("deep_research: failed to send entertainment message: {}", e),
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

async fn execute_deep_research(
    gcx: Arc<ARwLock<GlobalContext>>,
    subchat_tx: Arc<AMutex<tokio::sync::mpsc::UnboundedSender<serde_json::Value>>>,
    research_query: String,
    tool_call_id: String,
) -> Result<
    (
        ChatMessage,
        ChatUsage,
        serde_json::Map<String, serde_json::Value>,
    ),
    String,
> {
    send_entertainment_message(&subchat_tx, &tool_call_id, 0).await;

    let cancel_token = tokio_util::sync::CancellationToken::new();
    spawn_entertainment_task(subchat_tx, tool_call_id.clone(), cancel_token.clone());

    let subagent_config = get_subagent_config(gcx.clone(), SUBAGENT_ID, None)
        .await
        .ok_or_else(|| format!("subagent config '{}' not found", SUBAGENT_ID))?;

    let researcher_prompt = subagent_config.messages.user_template
        .as_ref()
        .ok_or_else(|| format!("messages.user_template not defined for subagent '{}'", SUBAGENT_ID))?;

    let messages = vec![
        ChatMessage::new("user".to_string(), researcher_prompt.clone()),
        ChatMessage::new("user".to_string(), research_query),
    ];

    let result = run_subchat_once(gcx, SUBAGENT_ID, messages).await;

    cancel_token.cancel();

    let subchat_result = result?;
    let reply = subchat_result
        .messages
        .last()
        .cloned()
        .ok_or("No response from deep research")?;

    Ok((reply, subchat_result.usage, subchat_result.metering))
}

#[async_trait]
impl Tool for ToolDeepResearch {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "deep_research".to_string(),
            display_name: "Deep Research".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Conduct comprehensive web research on a topic. Use this tool when you need up-to-date information from the internet, market analysis, technical documentation research, or synthesis of information from multiple web sources. The research takes several minutes and produces a detailed, citation-rich report. Do NOT use for questions about the current codebase - use code exploration tools instead.".to_string(),
            parameters: vec![
                ToolParam {
                    name: "research_query".to_string(),
                    param_type: "string".to_string(),
                    description: "A detailed research question or topic. Be specific: include the scope, what comparisons or metrics you need, any preferred sources, and the desired output format. Example: 'Research the current best practices for Rust async error handling in 2024, comparing tokio vs async-std approaches, with code examples and performance considerations.'".to_string(),
                }
            ],
            parameters_required: vec!["research_query".to_string()],
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let research_query = match args.get("research_query") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => {
                return Err(format!(
                    "argument `research_query` is not a string: {:?}",
                    v
                ))
            }
            None => return Err("Missing argument `research_query`".to_string()),
        };

        let (gcx, subchat_tx) = {
            let ccx_lock = ccx.lock().await;
            (ccx_lock.global_context.clone(), ccx_lock.subchat_tx.clone())
        };

        tracing::info!("Starting deep research for query: {}", research_query);

        let (research_result, usage_collector, metering) = execute_deep_research(
            gcx,
            subchat_tx,
            research_query.clone(),
            tool_call_id.clone(),
        )
        .await?;

        let research_content = format!(
            "# Deep Research Report\n\n{}",
            research_result.content.content_text_only()
        );

        let title = if research_query.len() > 80 {
            format!("{}...", &research_query[..80])
        } else {
            research_query.clone()
        };
        let root_chat_id = ccx.lock().await.root_chat_id.clone();
        let enrichment_params = EnrichmentParams {
            base_tags: vec!["research".to_string(), "deep-research".to_string()],
            base_filenames: vec![],
            base_kind: "research".to_string(),
            base_title: Some(title),
            source_chat_id: (!root_chat_id.is_empty()).then_some(root_chat_id),
        };
        let memory_note =
            match memories_add_enriched(ccx.clone(), &research_content, enrichment_params).await {
                Ok(path) => {
                    tracing::info!("Created enriched memory from deep research: {:?}", path);
                    format!(
                        "\n\n---\n📝 **This report has been saved to the knowledge base:** `{}`",
                        path.display()
                    )
                }
                Err(e) => {
                    tracing::warn!("Failed to create enriched memory from deep research: {}", e);
                    String::new()
                }
            };
        let final_message = format!("{}{}", research_content, memory_note);

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(final_message),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                usage: Some(usage_collector),
                extra: metering,
                output_filter: Some(OutputFilter::no_limits()),
                ..Default::default()
            })],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }

    async fn command_to_match_against_confirm_deny(
        &self,
        _ccx: Arc<AMutex<AtCommandsContext>>,
        args: &HashMap<String, Value>,
    ) -> Result<String, String> {
        let query = match args.get("research_query") {
            Some(Value::String(s)) => s.clone(),
            _ => return Ok("".to_string()),
        };
        let truncated_query = if query.len() > 100 {
            let end = query
                .char_indices()
                .take_while(|(i, _)| *i < 100)
                .last()
                .map(|(i, c)| i + c.len_utf8())
                .unwrap_or(100.min(query.len()));
            format!("{}...", &query[..end])
        } else {
            query
        };
        Ok(format!("deep_research \"{}\"", truncated_query))
    }

    fn confirm_deny_rules(&self) -> Option<IntegrationConfirmation> {
        Some(IntegrationConfirmation {
            ask_user: vec!["*".to_string()],
            deny: vec![],
        })
    }

    async fn match_against_confirm_deny(
        &self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        args: &HashMap<String, Value>,
    ) -> Result<MatchConfirmDeny, String> {
        let command_to_match = self
            .command_to_match_against_confirm_deny(ccx.clone(), &args)
            .await
            .map_err(|e| format!("Error getting tool command to match: {}", e))?;
        Ok(MatchConfirmDeny {
            result: MatchConfirmDenyResult::CONFIRMATION,
            command: command_to_match,
            rule: "default".to_string(),
        })
    }
}
