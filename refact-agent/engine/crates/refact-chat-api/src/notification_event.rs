use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NotificationEvent {
    TaskDone {
        chat_id: String,
        tool_call_id: String,
        summary: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        knowledge_path: Option<String>,
    },
    AskQuestions {
        chat_id: String,
        tool_call_id: String,
        questions: Vec<NotificationQuestion>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        asked_at_ms: Option<u64>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NotificationQuestion {
    pub id: String,
    #[serde(rename = "type")]
    pub question_type: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_event_roundtrips_ask_questions() {
        let event = NotificationEvent::AskQuestions {
            chat_id: "chat-1".to_string(),
            tool_call_id: "tool-1".to_string(),
            asked_at_ms: Some(123),
            questions: vec![NotificationQuestion {
                id: "q-1".to_string(),
                question_type: "single_choice".to_string(),
                text: "Pick a pond".to_string(),
                options: Some(vec!["north".to_string(), "south".to_string()]),
            }],
        };

        let json = serde_json::to_value(&event).expect("event serializes");
        assert_eq!(json["type"], "ask_questions");
        assert_eq!(json["questions"][0]["type"], "single_choice");

        let roundtrip: NotificationEvent =
            serde_json::from_value(json).expect("event deserializes");
        let NotificationEvent::AskQuestions {
            chat_id,
            tool_call_id,
            questions,
            asked_at_ms,
        } = roundtrip
        else {
            panic!("expected ask questions notification");
        };

        assert_eq!(asked_at_ms, Some(123));
        assert_eq!(chat_id, "chat-1");
        assert_eq!(tool_call_id, "tool-1");
        assert_eq!(questions.len(), 1);
        assert_eq!(questions[0].id, "q-1");
        assert_eq!(questions[0].question_type, "single_choice");
        assert_eq!(questions[0].text, "Pick a pond");
        assert_eq!(
            questions[0].options,
            Some(vec!["north".to_string(), "south".to_string()])
        );
    }

    #[test]
    fn legacy_ask_notification_without_timestamp_still_deserializes() {
        let event: NotificationEvent = serde_json::from_value(serde_json::json!({
            "type": "ask_questions", "chat_id": "chat-1", "tool_call_id": "tool-1", "questions": []
        }))
        .expect("legacy notification remains compatible");
        assert!(matches!(
            event,
            NotificationEvent::AskQuestions {
                asked_at_ms: None,
                ..
            }
        ));
    }

    #[test]
    fn old_ask_notification_without_timestamp_still_deserializes() {
        let event: NotificationEvent = serde_json::from_value(serde_json::json!({
            "type": "ask_questions", "chat_id": "chat-1", "tool_call_id": "tool-1",
            "questions": []
        }))
        .expect("legacy notification is compatible");
        assert!(matches!(
            event,
            NotificationEvent::AskQuestions {
                asked_at_ms: None,
                ..
            }
        ));
        assert!(serde_json::to_value(event)
            .unwrap()
            .get("asked_at_ms")
            .is_none());
    }

    #[test]
    fn notification_event_omits_empty_optional_fields() {
        let event = NotificationEvent::TaskDone {
            chat_id: "chat-1".to_string(),
            tool_call_id: "tool-1".to_string(),
            summary: "done".to_string(),
            knowledge_path: None,
        };

        let json = serde_json::to_value(&event).expect("event serializes");
        assert_eq!(json["type"], "task_done");
        assert!(json.get("knowledge_path").is_none());
    }
}
