//! Discriminated, bounded Work inspection using the HTTP projection service.
use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::coordination::{scope, service};
use crate::coordination::error::WorkError;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

pub struct ToolWorkInspect {
    pub config_path: String,
}

pub fn input_schema() -> Value {
    json!({"type":"object","additionalProperties":false,"required":["selector"],"properties":{
        "selector":{"oneOf":[
            {"type":"object","additionalProperties":false,"required":["kind","work_id"],"properties":{"kind":{"const":"work"},"work_id":{"type":"string","minLength":1}}},
            {"type":"object","additionalProperties":false,"required":["kind","paths"],"properties":{"kind":{"const":"files"},"paths":{"type":"array","minItems":1,"maxItems":crate::file_activity::settings().inspect_max_paths,"items":{"type":"string","minLength":1}}}},
            {"type":"object","additionalProperties":false,"required":["kind","worktree_id"],"properties":{"kind":{"const":"worktree"},"worktree_id":{"type":"string","minLength":1}}},
            {"type":"object","additionalProperties":false,"required":["kind"],"properties":{"kind":{"const":"project_worktrees"}}}
        ]},
        "cursor":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":crate::file_activity::settings().overview_max_rows}
    }})
}

#[async_trait]
impl Tool for ToolWorkInspect {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc { name:"work_inspect".into(), display_name:"Work Inspect".into(), source:ToolSource { source_type:ToolSourceType::Builtin, config_path:self.config_path.clone() }, experimental:false, allow_parallel:true,
            description:"Inspect permitted project work, declared/observed file activity, a registered worktree, or paginated project worktree inventory. Use exactly one selector variant. A files selector also lists integration_overlaps: peers touching the same repository-relative path from a separate checkout, an integration conflict candidate rather than a physical file conflict. No transcripts or new control authority are returned.".into(), input_schema:input_schema(), output_schema:None, annotations:None }
    }
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let request: service::InspectRequest = serde_json::from_value(json!(args))
            .map_err(|_| WorkError::invalid("invalid discriminated Work selector").to_string())?;
        let (app, chat_id, model) = {
            let ccx = ccx.lock().await;
            (
                ccx.app.clone(),
                ccx.chat_id.clone(),
                ccx.current_model.clone(),
            )
        };
        let caller = scope::model_caller(&app, &chat_id, &model)
            .await
            .map_err(|e| e.to_string())?;
        let worktree_id = match &request.selector {
            service::InspectSelector::Worktree {
                worktree_id,
            } => Some(worktree_id.clone()),
            _ => None,
        };
        let inspection = service::inspect(&app, &caller, request)
            .await
            .map_err(|e| e.to_string())?;
        let receipt = if let Some(worktree_id) = worktree_id {
            super::command_checkout::inspection_receipt(&ccx, &worktree_id).await
        } else {
            None
        };
        let content = if let Some((selector, _)) = &receipt {
            let mut response = serde_json::to_value(&inspection)
                .map_err(|_| WorkError::unavailable().to_string())?;
            response["command_checkout"] = json!(selector);
            serde_json::to_string(&response)
        } else {
            serde_json::to_string(&inspection)
        }
        .map_err(|_| WorkError::unavailable().to_string())?;
        let mut message = ChatMessage {
            role: "tool".into(),
            tool_call_id: tool_call_id.clone(),
            content: ChatContent::SimpleText(content),
            ..Default::default()
        };
        service::attach_provenance(
            &app,
            &mut message,
            &inspection.page,
            inspection.row.as_ref(),
        )
        .map_err(|e| e.to_string())?;
        if let Some((_, records)) = receipt {
            crate::privacy::records::merge_records(&mut message, records);
        }
        Ok((false, vec![ContextEnum::ChatMessage(message)]))
    }
    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}
