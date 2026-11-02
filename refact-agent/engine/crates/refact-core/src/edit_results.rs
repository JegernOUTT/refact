use std::borrow::Cow;

use crate::chat_types::{ChatContent, ChatMessage};

pub fn project_edit_result(message: &ChatMessage) -> Cow<'_, ChatMessage> {
    if message.role != "diff"
        && !(message.role == "tool" && message.extra.contains_key("edit_result"))
    {
        return Cow::Borrowed(message);
    }
    let text = message
        .extra
        .get("edit_result")
        .and_then(|value| {
            (value.get("version").and_then(|v| v.as_u64()) == Some(1))
                .then(|| value.get("model_content").and_then(|v| v.as_str()))
                .flatten()
        })
        .filter(|text| !text.is_empty())
        .unwrap_or("Edit result: excerpt unavailable.");
    let mut projected = message.clone();
    projected.content = ChatContent::SimpleText(text.to_owned());
    projected.extra.insert(
        "edit_result".into(),
        serde_json::json!({"version": 1, "model_content": text}),
    );
    Cow::Owned(projected)
}
