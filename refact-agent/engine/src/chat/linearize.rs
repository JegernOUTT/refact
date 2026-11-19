use crate::call_validation::ChatMessage;
use std::collections::HashSet;

pub use refact_chat_history::linearize::apply_summarization_linearize;

pub(crate) fn append_task_goal_context(
    messages: &mut Vec<ChatMessage>,
    controls: Vec<ChatMessage>,
) {
    for message in messages.iter_mut() {
        if !message.message_id.is_empty() {
            if let Some(control) = controls
                .iter()
                .find(|control| control.message_id == message.message_id)
            {
                *message = control.clone();
            }
        }
    }
    let mut seen: HashSet<_> = messages
        .iter()
        .filter(|message| !message.message_id.is_empty())
        .map(|message| message.message_id.clone())
        .collect();
    messages.extend(controls.into_iter().filter(|control| {
        !control.message_id.is_empty() && seen.insert(control.message_id.clone())
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    use refact_core::chat_types::ChatContent;
    use serde_json::json;

    fn message(role: &str, text: &str) -> ChatMessage {
        ChatMessage {
            message_id: uuid::Uuid::new_v4().to_string(),
            role: role.into(),
            content: ChatContent::SimpleText(text.into()),
            ..Default::default()
        }
    }

    #[test]
    fn task_goal_updates_leave_conversation_prefix_unchanged() {
        let goal = message("goal", "base");
        let plan = message("plan", "steps");
        let history = vec![
            message("system", "frozen"),
            goal.clone(),
            plan.clone(),
            message("user", "question"),
        ];
        let mut delta = message("event", "correction");
        delta
            .extra
            .insert("event".into(), json!({"subkind":"goal_delta"}));
        let mut before = history.clone();
        append_task_goal_context(&mut before, vec![goal.clone(), plan.clone()]);
        assert_eq!(
            serde_json::to_value(&before).unwrap(),
            serde_json::to_value(&history).unwrap()
        );
        let assistant = message("assistant", "work");
        let mut after = before.clone();
        after.push(assistant.clone());
        after.push(delta.clone());
        append_task_goal_context(&mut after, vec![goal.clone(), plan.clone(), delta.clone()]);
        assert_eq!(
            serde_json::to_value(&after[..before.len()]).unwrap(),
            serde_json::to_value(&before).unwrap()
        );
        assert_eq!(
            after.last().unwrap().content.content_text_only(),
            "correction"
        );
        let mut legacy = message("plan", "legacy without id");
        legacy.message_id.clear();
        let replay = serde_json::to_value(&after).unwrap();
        append_task_goal_context(&mut after, vec![legacy.clone()]);
        append_task_goal_context(&mut after, vec![legacy]);
        assert_eq!(serde_json::to_value(&after).unwrap(), replay);
        append_task_goal_context(&mut after, vec![goal, plan, delta]);
        assert_eq!(serde_json::to_value(&after).unwrap(), replay);
        assert_eq!(after[before.len()].message_id, assistant.message_id);
        after.push(message("user", "next turn"));
        assert_eq!(
            serde_json::to_value(&after[..after.len() - 1]).unwrap(),
            replay
        );
    }
}
