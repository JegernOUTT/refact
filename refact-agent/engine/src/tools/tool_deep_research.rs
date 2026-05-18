use std::collections::HashMap;
use std::sync::Arc;
use serde_json::Value;
use tokio::sync::Mutex as AMutex;
use async_trait::async_trait;

use crate::subchat::{run_subchat, resolve_subchat_config_with_parent};
use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, MatchConfirmDeny, MatchConfirmDenyResult,
    json_schema_from_params,
};
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::at_commands::at_commands::AtCommandsContext;
use crate::global_context::GlobalContext;
use crate::yaml_configs::customization_registry::get_subagent_config;
use crate::yaml_configs::customization_types::SubagentConfig;
use crate::knowledge_index::format_related_memories_section;
use crate::integrations::integr_abstract::IntegrationConfirmation;
use crate::memories::{memories_add_enriched, EnrichmentParams};
use crate::postprocessing::pp_command_output::OutputFilter;

const SUBAGENT_ID: &str = "deep_research";

pub struct ToolDeepResearch {
    pub config_path: String,
}

fn render_research_template(template: &str, research_query: &str) -> String {
    template
        .replace("{{research_query}}", research_query)
        .replace("{{query}}", research_query)
        .replace("{{task}}", research_query)
}

fn template_includes_research_query(template: &str) -> bool {
    template.contains("{{research_query}}")
        || template.contains("{{query}}")
        || template.contains("{{task}}")
}

fn build_deep_research_messages(
    subagent_config: &SubagentConfig,
    research_query: &str,
) -> Vec<ChatMessage> {
    let mut messages = Vec::new();

    if let Some(system_prompt) = subagent_config.messages.system_prompt.as_ref() {
        messages.push(ChatMessage::new(
            "system".to_string(),
            system_prompt.clone(),
        ));
    }

    for pre_msg in &subagent_config.messages.pre_messages {
        messages.push(ChatMessage::new(
            pre_msg.role.clone(),
            render_research_template(&pre_msg.content, research_query),
        ));
    }

    let mut query_was_rendered = false;
    if let Some(researcher_prompt) = subagent_config.messages.user_template.as_ref() {
        query_was_rendered = template_includes_research_query(researcher_prompt);
        messages.push(ChatMessage::new(
            "user".to_string(),
            render_research_template(researcher_prompt, research_query),
        ));
    }

    if !query_was_rendered {
        messages.push(ChatMessage::new(
            "user".to_string(),
            format!("# Research Query\n\n{}", research_query),
        ));
    }

    for post_msg in &subagent_config.messages.post_messages {
        messages.push(ChatMessage::new(
            post_msg.role.clone(),
            render_research_template(&post_msg.content, research_query),
        ));
    }

    messages
}

async fn execute_deep_research(
    gcx: Arc<GlobalContext>,
    subchat_tx: Arc<AMutex<tokio::sync::mpsc::UnboundedSender<serde_json::Value>>>,
    research_query: String,
    tool_call_id: String,
    abort_flag: Arc<std::sync::atomic::AtomicBool>,
    parent_depth: usize,
    parent_task_meta: Option<crate::chat::types::TaskMeta>,
    parent_worktree: Option<crate::worktrees::types::WorktreeMeta>,
) -> Result<(ChatMessage, serde_json::Map<String, serde_json::Value>), String> {
    let subagent_config = get_subagent_config(gcx.clone(), SUBAGENT_ID, None)
        .await
        .ok_or_else(|| format!("subagent config '{}' not found", SUBAGENT_ID))?;

    let messages = build_deep_research_messages(&subagent_config, &research_query);
    let tools = if subagent_config.tools.is_empty() {
        None
    } else {
        Some(subagent_config.tools.clone())
    };
    let max_steps = subagent_config
        .subchat
        .max_steps
        .unwrap_or(20)
        .min(50)
        .max(1);

    let config = resolve_subchat_config_with_parent(
        gcx.clone(),
        SUBAGENT_ID,
        subagent_config.subchat.stateful,
        None,
        Some("Deep Research".to_string()),
        None,
        None,
        None,
        tools,
        max_steps,
        false,
        None,
        "agent".to_string(),
        parent_task_meta,
        parent_worktree,
        Some(tool_call_id.clone()),
        Some(subchat_tx),
        Some(abort_flag),
        parent_depth + 1,
    )
    .await?;

    let subchat_result = run_subchat(gcx, messages, config).await?;
    let reply = subchat_result
        .messages
        .last()
        .cloned()
        .ok_or("No response from deep research")?;

    Ok((reply, subchat_result.metering))
}

#[async_trait]
impl Tool for ToolDeepResearch {
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
            input_schema: json_schema_from_params(&[("research_query", "string", "A detailed research question or topic. Be specific: include the scope, what comparisons or metrics you need, any preferred sources, and the desired output format. Example: 'Research the current best practices for Rust async error handling in 2024, comparing tokio vs async-std approaches, with code examples and performance considerations.'")], &["research_query"]),
            output_schema: None,
            annotations: None,
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
            (ccx_lock.app.gcx.clone(), ccx_lock.subchat_tx.clone())
        };

        let (abort_flag, parent_depth, parent_task_meta, parent_worktree) = {
            let ccx_lock = ccx.lock().await;
            (
                ccx_lock.abort_flag.clone(),
                ccx_lock.subchat_depth,
                ccx_lock.task_meta.clone(),
                ccx_lock.execution_scope_worktree(),
            )
        };

        tracing::info!("Starting deep research for query: {}", research_query);

        let (research_result, metering) = execute_deep_research(
            gcx,
            subchat_tx,
            research_query.clone(),
            tool_call_id.clone(),
            abort_flag,
            parent_depth,
            parent_task_meta,
            parent_worktree,
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
        let memory_note = match memories_add_enriched(
            ccx.clone(),
            &research_content,
            enrichment_params,
        )
        .await
        {
            Ok(path) => {
                tracing::info!("Created enriched memory from deep research: {:?}", path);
                format!(
                        "\n\n---\n📝 **This report has been saved to the knowledge base:** `{}`\n\nRelated memories may be shown elsewhere in short form. To load full content of a memory, call `cat(paths=\"{}\")`.",
                        path.display(),
                        path.display()
                    )
            }
            Err(e) => {
                tracing::warn!("Failed to create enriched memory from deep research: {}", e);
                String::new()
            }
        };
        let related_memories_note = {
            let gcx = ccx.lock().await.app.gcx.clone();
            let gcx_read = gcx.clone();
            let idx_guard = gcx_read.knowledge_index.lock().await;
            let cards = idx_guard.related_for_tags(
                &vec!["deep-research".to_string(), "research".to_string()],
                5,
            );
            format_related_memories_section(&cards, None)
        };

        let final_message = format!(
            "{}{}{}",
            research_content, memory_note, related_memories_note
        );

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(final_message),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                usage: None,
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
