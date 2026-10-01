use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum};
use crate::tools::tools_description::{Tool, ToolDesc, ToolSource, ToolSourceType};
use crate::attention::{AttentionPriority, AttentionRequest};
use refact_chat_api::{QuestionContract, QuestionKind};
use crate::chat::trajectories::try_save_trajectory_with_intent;
use crate::chat::types::{
    PendingQuestion, PendingQuestionSet, PendingQuestionStatus, TrajectoryCommitIntent,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QuestionItem {
    id: String,
    #[serde(rename = "type")]
    question_type: String,
    text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    options: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    priority: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default: Option<String>,
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
                                },
                                "priority": {
                                    "type": "string",
                                    "description": "How much of the user's attention this claims. 'blocking' skips the batching window, 'low' is refused when the user is already saturated. Defaults to normal.",
                                    "enum": ["low", "normal", "blocking"]
                                },
                                "default": {
                                    "type": "string",
                                    "description": "Safe answer applied if the user never responds. Only allowed with priority 'low'. The outcome is recorded as defaulted, never as answered."
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

        let (chat_id, app) = {
            let ccx_lock = ccx.lock().await;
            (ccx_lock.chat_id.clone(), ccx_lock.app.clone())
        };

        let session_arc = app.chat.sessions.read().await.get(&chat_id).cloned();
        let asked_at_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
        let pending = PendingQuestionSet {
            tool_call_id: tool_call_id.clone(),
            questions: questions
                .iter()
                .map(|q| PendingQuestion {
                    id: q.id.clone(),
                    question_type: q.question_type.clone(),
                    text: q.text.clone(),
                    options: q.options.clone(),
                })
                .collect(),
            asked_at_ms,
            status: PendingQuestionStatus::Pending,
            answers: Vec::new(),
            resolution_id: None,
        };
        let asked_at = chrono::Utc::now();
        let scope = crate::attention::scope_for_chat(&app, &chat_id).await;
        let waiter = format!("{chat_id}:{tool_call_id}");
        let budget = app.attention();
        let mut contracts: Vec<QuestionContract> = Vec::with_capacity(questions.len());
        for question in &questions {
            let priority = match question.priority.as_deref() {
                Some(priority) => crate::attention::parse_priority(priority)?,
                None => AttentionPriority::Normal,
            };
            let admission = budget
                .join(
                    AttentionRequest::new(
                        scope.clone(),
                        QuestionKind::UserInput,
                        format!("chat:{chat_id}"),
                        "user",
                        question.text.clone(),
                    )
                    .with_options(question.options.clone())
                    .with_priority(priority)
                    .with_default_answer(question.default.clone())
                    .with_delivery(Some(crate::attention::PromptDelivery {
                        chat_id: chat_id.clone(),
                        tool_call_id: tool_call_id.clone(),
                        asked_at_ms,
                        question: crate::attention::PromptDeliveryQuestion {
                            id: question.id.clone(),
                            question_type: question.question_type.clone(),
                            text: question.text.clone(),
                            options: question.options.clone(),
                        },
                    })),
                    &waiter,
                    asked_at,
                )
                .map_err(|error| error.to_string())?;
            contracts.push(
                budget
                    .questions()
                    .get(&admission.question_id)
                    .ok_or("admitted question is missing from the question service")?,
            );
        }
        if let Some(session_arc) = session_arc {
            {
                let mut session = session_arc.lock().await;
                session.register_pending_questions(pending)?;
            }
            if let Err(error) = try_save_trajectory_with_intent(
                app.clone(),
                session_arc.clone(),
                TrajectoryCommitIntent::Required,
            )
            .await
            {
                let mut session = session_arc.lock().await;
                session.rollback_pending_question_registration(tool_call_id);
                return Err(format!("failed to persist pending questions: {error}"));
            }
            {
                let mut session = session_arc.lock().await;
                let snapshot = session.snapshot();
                session.emit(snapshot);
            }
        }

        {
            let ccx_lock = ccx.lock().await;
            ccx_lock.abort_flag.store(true, Ordering::SeqCst);
        }

        let result = serde_json::json!({
            "type": "ask_questions",
            "tool_call_id": tool_call_id,
            "questions": questions,
            "contracts": contracts,
            "asked_at_ms": asked_at_ms,
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
