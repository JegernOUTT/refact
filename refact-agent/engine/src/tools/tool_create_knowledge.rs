use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use serde_json::Value;
use tracing::{error, info};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::background_tasks::wait_for_shutdown;
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::files_correction::get_project_dirs;
use crate::global_context::GlobalContext;
use crate::tools::tools_description::{
    Tool, ToolDesc, ToolSource, ToolSourceType, json_schema_from_params,
};
use crate::memories::{memories_add_enriched, EnrichmentParams};
use crate::knowledge_index::format_related_memories_section;
use refact_core::string_utils::redact_sensitive;

fn spawn_memory_enrichment_task(
    ccx: Arc<AMutex<AtCommandsContext>>,
    gcx: Arc<GlobalContext>,
    content: String,
    params: EnrichmentParams,
) {
    tokio::spawn(async move {
        tokio::select! {
            outcome = memories_add_enriched(ccx, &content, params) => match outcome {
                Ok(knowledge_path) => {
                    info!(
                        "save_knowledge: knowledge saved to {}",
                        knowledge_path.display()
                    );
                }
                Err(e) => {
                    error!("save_knowledge: failed to save knowledge: {}", e);
                }
            },
            _ = wait_for_shutdown(gcx) => {
                error!("save_knowledge: enrichment abandoned, engine is shutting down");
            }
        }
    });
}

pub struct ToolCreateKnowledge {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolCreateKnowledge {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "create_knowledge".to_string(),
            display_name: "Create Knowledge".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Creates a new knowledge entry. Returns immediately; AI metadata enrichment and indexing run in the background. Use it if you need to remember something.".to_string(),
            input_schema: json_schema_from_params(&[("content", "string", "The knowledge content to store."), ("tags", "string", "Comma-separated tags (optional, will be auto-enriched)."), ("filenames", "string", "Comma-separated related file paths (optional, will be auto-enriched).")], &["content"]),
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
        info!("create_knowledge {:?}", args);

        let content = match args.get("content") {
            Some(Value::String(s)) => s.clone(),
            Some(v) => return Err(format!("argument `content` is not a string: {:?}", v)),
            None => return Err("argument `content` is missing".to_string()),
        };
        let content = redact_sensitive(&content);

        let user_tags: Vec<String> = match args.get("tags") {
            Some(Value::String(s)) => s
                .split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect(),
            _ => vec![],
        };

        let user_filenames: Vec<String> = match args.get("filenames") {
            Some(Value::String(s)) => s
                .split(',')
                .map(|f| f.trim().to_string())
                .filter(|f| !f.is_empty())
                .collect(),
            _ => vec![],
        };

        let (root_chat_id, gcx) = {
            let ccx_lock = ccx.lock().await;
            (ccx_lock.root_chat_id.clone(), ccx_lock.app.gcx.clone())
        };

        if get_project_dirs(gcx.clone()).await.is_empty() {
            return Err("cannot save knowledge: no workspace folder is open".to_string());
        }

        let enrichment_params = EnrichmentParams {
            base_tags: user_tags.clone(),
            base_filenames: user_filenames,
            base_kind: "knowledge".to_string(),
            base_title: None,
            source_chat_id: (!root_chat_id.is_empty()).then_some(root_chat_id),
        };

        let related_section = {
            let idx_arc = gcx.knowledge_index.clone();
            let idx_guard = idx_arc.lock().await;
            let mut tags = user_tags.clone();
            tags.push("knowledge".to_string());
            tags.sort();
            tags.dedup();
            let cards = idx_guard.related_for_tags(&tags, 5);
            format_related_memories_section(&cards)
        };

        spawn_memory_enrichment_task(ccx.clone(), gcx, content, enrichment_params);

        let result_msg = format!(
            "Knowledge entry queued. Metadata enrichment and indexing run in the background, and a \
             failure after this point is reported only in the engine log. Search for the entry \
             later with `knowledge(search_key=...)`.{}",
            related_section
        );

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(result_msg),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            })],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec!["knowledge".to_string()]
    }
}
