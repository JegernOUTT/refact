use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use async_trait::async_trait;
use refact_buddy_core::conductor::ConductorWakeReason;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};
use crate::http::routers::v1::sidebar::{NotificationEvent, NotificationQuestion};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QuestionItem {
    id: String,
    #[serde(rename = "type")]
    question_type: String,
    text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    options: Option<Vec<String>>,
}

pub struct ToolAskQuestions {
    pub config_path: String,
}

#[async_trait]
impl Tool for ToolAskQuestions {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "ask_questions".to_string(),
            display_name: "Ask Questions".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Present questions to the user and wait for answers. Stops generation until user responds. Question types: yes_no, single_select, multi_select, free_text.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "questions": {
                        "type": "array",
                        "description": "List of questions to present to the user.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": {
                                    "type": "string",
                                    "description": "Unique identifier for the question."
                                },
                                "type": {
                                    "type": "string",
                                    "description": "Question type: yes_no, single_select, multi_select, or free_text.",
                                    "enum": ["yes_no", "single_select", "multi_select", "free_text"]
                                },
                                "text": {
                                    "type": "string",
                                    "description": "The question text to display to the user."
                                },
                                "options": {
                                    "type": "array",
                                    "description": "Options for single_select or multi_select questions.",
                                    "items": { "type": "string" }
                                }
                            },
                            "required": ["id", "type", "text"]
                        }
                    }
                },
                "required": ["questions"]
            }),
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
        let questions_value = args
            .get("questions")
            .ok_or("argument `questions` is missing")?;

        let questions: Vec<QuestionItem> = match questions_value {
            serde_json::Value::String(s) => serde_json::from_str(s)
                .map_err(|e| format!("failed to parse questions JSON string: {}", e))?,
            serde_json::Value::Array(_) => serde_json::from_value(questions_value.clone())
                .map_err(|e| format!("failed to parse questions array: {}", e))?,
            _ => return Err("questions must be a JSON string or array".to_string()),
        };

        if questions.is_empty() {
            return Err("questions array cannot be empty".to_string());
        }

        let mut seen_ids = std::collections::HashSet::new();
        for q in &questions {
            if q.id.is_empty() {
                return Err("question id cannot be empty".to_string());
            }
            if q.text.is_empty() {
                return Err(format!("question '{}' text cannot be empty", q.id));
            }
            if !seen_ids.insert(&q.id) {
                return Err(format!("duplicate question id: '{}'", q.id));
            }
            match q.question_type.as_str() {
                "yes_no" | "free_text" => {}
                "single_select" | "multi_select" => {
                    if q.options.is_none() || q.options.as_ref().unwrap().is_empty() {
                        return Err(format!(
                            "question '{}' of type '{}' requires non-empty options array",
                            q.id, q.question_type
                        ));
                    }
                }
                other => {
                    return Err(format!(
                        "unknown question type '{}' for question '{}'. Valid types: yes_no, single_select, multi_select, free_text",
                        other, q.id
                    ));
                }
            }
        }

        {
            let ccx_lock = ccx.lock().await;
            ccx_lock.abort_flag.store(true, Ordering::SeqCst);
        }

        let chat_id = {
            let ccx_lock = ccx.lock().await;
            ccx_lock.chat_id.clone()
        };

        {
            let ccx_lock = ccx.lock().await;
            let gcx = ccx_lock.global_context.clone();
            if let Some(ref tx) = gcx.notification_events_tx {
                let notification_questions: Vec<NotificationQuestion> = questions
                    .iter()
                    .map(|q| NotificationQuestion {
                        id: q.id.clone(),
                        question_type: q.question_type.clone(),
                        text: q.text.clone(),
                        options: q.options.clone(),
                    })
                    .collect();
                let _ = tx.send(NotificationEvent::AskQuestions {
                    chat_id: chat_id.clone(),
                    tool_call_id: tool_call_id.clone(),
                    questions: notification_questions,
                });
            }
        }

        let (wake_gcx, wake_task_id) = {
            let ccx_lock = ccx.lock().await;
            (
                ccx_lock.global_context.clone(),
                ccx_lock.task_meta.as_ref().map(|meta| meta.task_id.clone()),
            )
        };
        crate::buddy::conductor::wake::enqueue_chat_or_task_wake(
            wake_gcx,
            &chat_id,
            wake_task_id.as_deref(),
            ConductorWakeReason::ChatLifecycle,
        )
        .await;

        let result = serde_json::json!({
            "type": "ask_questions",
            "tool_call_id": tool_call_id,
            "questions": questions,
        });

        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(result.to_string()),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            })],
        ))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}
