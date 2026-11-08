use std::{collections::HashMap, sync::Arc};
use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex;
use crate::{
    at_commands::at_commands::AtCommandsContext,
    call_validation::{ChatContent, ChatMessage, ContextEnum},
    coordination::scope,
    organization::{service, types::*},
};
use super::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

pub struct ToolOrganization {
    pub name: String,
    pub config_path: String,
}
#[async_trait]
impl Tool for ToolOrganization {
    fn tool_description(&self) -> ToolDesc {
        let input_schema = match self.name.as_str() {
            "organization_list" => {
                json!({"type":"object","additionalProperties":false,"required":["action"],"properties":{"action":{"enum":["taxonomy","lookup"]},"kind":{"enum":["topics","tags"]},"limit":{"type":"integer","minimum":1,"maximum":200},"cursor":{"type":"string"},"items":{"type":"array","maxItems":200,"items":{"type":"object","required":["kind","id"],"additionalProperties":false,"properties":{"kind":{"enum":["chat","task"]},"id":{"type":"string"}}}}}})
            }
            "organization_candidates" => {
                json!({"type":"object","additionalProperties":false,"properties":{"limit":{"type":"integer","minimum":1,"maximum":12}}})
            }
            "organization_apply" => {
                json!({"type":"object","additionalProperties":false,"required":["action","request"],"properties":{"action":{"enum":["apply","review"]},"request":{"type":"object","description":"apply: expected_revision,request_id,operations (assign_item only). review: expected_revision,request_id,outcomes [{item,fingerprint,decision:assign|keep|defer,topic_name:null|string,tag_names:string[]}]. Semantic names are resolved/created atomically; omit operations. Names max120 chars; max100 operations; tag_ids max16. Locks user-only."}}})
            }
            _ => json!({"type":"object","additionalProperties":false,"properties":{}}),
        };
        ToolDesc{name:self.name.clone(),display_name:self.name.clone(),source:ToolSource{source_type:ToolSourceType::Builtin,config_path:self.config_path.clone()},experimental:false,allow_parallel:self.name!="organization_apply",description:"Project-scoped thematic organization, not transcript editing. List taxonomy or lookup canonical root chat/task assignments; candidates returns bounded USER excerpts and fingerprints. Model taxonomy edits require source-bound review. Apply existing IDs only, or review assign/keep/defer after candidate read. No caller paths or authority arguments. Manual locks and names are protected. Outputs carry privacy provenance; partial is not full-history coverage.".into(),input_schema,output_schema:None,annotations:None}
    }
    async fn tool_execute(
        &mut self,
        ccx: Arc<Mutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let allowed: &[&str] = match self.name.as_str() {
            "organization_list" => &["action", "kind", "limit", "cursor", "items"],
            "organization_candidates" => &["limit"],
            "organization_apply" => &["action", "request"],
            _ => &[],
        };
        if args.keys().any(|key| !allowed.contains(&key.as_str()))
            || serde_json::to_vec(args).map_err(|e| e.to_string())?.len() > 128 * 1024
        {
            return Err(Error::invalid("unknown argument or request exceeds 128 KiB").to_string());
        }
        let (app, chat_id, model, context_records) = {
            let c = ccx.lock().await;
            (
                c.app.clone(),
                c.chat_id.clone(),
                c.current_model.clone(),
                refact_privacy::records_from_messages(&c.messages)
                    .map_err(|_| "organization context provenance unavailable")?
                    .into_iter()
                    .map(|(_, record)| record)
                    .collect::<Vec<_>>(),
            )
        };
        let caller = scope::model_caller(&app, &chat_id, &model)
            .await
            .map_err(|e| e.to_string())?;
        let caller = service::project_caller(&app, &caller)
            .await
            .map_err(|e| e.to_string())?;
        let mut records = service::clear_store(&app, &caller)
            .await
            .map_err(|e| e.to_string())?;
        let result: Result<Value, Error> = async {
            match self.name.as_str() {
                "organization_list" => match args.get("action").and_then(Value::as_str) {
                    Some("taxonomy") => {
                        let kind = serde_json::from_value(
                            args.get("kind").cloned().unwrap_or(json!("topics")),
                        )
                        .map_err(|_| Error::invalid("invalid kind"))?;
                        let limit =
                            args.get("limit").and_then(Value::as_u64).unwrap_or(200) as usize;
                        serde_json::to_value(
                            service::taxonomy(
                                &app,
                                &caller,
                                kind,
                                limit,
                                args.get("cursor").and_then(Value::as_str),
                            )
                            .await?,
                        )
                        .map_err(Into::into)
                    }
                    Some("lookup") => {
                        let items =
                            serde_json::from_value(args.get("items").cloned().unwrap_or(json!([])))
                                .map_err(|_| Error::invalid("invalid items"))?;
                        let response = service::items(
                            &app,
                            &caller,
                            ItemsRequest {
                                items,
                            },
                        )
                        .await?;
                        records.extend(response.source_records.clone());
                        serde_json::to_value(response).map_err(Into::into)
                    }
                    _ => Err(Error::invalid("action must be taxonomy or lookup")),
                },
                "organization_candidates" => {
                    let page = crate::organization::candidates::list_candidates(
                        &app,
                        &caller,
                        args.get("limit").and_then(Value::as_u64).unwrap_or(12) as usize,
                    )
                    .await?;
                    for c in &page.candidates {
                        records.extend(c.source_records.clone());
                    }
                    serde_json::to_value(page).map_err(Into::into)
                }
                "organization_apply" => {
                    let request = args
                        .get("request")
                        .cloned()
                        .ok_or_else(|| Error::invalid("request required"))?;
                    match args.get("action").and_then(Value::as_str) {
                        Some("apply") => serde_json::to_value(
                            service::apply(
                                &app,
                                &caller,
                                serde_json::from_value(request)
                                    .map_err(|_| Error::invalid("invalid apply request"))?,
                            )
                            .await?,
                        )
                        .map_err(Into::into),
                        Some("review") => serde_json::to_value(
                            crate::organization::candidates::commit_review_with_context(
                                &app,
                                &caller,
                                serde_json::from_value(request)
                                    .map_err(|_| Error::invalid("invalid review request"))?,
                                context_records,
                            )
                            .await?,
                        )
                        .map_err(Into::into),
                        _ => Err(Error::invalid("action must be apply or review")),
                    }
                }
                "organization_status" => {
                    let owned = scope::WorkCaller {
                        source_root: caller.source_root.clone(),
                        checkout_root: caller.checkout_root.clone(),
                        root_chat_id: caller.root_chat_id.clone(),
                        principal: caller.principal.clone(),
                        model: caller.model.clone(),
                    };
                    service::blocking(move || crate::organization::candidates::status(&owned)).await
                }
                _ => Err(Error::invalid("unknown organization tool")),
            }
        }
        .await;
        records.extend(
            service::clear_store(&app, &caller)
                .await
                .map_err(|e| e.to_string())?,
        );
        let mut message = ChatMessage {
            role: "tool".into(),
            tool_call_id: tool_call_id.clone(),
            content: ChatContent::SimpleText(
                serde_json::to_string(&result.map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?,
            ),
            ..Default::default()
        };
        crate::privacy::records::merge_records(&mut message, records);
        Ok((false, vec![ContextEnum::ChatMessage(message)]))
    }
    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}
