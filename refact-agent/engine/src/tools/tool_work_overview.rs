//! Read-only, caller-scoped Work discovery through the shared projection.
use std::collections::HashMap;
use std::sync::Arc;
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;
use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::coordination::{scope, service};
use crate::coordination::error::WorkError;
use crate::coordination::types::WorkOverviewRequest;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

pub struct ToolWorkOverview {
    pub config_path: String,
}

pub(crate) fn parse_request(
    args: &HashMap<String, Value>,
) -> Result<WorkOverviewRequest, WorkError> {
    let request: WorkOverviewRequest = serde_json::from_value(json!(args))
        .map_err(|_| WorkError::invalid("invalid Work overview request"))?;
    if request
        .rows
        .is_some_and(|n| n == 0 || n > crate::file_activity::settings().overview_max_rows)
    {
        return Err(WorkError::invalid(
            "limit is outside the configured page range",
        ));
    }
    Ok(request)
}

#[async_trait]
impl Tool for ToolWorkOverview {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "work_overview".into(), display_name: "Work Overview".into(),
            source: ToolSource { source_type: ToolSourceType::Builtin, config_path: self.config_path.clone() },
            experimental: false, allow_parallel: true,
            description: "Discover same-project chats and agents across both runtimes. Root scope means this caller's conversation tree, not another supplied root. Metadata conveys no control authority. Partial coverage is not proof that a file is safe to edit; transcripts are never included.".into(),
            input_schema: json!({"type":"object", "additionalProperties":false, "properties":{
                "scope":{"type":"string","enum":["project","root"]},
                "path":{"type":"string","description":"Authorized project/checkout file or directory path."},
                "states":{"type":"array","items":{"type":"string","enum":crate::coordination::types::WorkStateTag::ALL.map(|state| state.as_str())}},
                "limit":{"type":"integer","minimum":1,"maximum":crate::file_activity::settings().overview_max_rows},
                "cursor":{"type":"string","description":"Opaque continuation; fetch a fresh snapshot on cursor_expired."}
            }}),
            output_schema: None, annotations: None,
        }
    }
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let request = parse_request(args).map_err(|e| e.to_string())?;
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
        let page = service::overview(&app, &caller, request)
            .await
            .map_err(|e| e.to_string())?;
        let mut message = ChatMessage {
            role: "tool".into(),
            tool_call_id: tool_call_id.clone(),
            content: ChatContent::SimpleText(
                serde_json::to_string(&page).map_err(|_| WorkError::unavailable().to_string())?,
            ),
            ..Default::default()
        };
        service::attach_provenance(&app, &mut message, &page, None).map_err(|e| e.to_string())?;
        Ok((false, vec![ContextEnum::ChatMessage(message)]))
    }
    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strict_request_rejects_invalid_types_limits_and_spoofed_roots() {
        for value in [
            json!({"scope":7}),
            json!({"limit":-1}),
            json!({"limit":0}),
            json!({"limit":1.5}),
            json!({"limit":"5"}),
            json!({"states":["made_up"]}),
            json!({"project":"foreign"}),
        ] {
            assert!(parse_request(&serde_json::from_value(value).unwrap()).is_err());
        }
        assert_eq!(
            parse_request(&serde_json::from_value(json!({"limit":7})).unwrap())
                .unwrap()
                .rows,
            Some(7)
        );
        assert!(parse_request(&HashMap::new()).is_ok());
    }
}
