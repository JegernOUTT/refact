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

fn render_inspection_error(
    error: WorkError,
    selector: &service::InspectSelector,
    attaching: bool,
) -> String {
    let guidance = match selector {
        service::InspectSelector::Files { .. } => "Try fewer or narrower authorized selected paths as a new request. If even one path cannot fit, report request_id to the local operator",
        service::InspectSelector::ProjectWorktrees {} => "Inventory pagination cannot shrink fixed metadata or a single worktree's complete provenance. Inspect a known authorized worktree separately if useful; if it cannot fit, report request_id to the local operator",
        _ => "This detail/single-worktree selector has no path filter to narrow its provenance. Report request_id to the local operator",
    };
    error
        .for_provenance_request(guidance, attaching)
        .to_string()
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
            description:"Inspect permitted project work, declared/observed file activity, a registered worktree, or paginated project worktree inventory. Use exactly one selector variant. A files selector also lists integration_overlaps: peers touching the same repository-relative path from a separate checkout, an integration conflict candidate rather than a physical file conflict. No transcripts or new control authority are returned. On provenance_capacity, files may narrow selected paths; fixed/detail failures need operator review, not a lower limit or unchanged retry.".into(), input_schema:input_schema(), output_schema:None, annotations:None }
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
        let selector = request.selector.clone();
        let inspection = service::inspect(&app, &caller, request)
            .await
            .map_err(|e| render_inspection_error(e, &selector, false))?;
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
        .map_err(|e| render_inspection_error(e, &selector, true))?;
        if let Some((_, records)) = receipt {
            crate::privacy::records::merge_records(&mut message, records);
        }
        Ok((false, vec![ContextEnum::ChatMessage(message)]))
    }
    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordination::error::WorkFailureCause;

    #[test]
    fn provenance_rendering_is_selector_specific_and_preserves_envelope() {
        for (selector, expected, forbidden) in [
            (
                json!({"kind":"files","paths":["src/lib.rs"]}),
                "fewer or narrower authorized selected paths",
                "has no path filter",
            ),
            (
                json!({"kind":"work","work_id":"work"}),
                "has no path filter",
                "Try fewer",
            ),
            (
                json!({"kind":"worktree","worktree_id":"wt"}),
                "has no path filter",
                "Try fewer",
            ),
            (
                json!({"kind":"project_worktrees"}),
                "Inventory pagination cannot shrink",
                "Try fewer",
            ),
        ] {
            let selector = serde_json::from_value(selector).unwrap();
            for attaching in [false, true] {
                let error = WorkError::projection_failure(WorkFailureCause::ProvenanceCapacity);
                let request_id = error.request_id.clone();
                let rendered: Value =
                    serde_json::from_str(&render_inspection_error(error, &selector, attaching))
                        .unwrap();
                assert_eq!(rendered["request_id"], request_id);
                assert_eq!(rendered["cause"], "provenance_capacity");
                assert_eq!(rendered["code"], "unavailable");
                assert_eq!(rendered["retryable"], false);
                let message = rendered["message"].as_str().unwrap();
                assert!(message.contains(expected), "{message}");
                assert!(!message.contains(forbidden), "{message}");
                assert!(message.contains("not truncated or omitted"));
                assert!(message
                    .contains("Reducing limit or repeating an unchanged request does not resolve"));
                assert_eq!(message.contains("while attaching"), attaching);
                assert_eq!(message.contains("while preparing"), !attaching);
            }
        }
    }

    #[test]
    fn non_capacity_errors_are_not_rewritten() {
        let selector = service::InspectSelector::Files {
            paths: vec!["src".into()],
        };
        for error in [
            WorkError::scope_denied(),
            WorkError::not_visible(),
            WorkError::cursor_expired(),
            WorkError::policy_unavailable(),
        ] {
            let expected = error.to_string();
            assert_eq!(render_inspection_error(error, &selector, true), expected);
        }
    }
}
