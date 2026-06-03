use std::collections::HashSet;
use std::fmt;

use crate::call_validation::ChatMessage;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrajectorySurgeryError {
    EmptyMessageId,
    EmptyRemoval,
    MessageNotFound {
        message_id: String,
    },
    DuplicateMessageId {
        message_id: String,
    },
    InsertIndexOutOfBounds {
        index: usize,
        len: usize,
    },
    EmptyAssistantToolCallId {
        message_id: String,
    },
    EmptyToolResultId {
        message_id: String,
    },
    MissingToolResult {
        message_id: String,
        tool_call_id: String,
    },
    MissingToolCall {
        message_id: String,
        tool_call_id: String,
    },
}

impl fmt::Display for TrajectorySurgeryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyMessageId => write!(f, "message_id must not be empty"),
            Self::EmptyRemoval => write!(f, "remove_messages requires at least one message_id"),
            Self::MessageNotFound { message_id } => {
                write!(f, "message not found: {message_id}")
            }
            Self::DuplicateMessageId { message_id } => {
                write!(f, "message_id is ambiguous: {message_id}")
            }
            Self::InsertIndexOutOfBounds { index, len } => {
                write!(
                    f,
                    "insert index {index} is out of bounds for {len} messages"
                )
            }
            Self::EmptyAssistantToolCallId { message_id } => {
                write!(
                    f,
                    "assistant message {message_id} has an empty tool call id"
                )
            }
            Self::EmptyToolResultId { message_id } => {
                write!(
                    f,
                    "tool result message {message_id} has an empty tool_call_id"
                )
            }
            Self::MissingToolResult {
                message_id,
                tool_call_id,
            } => write!(
                f,
                "assistant message {message_id} tool call {tool_call_id} has no tool result"
            ),
            Self::MissingToolCall {
                message_id,
                tool_call_id,
            } => write!(
                f,
                "tool result message {message_id} references missing tool call {tool_call_id}"
            ),
        }
    }
}

impl std::error::Error for TrajectorySurgeryError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InsertPosition {
    Index(usize),
    BeforeMessage(String),
    AfterMessage(String),
}

pub fn validate_pairing(messages: &[ChatMessage]) -> Result<(), TrajectorySurgeryError> {
    let mut result_ids = HashSet::new();
    for message in messages.iter().filter(|message| is_tool_result(message)) {
        if message.tool_call_id.is_empty() {
            return Err(TrajectorySurgeryError::EmptyToolResultId {
                message_id: message.message_id.clone(),
            });
        }
        result_ids.insert(message.tool_call_id.clone());
    }

    let mut assistant_ids = HashSet::new();
    for message in messages
        .iter()
        .filter(|message| message.role == "assistant")
    {
        let Some(tool_calls) = message.tool_calls.as_ref() else {
            continue;
        };
        for tool_call in tool_calls {
            if tool_call.id.is_empty() {
                return Err(TrajectorySurgeryError::EmptyAssistantToolCallId {
                    message_id: message.message_id.clone(),
                });
            }
            if !result_ids.contains(&tool_call.id) {
                return Err(TrajectorySurgeryError::MissingToolResult {
                    message_id: message.message_id.clone(),
                    tool_call_id: tool_call.id.clone(),
                });
            }
            assistant_ids.insert(tool_call.id.clone());
        }
    }

    for message in messages.iter().filter(|message| is_tool_result(message)) {
        if !assistant_ids.contains(&message.tool_call_id) {
            return Err(TrajectorySurgeryError::MissingToolCall {
                message_id: message.message_id.clone(),
                tool_call_id: message.tool_call_id.clone(),
            });
        }
    }

    Ok(())
}

pub fn edit_message(
    messages: &[ChatMessage],
    message_id: &str,
    mut replacement: ChatMessage,
) -> Result<Vec<ChatMessage>, TrajectorySurgeryError> {
    let index = find_message_index(messages, message_id)?;
    replacement.message_id = messages[index].message_id.clone();

    let mut edited = messages.to_vec();
    edited[index] = replacement;
    validate_pairing(&edited)?;
    Ok(edited)
}

