use serde::{Deserialize, Serialize};
use std::hash::Hash;
use axum::http::StatusCode;
use ropey::Rope;

use crate::custom_error::ScratchError;
use crate::worktrees::types::WorktreeMeta;

pub use refact_core::chat_types::{
    Checkpoint, CodeCompletionInputs, CodeCompletionPost, ContextEnum, ContextFile,
    ChatContent, ChatMessage, ChatToolCall, ChatToolFunction, ChatUsage, CursorPosition,
    MeteringUsd, MultimodalElement, OutputFilter, PostprocessSettings, ReasoningEffort,
    SamplingParameters, SearchResult, deserialize_path, format_search_results, serialize_path,
    normalize_mode_id, canonical_mode_id,
};

pub fn code_completion_post_validate(
    code_completion_post: &CodeCompletionPost,
) -> axum::response::Result<(), ScratchError> {
    let pos = &code_completion_post.inputs.cursor;
    let Some(source) = code_completion_post
        .inputs
        .sources
        .get(&code_completion_post.inputs.cursor.file)
    else {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "Invalid post: cursor in a file that is not a source".to_string(),
        ));
    };
    let text = Rope::from_str(source);
    let line_number = pos.line as usize;
    if line_number >= text.len_lines() {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "Invalid post: line number exceeds lines in file".to_string(),
        ));
    }
    let line = text.line(line_number);
    let col = pos.character as usize;
    if col > line.len_chars() {
        return Err(ScratchError::new(
            StatusCode::BAD_REQUEST,
            "Invalid post: char number exceeds chars in line".to_string(),
        ));
    }
    Ok(())
}



#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ChatModelType {
    Light,
    Default,
    Thinking,
    Buddy,
}

