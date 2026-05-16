use serde::Deserialize;

use refact_core::chat_types::ChatMessage;

#[derive(Deserialize, Clone)]
pub struct FollowUpResponse {
    pub follow_ups: Vec<String>,
    pub topic_changed: bool,
}

pub fn make_conversation(messages: &Vec<ChatMessage>, system_prompt: &str) -> Vec<ChatMessage> {
    let mut history_message = "*Conversation:*\n".to_string();
    for m in messages.iter().rev().take(2) {
        let content = m.content.to_text_with_image_placeholders();
        let char_count = content.chars().count();
        let limited_content = if char_count > 5000 {
            let skip_count = char_count - 5000;
            format!(
                "...{}",
                content.chars().skip(skip_count).collect::<String>()
            )
        } else {
            content
        };
        let message_row = match m.role.as_str() {
            "user" => format!("👤:{}\n\n", limited_content),
            "assistant" => format!("🤖:{}\n\n", limited_content),
            _ => continue,
        };
        history_message.insert_str(0, &message_row);
    }
    vec![
        ChatMessage::new("system".to_string(), system_prompt.to_string()),
        ChatMessage::new("user".to_string(), history_message),
    ]
}