pub fn remove_messages<I, S>(
    messages: &[ChatMessage],
    message_ids: I,
) -> Result<Vec<ChatMessage>, TrajectorySurgeryError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let ids = collect_message_ids(message_ids)?;
    if ids.is_empty() {
        return Err(TrajectorySurgeryError::EmptyRemoval);
    }
    for id in &ids {
        find_message_index(messages, id)?;
    }

    let remove_ids: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let edited: Vec<ChatMessage> = messages
        .iter()
        .filter(|message| !remove_ids.contains(message.message_id.as_str()))
        .cloned()
        .collect();
    validate_pairing(&edited)?;
    Ok(edited)
}

pub fn insert_message(
    messages: &[ChatMessage],
    position: InsertPosition,
    message: ChatMessage,
) -> Result<Vec<ChatMessage>, TrajectorySurgeryError> {
    ensure_new_message_id(messages, &message)?;
    let index = resolve_insert_index(messages, position)?;

    let mut edited = messages.to_vec();
    edited.insert(index, message);
    validate_pairing(&edited)?;
    Ok(edited)
}

fn is_tool_result(message: &ChatMessage) -> bool {
    matches!(message.role.as_str(), "tool" | "diff")
}

fn collect_message_ids<I, S>(message_ids: I) -> Result<Vec<String>, TrajectorySurgeryError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut ids: Vec<String> = Vec::new();
    for message_id in message_ids {
        let message_id = message_id.as_ref();
        if message_id.is_empty() {
            return Err(TrajectorySurgeryError::EmptyMessageId);
        }
        if !ids.iter().any(|id| id.as_str() == message_id) {
            ids.push(message_id.to_string());
        }
    }
    Ok(ids)
}

fn find_message_index(
    messages: &[ChatMessage],
    message_id: &str,
) -> Result<usize, TrajectorySurgeryError> {
    if message_id.is_empty() {
        return Err(TrajectorySurgeryError::EmptyMessageId);
    }

    let mut found = None;
    for (index, message) in messages.iter().enumerate() {
        if message.message_id == message_id {
            if found.is_some() {
                return Err(TrajectorySurgeryError::DuplicateMessageId {
                    message_id: message_id.to_string(),
                });
            }
            found = Some(index);
        }
    }

    found.ok_or_else(|| TrajectorySurgeryError::MessageNotFound {
        message_id: message_id.to_string(),
    })
}

fn ensure_new_message_id(
    messages: &[ChatMessage],
    message: &ChatMessage,
) -> Result<(), TrajectorySurgeryError> {
    if message.message_id.is_empty() {
        return Err(TrajectorySurgeryError::EmptyMessageId);
    }
    if messages
        .iter()
        .any(|existing| existing.message_id == message.message_id)
    {
        return Err(TrajectorySurgeryError::DuplicateMessageId {
            message_id: message.message_id.clone(),
        });
    }
    Ok(())
}

