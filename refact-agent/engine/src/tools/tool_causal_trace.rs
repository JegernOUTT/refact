use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::coordination::{causal, causal_view, roots, scope};
use crate::coordination::error::WorkError;
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};

const DEFAULT_MAX_EVENTS: usize = 50;
const MAX_EVENTS: usize = 200;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CausalTraceArgs {
    event_id: String,
    #[serde(default)]
    replay: bool,
    #[serde(default = "default_max_events")]
    max_events: usize,
}

fn default_max_events() -> usize {
    DEFAULT_MAX_EVENTS
}

pub struct ToolCausalTrace {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolCausalTrace {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "causal_trace".into(),
            display_name: "Causal Trace".into(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Read why a recorded event happened, showing its visible causes in this project. Optional replay checks whether the recorded coordination decisions reproduce; it never re-runs a model.".into(),
            input_schema: json!({"type":"object", "additionalProperties":false, "required":["event_id"], "properties":{
                "event_id":{"type":"string", "minLength":1},
                "replay":{"type":"boolean", "default":false},
                "max_events":{"type":"integer", "minimum":1, "maximum":MAX_EVENTS, "default":DEFAULT_MAX_EVENTS}
            }}),
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
        let request: CausalTraceArgs = serde_json::from_value(json!(args))
            .map_err(|_| WorkError::invalid("invalid causal trace request").to_string())?;
        if request.event_id.trim().is_empty() {
            return Err(WorkError::invalid("event_id must not be empty").to_string());
        }
        if request.max_events == 0 || request.max_events > MAX_EVENTS {
            return Err(WorkError::invalid("max_events must be between 1 and 200").to_string());
        }

        let (app, chat_id, model_id) = {
            let context = ccx.lock().await;
            (
                context.app.clone(),
                context.chat_id.clone(),
                context.current_model.clone(),
            )
        };
        let caller = scope::model_caller(&app, &chat_id, &model_id)
            .await
            .map_err(|error| error.to_string())?;
        let project =
            refact_work_core::ProjectKey::new(roots::project_identity(&caller.source_root));
        let (events, history_complete) = causal_view::events_for(&app).await;
        let events: Vec<_> = events
            .into_iter()
            .filter(|event| event.project == project)
            .collect();
        let model = caller
            .model
            .as_ref()
            .ok_or_else(|| WorkError::policy_unavailable().to_string())?;
        let disclosure = crate::coordination::privacy_guard::policy_key(&app.gcx, model)
            .map_err(|error| error.to_string())?;
        let events_visible = causal::clear_for_model(&app.gcx, Some(model), events)
            .map_err(|error| error.to_string())?;
        if crate::coordination::privacy_guard::policy_key(&app.gcx, model)
            .map_err(|error| error.to_string())?
            != disclosure
        {
            return Err(WorkError::policy_unavailable().to_string());
        }
        let mut visible_by_id = HashMap::with_capacity(events_visible.len());
        for event in events_visible {
            visible_by_id.insert(event.event_id.clone(), event);
        }
        let events_visible: Vec<_> = visible_by_id.values().cloned().collect();
        let trace = causal::reconstruct(&events_visible, &request.event_id);
        if trace.events.is_empty() {
            return Ok((
                false,
                vec![tool_message(
                    tool_call_id,
                    "no recorded event with this id is visible to you".to_string(),
                )],
            ));
        }

        let replay = request
            .replay
            .then(|| causal_view::replay(&events_visible, &trace));
        let mut ordered = Vec::with_capacity(trace.events.len());
        if let Some(root) = trace
            .events
            .iter()
            .find(|event| event.event_id == trace.root)
        {
            ordered.push(root);
        }
        ordered.extend(
            trace
                .events
                .iter()
                .filter(|event| event.event_id != trace.root),
        );
        let truncated = ordered.len() > request.max_events;
        let rendered_events = ordered
            .iter()
            .take(request.max_events)
            .enumerate()
            .map(|(index, event)| {
                let causes = event
                    .causes_or_unknown()
                    .iter()
                    .map(|cause| match cause.event_id() {
                        Some(id) if visible_by_id.contains_key(id) => id,
                        Some(_) => "withheld or unavailable event",
                        None => "unknown",
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{}. {} — {} in scope {} at {} ms; causes: {}",
                    index + 1,
                    event.event_id,
                    event.kind.as_str(),
                    event.scope,
                    event.recorded_at_ms,
                    causes
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let mut output = format!("Causal trace for {}:\n{}", trace.root, rendered_events);
        if truncated {
            output.push_str(&format!(
                "\nTrace truncated to {} events; {} additional events omitted.",
                request.max_events,
                ordered.len() - request.max_events
            ));
        }
        if !trace.complete || !history_complete {
            output.push_str("\nTrace may be incomplete: causal gaps or unavailable event history.");
        }
        if let Some(report) = replay {
            output.push_str(&format!(
                "\nReplay reproduced: {} ({} mismatch(es)).",
                if report.reproduced() { "yes" } else { "no" },
                report.finding_count
            ));
            for finding in report.findings.iter().take(3) {
                output.push_str(&format!(
                    "\nMismatch at {}: {:?}",
                    finding.step, finding.divergence
                ));
            }
        }
        Ok((false, vec![tool_message(tool_call_id, output)]))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        Vec::new()
    }
}

fn tool_message(tool_call_id: &str, content: String) -> ContextEnum {
    ContextEnum::ChatMessage(ChatMessage {
        role: "tool".into(),
        tool_call_id: tool_call_id.to_string(),
        content: ChatContent::SimpleText(content),
        ..Default::default()
    })
}
