use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;

use super::SCHEMA_VERSION;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct V1Question {
    id: String,
    text: String,
    asked_at: DateTime<Utc>,
    answer: Option<String>,
    answered_at: Option<DateTime<Utc>>,
}

pub(super) fn migrate(records: &mut [Value]) -> Result<bool, String> {
    let mut migrated = false;
    for record in records {
        match record.get("schema_version").and_then(Value::as_u64) {
            Some(1) => {
                migrate_v1_to_v2(record)?;
                migrated = true;
            }
            Some(version) if version == u64::from(SCHEMA_VERSION) => {}
            Some(version) => {
                return Err(format!(
                    "Unsupported background agent schema version: {version}"
                ));
            }
            None => {
                return Err("Background agent schema_version must be an unsigned integer".into())
            }
        }
    }
    Ok(migrated)
}

fn migrate_v1_to_v2(record: &mut Value) -> Result<(), String> {
    let agent_id = record["agent_id"]
        .as_str()
        .ok_or("Background agent agent_id must be a string")?
        .to_string();
    let parent_id = record["parent_chat_id"]
        .as_str()
        .ok_or("Background agent parent_chat_id must be a string")?
        .to_string();
    if let Some(questions) = record.get_mut("questions") {
        let questions = questions
            .as_array_mut()
            .ok_or("Background agent questions must be an array")?;
        for question in questions {
            if question.get("kind").is_some() {
                continue;
            }
            let old: V1Question = serde_json::from_value(question.clone())
                .map_err(|e| format!("Invalid v1 background agent question: {e}"))?;
            if old.answer.is_none() && old.answered_at.is_some() {
                return Err("Invalid v1 pending question: unexpected answer timestamp".into());
            }
            let mut contract = refact_chat_api::QuestionContract::pending(
                &old.id,
                refact_chat_api::QuestionKind::AgentParent,
                &agent_id,
                &parent_id,
                format!("{agent_id}:{}", old.id),
                &old.text,
                None,
                old.asked_at,
                None,
            )?;
            contract.state = if old.answer.is_some() {
                refact_chat_api::QuestionState::Answered
            } else {
                refact_chat_api::QuestionState::Pending
            };
            contract.answer = old.answer;
            contract.answered_at = old.answered_at;
            contract.validate()?;
            let fields = question
                .as_object_mut()
                .ok_or("Invalid v1 question: expected object")?;
            fields.insert("kind".into(), serde_json::json!(contract.kind));
            fields.insert("asker".into(), Value::from(contract.asker));
            fields.insert("answerer".into(), Value::from(contract.answerer));
            fields.insert(
                "correlation_id".into(),
                Value::from(contract.correlation_id),
            );
            fields.insert("state".into(), serde_json::json!(contract.state));
        }
    }
    record["schema_version"] = Value::from(SCHEMA_VERSION);
    Ok(())
}