fn resolve_insert_index(
    messages: &[ChatMessage],
    position: InsertPosition,
) -> Result<usize, TrajectorySurgeryError> {
    match position {
        InsertPosition::Index(index) => {
            if index <= messages.len() {
                Ok(index)
            } else {
                Err(TrajectorySurgeryError::InsertIndexOutOfBounds {
                    index,
                    len: messages.len(),
                })
            }
        }
        InsertPosition::BeforeMessage(message_id) => find_message_index(messages, &message_id),
        InsertPosition::AfterMessage(message_id) => {
            find_message_index(messages, &message_id).map(|index| index + 1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_validation::{ChatContent, ChatToolCall, ChatToolFunction};

    fn user(id: &str, content: &str) -> ChatMessage {
        ChatMessage {
            message_id: id.to_string(),
            role: "user".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            ..Default::default()
        }
    }

    fn assistant(id: &str, content: &str) -> ChatMessage {
        ChatMessage {
            message_id: id.to_string(),
            role: "assistant".to_string(),
            content: ChatContent::SimpleText(content.to_string()),
            ..Default::default()
        }
    }

    fn assistant_tool(id: &str, tool_call_id: &str) -> ChatMessage {
        ChatMessage {
            message_id: id.to_string(),
            role: "assistant".to_string(),
            content: ChatContent::SimpleText(String::new()),
            tool_calls: Some(vec![ChatToolCall {
                id: tool_call_id.to_string(),
                index: Some(0),
                function: ChatToolFunction {
                    arguments: "{}".to_string(),
                    name: "test_tool".to_string(),
                },
                tool_type: "function".to_string(),
                extra_content: None,
            }]),
            ..Default::default()
        }
    }

    fn tool(id: &str, tool_call_id: &str) -> ChatMessage {
        ChatMessage {
            message_id: id.to_string(),
            role: "tool".to_string(),
            content: ChatContent::SimpleText("ok".to_string()),
            tool_call_id: tool_call_id.to_string(),
            ..Default::default()
        }
    }

    fn pair_messages() -> Vec<ChatMessage> {
        vec![
            user("user-1", "hello"),
            assistant_tool("assistant-tool", "call-1"),
            tool("tool-1", "call-1"),
            assistant("assistant-final", "done"),
        ]
    }

    fn message_ids(messages: &[ChatMessage]) -> Vec<&str> {
        messages
            .iter()
            .map(|message| message.message_id.as_str())
            .collect()
    }

    #[test]
    fn valid_pair_accepted() {
        validate_pairing(&pair_messages()).unwrap();
    }

    #[test]
    fn assistant_only_removal_rejected() {
        let err = remove_messages(&pair_messages(), ["assistant-tool"]).unwrap_err();

        assert_eq!(
            err,
            TrajectorySurgeryError::MissingToolCall {
                message_id: "tool-1".to_string(),
                tool_call_id: "call-1".to_string(),
            }
        );
    }

    #[test]
    fn tool_only_removal_rejected() {
        let err = remove_messages(&pair_messages(), ["tool-1"]).unwrap_err();

        assert_eq!(
            err,
            TrajectorySurgeryError::MissingToolResult {
                message_id: "assistant-tool".to_string(),
                tool_call_id: "call-1".to_string(),
            }
        );
    }

    #[test]
    fn safe_non_tool_edit_accepted() {
        let edited =
            edit_message(&pair_messages(), "user-1", user("different-id", "changed")).unwrap();

        assert_eq!(edited[0].message_id, "user-1");
        assert_eq!(edited[0].content.content_text_only(), "changed");
        validate_pairing(&edited).unwrap();
    }

    #[test]
    fn tool_id_break_rejected() {
        let err = edit_message(
            &pair_messages(),
            "assistant-tool",
            assistant_tool("replacement-id", "call-2"),
        )
        .unwrap_err();

        assert_eq!(
            err,
            TrajectorySurgeryError::MissingToolResult {
                message_id: "assistant-tool".to_string(),
                tool_call_id: "call-2".to_string(),
            }
        );
    }

    #[test]
    fn valid_insert_before_and_after() {
        let before = insert_message(
            &pair_messages(),
            InsertPosition::BeforeMessage("assistant-final".to_string()),
            user("insert-before", "before"),
        )
        .unwrap();
        assert_eq!(
            message_ids(&before),
            vec![
                "user-1",
                "assistant-tool",
                "tool-1",
                "insert-before",
                "assistant-final"
            ]
        );

        let after = insert_message(
            &pair_messages(),
            InsertPosition::AfterMessage("user-1".to_string()),
            user("insert-after", "after"),
        )
        .unwrap();
        assert_eq!(
            message_ids(&after),
            vec![
                "user-1",
                "insert-after",
                "assistant-tool",
                "tool-1",
                "assistant-final"
            ]
        );
    }

    #[test]
    fn empty_removal_rejected() {
        let err = remove_messages(&pair_messages(), std::iter::empty::<&str>()).unwrap_err();

        assert_eq!(err, TrajectorySurgeryError::EmptyRemoval);
    }
}
