use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::hash::Hash;
use axum::http::StatusCode;
use ropey::Rope;

use crate::custom_error::ScratchError;
use crate::git::checkpoints::Checkpoint;
use crate::scratchpads::multimodality::MultimodalElement;

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct CursorPosition {
    pub file: String,
    pub line: i32,
    pub character: i32,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct CodeCompletionInputs {
    pub sources: HashMap<String, String>,
    pub cursor: CursorPosition,
    pub multiline: bool,
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum ReasoningEffort {
    #[serde(alias = "none")]
    NoReasoning,
    Minimal,
    Low,
    #[default]
    Medium,
    High,
    XHigh,
    Max,
}

impl ReasoningEffort {
    pub fn to_string(&self) -> String {
        match self {
            Self::NoReasoning => "none".to_string(),
            other => format!("{:?}", other).to_lowercase(),
        }
    }

    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "none" => Some(Self::NoReasoning),
            "minimal" => Some(Self::Minimal),
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "xhigh" => Some(Self::XHigh),
            "max" => Some(Self::Max),
            _ => None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct SamplingParameters {
    #[serde(default)]
    pub max_new_tokens: usize,
    pub temperature: Option<f32>,
    pub frequency_penalty: Option<f32>,
    pub top_p: Option<f32>,
    #[serde(default)]
    pub stop: Vec<String>,
    pub n: Option<usize>,
    #[serde(default)]
    pub boost_reasoning: bool,
    #[serde(default)]
    pub reasoning_effort: Option<ReasoningEffort>,
    #[serde(default)]
    pub thinking_budget: Option<usize>,
    #[serde(default)]
    pub thinking: Option<serde_json::Value>,
    #[serde(default)]
    pub enable_thinking: Option<bool>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CodeCompletionPost {
    pub inputs: CodeCompletionInputs,
    #[serde(default)]
    pub parameters: SamplingParameters,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub stream: bool,
    #[serde(default)]
    pub no_cache: bool,
    #[serde(default)]
    pub use_ast: bool,
    #[allow(dead_code)]
    #[serde(default)]
    pub use_vecdb: bool,
    #[serde(default)]
    pub rag_tokens_n: usize,
}

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

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct ContextFile {
    pub file_name: String,
    pub file_content: String,
    pub line1: usize, // starts from 1, zero means non-valid
    pub line2: usize, // starts from 1
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_rev: Option<String>,
    #[serde(default, skip_serializing)]
    pub symbols: Vec<String>,
    #[serde(default = "default_gradient_type_value", skip_serializing)]
    pub gradient_type: i32,
    #[serde(default, skip_serializing)]
    pub usefulness: f32, // higher is better
    #[serde(default, skip_serializing)]
    pub skip_pp: bool, // if true, skip postprocessing compression for this file
}

impl Default for ContextFile {
    fn default() -> Self {
        Self {
            file_name: String::new(),
            file_content: String::new(),
            line1: 0,
            line2: 0,
            file_rev: None,
            symbols: Vec::new(),
            gradient_type: -1,
            usefulness: 0.0,
            skip_pp: false,
        }
    }
}

fn default_gradient_type_value() -> i32 {
    -1
}

#[derive(Debug, Clone)]
pub enum ContextEnum {
    ContextFile(ContextFile),
    ChatMessage(ChatMessage),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatToolFunction {
    pub arguments: String,
    pub name: String,
}

impl ChatToolFunction {
    /// Parse arguments as a JSON object, normalizing empty/non-object values to `{}`.
    ///
    /// LLMs sometimes emit empty strings, `""`, `null`, or other non-object JSON
    /// as tool arguments (especially on truncated responses). This method treats any
    /// arguments string that doesn't look like a JSON object as equivalent to `{}`.
    pub fn parse_args(&self) -> Result<std::collections::HashMap<String, serde_json::Value>, serde_json::Error> {
        let trimmed = self.arguments.trim();
        let args_str = if trimmed.starts_with('{') { trimmed } else { "{}" };
        serde_json::from_str(args_str)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatToolCall {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<usize>,
    pub function: ChatToolFunction,
    #[serde(rename = "type")]
    pub tool_type: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(untagged)]
pub enum ChatContent {
    SimpleText(String),
    Multimodal(Vec<MultimodalElement>),
    ContextFiles(Vec<ContextFile>),
}

impl Default for ChatContent {
    fn default() -> Self {
        ChatContent::SimpleText(String::new())
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct MeteringUsd {
    pub prompt_usd: f64,
    pub generated_usd: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_usd: Option<f64>,
    pub total_usd: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ChatUsage {
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub total_tokens: usize,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "cache_creation_input_tokens", alias = "cache_creation_tokens")]
    pub cache_creation_tokens: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "cache_read_input_tokens", alias = "cache_read_tokens")]
    pub cache_read_tokens: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metering_usd: Option<MeteringUsd>,
}

#[derive(Debug, Serialize, Clone, Default)]
pub struct ChatMessage {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message_id: String,
    pub role: String,
    pub content: ChatContent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ChatToolCall>>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub tool_call_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_failed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<ChatUsage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checkpoints: Vec<Checkpoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_blocks: Option<Vec<serde_json::Value>>,
    /// Citations from web search results
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub citations: Vec<serde_json::Value>,
    /// Server-executed content blocks (e.g., server_tool_use, web_search_tool_result)
    /// that must be passed back verbatim in multi-turn conversations.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub server_content_blocks: Vec<serde_json::Value>,
    /// Extra provider-specific fields that should be preserved round-trip
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty", flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
    #[serde(skip)]
    pub output_filter: Option<crate::postprocessing::pp_command_output::OutputFilter>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ChatModelType {
    Light,
    Default,
    Thinking,
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
        }
    }
}

/// Normalize a mode ID string (legacy enum values or dynamic mode IDs).
/// Handles uppercase legacy values and returns lowercase mode IDs.
/// Returns error if mode is empty or contains invalid characters.
pub fn normalize_mode_id(mode: &str) -> Result<String, String> {
    let trimmed = mode.trim();
    
    if trimmed.is_empty() {
        return Ok("agent".to_string());
    }
    
    // Validate characters: lowercase, digits, underscore, hyphen
    if !trimmed.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-') {
        // Try to normalize uppercase legacy values
        let normalized = trimmed.to_lowercase();
        if !normalized.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-') {
            return Err(format!("Invalid mode ID: '{}' contains invalid characters", trimmed));
        }
        return Ok(normalized);
    }
    
    Ok(trimmed.to_string())
}

/// Check if a mode ID is agentic (supports tool execution and knowledge enrichment).
pub fn is_agentic_mode_id(mode_id: &str) -> bool {
    matches!(mode_id, "agent" | "task_planner" | "task_agent")
}

/// Validate and canonicalize a mode ID with strict registry existence check.
/// Returns 422-compatible error if mode is invalid or doesn't exist in registry.
pub async fn validate_mode_for_request(
    gcx: std::sync::Arc<tokio::sync::RwLock<crate::global_context::GlobalContext>>,
    mode: &str,
) -> Result<String, String> {
    let canonical = canonical_mode_id(mode)?;
    
    let mode_config = crate::yaml_configs::customization_registry::get_mode_config(
        gcx,
        &canonical,
        None,
    ).await;
    
    if mode_config.is_none() {
        return Err(format!("Mode '{}' does not exist in registry", canonical));
    }
    
    Ok(canonical)
}

/// Canonicalize a mode ID string with full validation and legacy mapping.
/// 
/// This function:
/// 1. Normalizes format (lowercases, validates characters)
/// 2. Maps legacy enum values to canonical mode IDs
/// 3. Validates length (max 128 chars)
/// 4. Returns error for invalid input
///
/// Examples:
/// - "AGENT" → "agent"
/// - "agent" → "agent"
/// - "CONFIGURE" → "configurator"
/// - "NO_TOOLS" → "explore"
/// - "my_custom_mode" → "my_custom_mode"
/// - "" → "agent" (default)
/// - "invalid!mode" → Err
pub fn canonical_mode_id(mode: &str) -> Result<String, String> {
    let trimmed = mode.trim();
    
    if trimmed.is_empty() {
        return Ok("agent".to_string());
    }
    
    if trimmed.len() > 128 {
        return Err(format!("Mode ID too long: {} chars (max 128)", trimmed.len()));
    }
    
    let normalized = normalize_mode_id(trimmed)?;
    
    let canonical = match normalized.to_uppercase().as_str() {
        "NO_TOOLS" => "explore".to_string(),
        "EXPLORE" => "explore".to_string(),
        "AGENT" => "agent".to_string(),
        "CONFIGURE" | "CONFIGURATOR" => "configurator".to_string(),
        "PROJECT_SUMMARY" => "project_summary".to_string(),
        "PLAN" => "plan".to_string(),
        "TASK_PLANNER" => "task_planner".to_string(),
        "TASK_AGENT" => "task_agent".to_string(),
        _ => normalized,
    };
    
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

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(default)]
pub struct PostprocessSettings {
    pub use_ast_based_pp: bool,
    pub useful_background: f32, // first, fill usefulness of all lines with this
    pub useful_symbol_default: f32, // when a symbol present, set usefulness higher
    // search results fill usefulness as it passed from outside
    pub downgrade_parent_coef: f32, // goto parent from search results and mark it useful, with this coef
    pub downgrade_body_coef: f32, // multiply body usefulness by this, so it's less useful than the declaration
    pub comments_propagate_up_coef: f32, // mark comments above a symbol as useful, with this coef
    pub close_small_gaps: bool,
    pub take_floor: f32,    // take/dont value
    pub max_files_n: usize, // don't produce more than n files in output
}

impl Default for PostprocessSettings {
    fn default() -> Self {
        Self::new()
    }
}

impl PostprocessSettings {
    pub fn new() -> Self {
        PostprocessSettings {
            use_ast_based_pp: true,
            downgrade_body_coef: 0.8,
            downgrade_parent_coef: 0.6,
            useful_background: 5.0,
            useful_symbol_default: 10.0,
            close_small_gaps: true,
            comments_propagate_up_coef: 0.99,
            take_floor: 0.0,
            max_files_n: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use crate::call_validation::{CodeCompletionInputs, CursorPosition, SamplingParameters};
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