impl Default for ChatModelType {
    fn default() -> Self {
        ChatModelType::Default
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SubchatParameters {
    #[serde(default)]
    pub subchat_model_type: ChatModelType,
    #[serde(default)]
    pub subchat_model: String,
    pub subchat_n_ctx: usize,
    pub subchat_max_new_tokens: usize,
    pub subchat_temperature: Option<f32>,
    pub subchat_tokens_for_rag: usize,
    pub subchat_reasoning_effort: Option<ReasoningEffort>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatMeta {
    #[serde(default)]
    pub chat_id: String,
    #[serde(default)]
    pub request_attempt_id: String,
    #[serde(default)]
    pub chat_remote: bool,
    #[serde(default = "default_mode_id")]
    pub chat_mode: String,
    #[serde(default)]
    pub current_config_file: String,
    #[serde(default = "default_true")]
    pub include_project_info: bool,
    #[serde(default)]
    pub context_tokens_cap: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<WorktreeMeta>,
}

fn default_mode_id() -> String {
    "agent".to_string()
}

impl Default for ChatMeta {
    fn default() -> Self {
        ChatMeta {
            chat_id: String::new(),
            request_attempt_id: String::new(),
            chat_remote: false,
            chat_mode: default_mode_id(),
            current_config_file: String::new(),
            include_project_info: true,
            context_tokens_cap: None,
            worktree: None,
        }
    }
}



/// Check if a mode ID is agentic (supports tool execution and knowledge enrichment).
pub fn is_agentic_mode_id(mode_id: &str) -> bool {
    matches!(mode_id, "agent" | "task_planner" | "task_agent")
}

/// Validate and canonicalize a mode ID with strict registry existence check.
/// Returns 422-compatible error if mode is invalid or doesn't exist in registry.
pub async fn validate_mode_for_request(
    gcx: std::sync::Arc<crate::global_context::GlobalContext>,
    mode: &str,
) -> Result<String, String> {
    let canonical = canonical_mode_id(mode)?;

    let mode_config =
        crate::yaml_configs::customization_registry::get_mode_config(gcx, &canonical, None).await;

    if mode_config.is_none() {
        return Err(format!("Mode '{}' does not exist in registry", canonical));
    }

    Ok(canonical)
}



fn default_true() -> bool {
    true
}

#[derive(Serialize, Deserialize, Clone, Hash, Debug, Eq, PartialEq, Default, Ord, PartialOrd)]
pub struct DiffChunk {
    pub file_name: String,
    pub file_action: String, // edit, rename, add, remove
    pub line1: usize,
    pub line2: usize,
    pub lines_remove: String,
    pub lines_add: String,
    #[serde(default)]
    pub file_name_rename: Option<String>,
    #[serde(default = "default_true", skip_serializing)]
    pub is_file: bool,
    pub application_details: String,
}


#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use super::*;

    #[test]
    fn test_valid_post1() {
        let post = CodeCompletionPost {
            inputs: CodeCompletionInputs {
                sources: HashMap::from_iter([(
                    "hello.py".to_string(),
                    "def hello_world():".to_string(),
                )]),
                cursor: CursorPosition {
                    file: "hello.py".to_string(),
                    line: 0,
                    character: 18,
                },
                multiline: true,
            },
            parameters: SamplingParameters {
                max_new_tokens: 20,
                temperature: Some(0.1),
                ..Default::default()
            },
            model: "".to_string(),
            stream: false,
            no_cache: false,
            use_ast: true,
            use_vecdb: true,
            rag_tokens_n: 0,
        };
        assert!(code_completion_post_validate(&post).is_ok());
    }

    #[test]
    fn test_valid_post2() {
        let post = CodeCompletionPost {
            inputs: CodeCompletionInputs {
                sources: HashMap::from_iter([(
                    "hello.py".to_string(),
                    "你好世界Ωßåß🤖".to_string(),
                )]),
                cursor: CursorPosition {
                    file: "hello.py".to_string(),
                    line: 0,
                    character: 10,
                },
                multiline: true,
            },
            parameters: SamplingParameters {
                max_new_tokens: 20,
                temperature: Some(0.1),
                ..Default::default()
            },
            model: "".to_string(),
            stream: false,
            no_cache: false,
            use_ast: true,
            use_vecdb: true,
            rag_tokens_n: 0,
        };
        assert!(code_completion_post_validate(&post).is_ok());
    }

    #[test]
    fn test_invalid_post_incorrect_line() {
        let post = CodeCompletionPost {
            inputs: CodeCompletionInputs {
                sources: HashMap::from_iter([(
                    "hello.py".to_string(),
                    "def hello_world():".to_string(),
                )]),
                cursor: CursorPosition {
                    file: "hello.py".to_string(),
                    line: 2,
                    character: 18,
                },
                multiline: true,
            },
            parameters: SamplingParameters {
                max_new_tokens: 20,
                temperature: Some(0.1),
                ..Default::default()
            },
            model: "".to_string(),
            stream: false,
            no_cache: false,
            use_ast: true,
            use_vecdb: true,
            rag_tokens_n: 0,
        };
        assert!(code_completion_post_validate(&post).is_err());
    }

    #[test]
    fn test_invalid_post_incorrect_col() {
        let post = CodeCompletionPost {
            inputs: CodeCompletionInputs {
                sources: HashMap::from_iter([(
                    "hello.py".to_string(),
                    "def hello_world():".to_string(),
                )]),
                cursor: CursorPosition {
                    file: "hello.py".to_string(),
                    line: 0,
                    character: 80,
                },
                multiline: true,
            },
            parameters: SamplingParameters {
                max_new_tokens: 20,
                temperature: Some(0.1),
                ..Default::default()
            },
            model: "".to_string(),
            stream: false,
            no_cache: false,
            use_ast: true,
            use_vecdb: true,
            rag_tokens_n: 0,
        };
        assert!(code_completion_post_validate(&post).is_err());
    }

    fn make_tool_fn(arguments: &str) -> ChatToolFunction {
        ChatToolFunction {
            arguments: arguments.to_string(),
            name: "test_tool".to_string(),
        }
    }

    #[test]
    fn test_parse_args_valid_object() {
        let f = make_tool_fn(r#"{"key": "value", "num": 42}"#);
        let args = f.parse_args().unwrap();
        assert_eq!(args["key"], "value");
        assert_eq!(args["num"], 42);
    }

    #[test]
    fn test_parse_args_empty_object() {
        let f = make_tool_fn("{}");
        let args = f.parse_args().unwrap();
        assert!(args.is_empty());
    }

    #[test]
    fn test_parse_args_empty_string() {
        let f = make_tool_fn("");
        let args = f.parse_args().unwrap();
        assert!(args.is_empty());
    }

    #[test]
    fn test_parse_args_whitespace_only() {
        let f = make_tool_fn("   \n\t  ");
        let args = f.parse_args().unwrap();
        assert!(args.is_empty());
    }

    #[test]
    fn test_parse_args_json_empty_string_literal() {
        // LLM sends "" (two quote chars) — valid JSON string, not an object
        let f = make_tool_fn(r#""""#);
        let args = f.parse_args().unwrap();
        assert!(args.is_empty());
    }

    #[test]
    fn test_parse_args_json_null() {
        let f = make_tool_fn("null");
        let args = f.parse_args().unwrap();
        assert!(args.is_empty());
    }

    #[test]
    fn test_parse_args_json_array() {
        // An array is not an object — should normalize to {}
        let f = make_tool_fn("[1, 2, 3]");
        let args = f.parse_args().unwrap();
        assert!(args.is_empty());
    }

    #[test]
    fn test_parse_args_padded_with_whitespace() {
        let f = make_tool_fn(r#"  { "a": 1 }  "#);
        let args = f.parse_args().unwrap();
        assert_eq!(args["a"], 1);
    }

    #[test]
    fn test_parse_args_invalid_json_object() {
        // Starts with '{' but is malformed — should propagate the serde error
        let f = make_tool_fn("{broken json");
        assert!(f.parse_args().is_err());
    }
}

pub fn deserialize_messages_from_post(
    messages: &Vec<serde_json::Value>,
) -> Result<Vec<ChatMessage>, ScratchError> {
    let messages: Vec<ChatMessage> = messages
        .iter()
        .map(|x| serde_json::from_value(x.clone()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| {
            tracing::error!("can't deserialize ChatMessage: {}", e);
            ScratchError::new(StatusCode::BAD_REQUEST, format!("JSON problem: {}", e))
        })?;
    Ok(messages)
}
