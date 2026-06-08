use std::collections::HashMap;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;
use base64::Engine;
use image::ImageReader;
use regex::Regex;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use tokenizers::Tokenizer;

pub fn serialize_path<S: serde::Serializer>(
    path: &PathBuf,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&path.to_string_lossy())
}

pub fn deserialize_path<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<PathBuf, D::Error> {
    Ok(PathBuf::from(String::deserialize(deserializer)?))
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Checkpoint {
    #[serde(
        serialize_with = "serialize_path",
        deserialize_with = "deserialize_path"
    )]
    pub workspace_folder: PathBuf,
    pub commit_hash: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct MultimodalElement {
    pub m_type: String,
    pub m_content: String,
}

const MAX_IMAGE_BASE64_LEN: usize = 15 * 1024 * 1024;
const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;

pub fn image_reader_from_b64string(b64: &str) -> Result<ImageReader<Cursor<Vec<u8>>>, String> {
    if b64.len() > MAX_IMAGE_BASE64_LEN {
        return Err(format!("image base64 too large: {} bytes", b64.len()));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|_| "base64 decode failed".to_string())?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(format!("image too large: {} bytes", bytes.len()));
    }
    let cursor = Cursor::new(bytes);
    ImageReader::new(cursor)
        .with_guessed_format()
        .map_err(|e| e.to_string())
}

fn calculate_image_tokens_by_dimensions_openai(mut width: u32, mut height: u32) -> i32 {
    const SMALL_CHUNK_SIZE: u32 = 512;
    const COST_PER_SMALL_CHUNK: i32 = 170;
    const BIG_CHUNK_SIZE: u32 = 2048;
    const CONST_COST: i32 = 85;
    let shrink_factor = (width.max(height) as f64) / (BIG_CHUNK_SIZE as f64);
    if shrink_factor > 1.0 {
        width = (width as f64 / shrink_factor) as u32;
        height = (height as f64 / shrink_factor) as u32;
    }
    let width_chunks = (width as f64 / SMALL_CHUNK_SIZE as f64).ceil() as u32;
    let height_chunks = (height as f64 / SMALL_CHUNK_SIZE as f64).ceil() as u32;
    (width_chunks * height_chunks) as i32 * COST_PER_SMALL_CHUNK + CONST_COST
}

pub fn calculate_image_tokens_openai(image_string: &str, detail: &str) -> Result<i32, String> {
    let reader = image_reader_from_b64string(image_string)
        .map_err(|_| "Failed to read image".to_string())?;
    let (width, height) = reader
        .into_dimensions()
        .map_err(|_| "Failed to get dimensions".to_string())?;
    match detail {
        "high" => Ok(calculate_image_tokens_by_dimensions_openai(width, height)),
        "low" => Ok(85),
        _ => Err("detail must be one of high or low".to_string()),
    }
}

pub fn count_text_tokens(tokenizer: Option<Arc<Tokenizer>>, text: &str) -> Result<usize, String> {
    if let Some(tok) = tokenizer {
        Ok(tok.encode(text, false).map_err(|e| e.to_string())?.len())
    } else {
        Ok(text.len() / 3 + 1)
    }
}

impl MultimodalElement {
    pub fn new(m_type: String, m_content: String) -> Result<Self, String> {
        if m_type != "text" && !m_type.starts_with("image/") {
            return Err(format!(
                "MultimodalElement::new() received invalid type: {}",
                m_type
            ));
        }
        if m_type.starts_with("image/") {
            image_reader_from_b64string(&m_content)
                .map_err(|e| format!("MultimodalElement::new() failed to parse image: {}", e))?;
        }
        Ok(MultimodalElement { m_type, m_content })
    }

    pub fn is_text(&self) -> bool {
        self.m_type == "text"
    }

    pub fn is_image(&self) -> bool {
        self.m_type.starts_with("image/")
    }

    pub fn count_tokens(
        &self,
        tokenizer: Option<Arc<Tokenizer>>,
        style: &Option<String>,
    ) -> Result<i32, String> {
        if self.is_text() {
            Ok(count_text_tokens(tokenizer, &self.m_content)? as i32)
        } else if self.is_image() {
            let style = style.clone().unwrap_or("openai".to_string());
            match style.as_str() {
                "openai" => calculate_image_tokens_openai(&self.m_content, "high"),
                _ => unreachable!(),
            }
        } else {
            unreachable!()
        }
    }
}

pub fn parse_image_b64_from_image_url_openai(image_url: &str) -> Option<(String, String, String)> {
    let re = Regex::new(r"data:(image/(png|jpeg|jpg|webp|gif));base64,([A-Za-z0-9+/=]+)").unwrap();
    re.captures(image_url).and_then(|captures| {
        let image_type = captures.get(1)?.as_str().to_string();
        let encoding = "base64".to_string();
        let value = captures.get(3)?.as_str().to_string();
        Some((image_type, encoding, value))
    })
}

impl MultimodalElement {
    pub fn from_openai_image(openai_image: MultimodalElementImageOpenAI) -> Result<Self, String> {
        let (image_type, _, image_content) =
            parse_image_b64_from_image_url_openai(&openai_image.image_url.url).ok_or(format!(
                "Failed to parse image URL: {}",
                openai_image.image_url.url
            ))?;
        MultimodalElement::new(image_type, image_content)
    }

    pub fn from_openai_text(openai_text: MultimodalElementTextOpenAI) -> Result<Self, String> {
        MultimodalElement::new("text".to_string(), openai_text.text)
    }
}

fn default_detail() -> String {
    "auto".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct MultimodalElementTextOpenAI {
    #[serde(rename = "type")]
    pub content_type: String,
    pub text: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct MultimodalElementImageOpenAI {
    #[serde(rename = "type")]
    pub content_type: String,
    pub image_url: MultimodalElementImageOpenAIImageURL,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Default)]
pub struct MultimodalElementImageOpenAIImageURL {
    pub url: String,
    #[serde(default = "default_detail")]
    pub detail: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(untagged)]
pub enum ChatMultimodalElement {
    MultimodalElementTextOpenAI(MultimodalElementTextOpenAI),
    MultimodalElementImageOpenAI(MultimodalElementImageOpenAI),
    MultimodalElement(MultimodalElement),
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ChatContentRaw {
    SimpleText(String),
    Multimodal(Vec<ChatMultimodalElement>),
    ContextFiles(Vec<ContextFile>),
}

impl ChatContentRaw {
    pub fn to_internal_format(&self) -> Result<ChatContent, String> {
        match self {
            ChatContentRaw::SimpleText(text) => Ok(ChatContent::SimpleText(text.clone())),
            ChatContentRaw::Multimodal(elements) => {
                let internal: Result<Vec<MultimodalElement>, String> = elements
                    .iter()
                    .map(|el| match el {
                        ChatMultimodalElement::MultimodalElementTextOpenAI(text_el) => {
                            MultimodalElement::from_openai_text(text_el.clone())
                        }
                        ChatMultimodalElement::MultimodalElementImageOpenAI(image_el) => {
                            MultimodalElement::from_openai_image(image_el.clone())
                        }
                        ChatMultimodalElement::MultimodalElement(el) => Ok(el.clone()),
                    })
                    .collect();
                internal.map(ChatContent::Multimodal)
            }
            ChatContentRaw::ContextFiles(files) => Ok(ChatContent::ContextFiles(files.clone())),
        }
    }
}

pub fn chat_content_raw_from_value(value: Value) -> Result<ChatContentRaw, String> {
    fn validate_multimodal_element(element: &ChatMultimodalElement) -> Result<(), String> {
        match element {
            ChatMultimodalElement::MultimodalElementTextOpenAI(el) => {
                if el.content_type != "text" {
                    return Err("Invalid multimodal element: type must be `text`".to_string());
                }
            }
            ChatMultimodalElement::MultimodalElementImageOpenAI(el) => {
                if el.content_type != "image_url" {
                    return Err("Invalid multimodal element: type must be `image_url`".to_string());
                }
                if parse_image_b64_from_image_url_openai(&el.image_url.url).is_none() {
                    return Err("Invalid image URL in MultimodalElementImageOpenAI".to_string());
                }
            }
            ChatMultimodalElement::MultimodalElement(_el) => {}
        };
        Ok(())
    }

    match value {
        Value::Null => Ok(ChatContentRaw::SimpleText(String::new())),
        Value::String(s) => Ok(ChatContentRaw::SimpleText(s)),
        Value::Array(array) => {
            if let Some(first) = array.first() {
                if first.get("file_name").is_some() {
                    let files: Result<Vec<ContextFile>, _> = array
                        .iter()
                        .map(|item| serde_json::from_value(item.clone()))
                        .collect();
                    if let Ok(context_files) = files {
                        return Ok(ChatContentRaw::ContextFiles(context_files));
                    }
                }
            }
            let mut elements = vec![];
            for (idx, item) in array.into_iter().enumerate() {
                let element: ChatMultimodalElement = serde_json::from_value(item)
                    .map_err(|e| format!("Error deserializing element at index {}: {}", idx, e))?;
                validate_multimodal_element(&element)
                    .map_err(|e| format!("Validation error for element at index {}: {}", idx, e))?;
                elements.push(element);
            }
            Ok(ChatContentRaw::Multimodal(elements))
        }
        Value::Object(obj) => {
            if let Some(content_val) = obj.get("content") {
                match chat_content_raw_from_value(content_val.clone()) {
                    Ok(inner) => return Ok(inner),
                    Err(_) => {
                        if let Some(s) = content_val.as_str() {
                            return Ok(ChatContentRaw::SimpleText(s.to_string()));
                        }
                    }
                }
            }
            Ok(ChatContentRaw::SimpleText(
                serde_json::to_string(&Value::Object(obj)).unwrap_or_default(),
            ))
        }
        other => {
            let type_name = match &other {
                Value::Bool(_) => "bool",
                Value::Number(_) => "number",
                _ => "unknown",
            };
            let value_str =
                serde_json::to_string(&other).unwrap_or_else(|_| "failed to serialize".to_string());
            tracing::error!(
                "deserialize_chat_content() can't parse content type: {}, value: {}",
                type_name,
                value_str
            );
            Err("deserialize_chat_content() can't parse content".to_string())
        }
    }
}

impl<'de> Deserialize<'de> for ChatMessage {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value: Value = Deserialize::deserialize(deserializer)?;
        let obj = value
            .as_object()
            .ok_or_else(|| serde::de::Error::custom("expected object"))?;

        let role = obj
            .get("role")
            .and_then(|s| s.as_str())
            .ok_or_else(|| serde::de::Error::missing_field("role"))?
            .to_string();

        let content = match obj.get("content") {
            Some(content_value) => {
                let content_raw = chat_content_raw_from_value(content_value.clone())
                    .map_err(serde::de::Error::custom)?;
                content_raw
                    .to_internal_format()
                    .map_err(serde::de::Error::custom)?
            }
            None => ChatContent::SimpleText(String::new()),
        };

        let message_id = obj
            .get("message_id")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string();
        let finish_reason = obj
            .get("finish_reason")
            .and_then(|x| x.as_str().map(|x| x.to_string()));
        let reasoning_content = obj
            .get("reasoning_content")
            .and_then(|x| x.as_str().map(|x| x.to_string()));
        let tool_call_id = obj
            .get("tool_call_id")
            .and_then(|s| s.as_str())
            .unwrap_or_default()
            .to_string();
        let tool_failed = obj.get("tool_failed").and_then(|x| x.as_bool());
        let preserve = obj.get("preserve").and_then(|x| x.as_bool());

        let tool_calls: Option<Vec<ChatToolCall>> = obj
            .get("tool_calls")
            .and_then(|v| v.as_array())
            .map(|v| {
                v.iter()
                    .map(|v| serde_json::from_value(v.clone()).map_err(serde::de::Error::custom))
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?;

        let thinking_blocks: Option<Vec<Value>> = obj
            .get("thinking_blocks")
            .and_then(|v| v.as_array())
            .map(|v| v.iter().cloned().collect());

        let citations: Vec<Value> = obj
            .get("citations")
            .and_then(|v| v.as_array())
            .map(|v| v.iter().cloned().collect())
            .unwrap_or_default();

        let server_content_blocks: Vec<Value> = obj
            .get("server_content_blocks")
            .and_then(|v| v.as_array())
            .map(|v| v.iter().cloned().collect())
            .unwrap_or_default();

        let summarized_range: Option<(usize, usize)> = obj
            .get("summarized_range")
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        let summarization_tier: Option<String> = obj
            .get("summarization_tier")
            .and_then(|v| v.as_str().map(|s| s.to_string()));
        let summarized_token_estimate: Option<usize> = obj
            .get("summarized_token_estimate")
            .and_then(|v| v.as_u64().map(|n| n as usize));

        let usage: Option<ChatUsage> = obj
            .get("usage")
            .and_then(|v| serde_json::from_value(v.clone()).ok());

        let checkpoints: Vec<Checkpoint> = obj
            .get("checkpoints")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        const KNOWN_FIELDS: &[&str] = &[
            "role",
            "content",
            "message_id",
            "finish_reason",
            "reasoning_content",
            "tool_calls",
            "tool_call_id",
            "tool_failed",
            "preserve",
            "usage",
            "checkpoints",
            "thinking_blocks",
            "citations",
            "server_content_blocks",
            "summarized_range",
            "summarization_tier",
            "summarized_token_estimate",
        ];
        let extra: serde_json::Map<String, Value> = obj
            .iter()
            .filter(|(k, _)| !KNOWN_FIELDS.contains(&k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        Ok(ChatMessage {
            message_id,
            role,
            content,
            finish_reason,
            reasoning_content,
            tool_calls,
            tool_call_id,
            tool_failed,
            preserve,
            usage,
            checkpoints,
            thinking_blocks,
            citations,
            server_content_blocks,
            summarized_range,
            summarization_tier,
            summarized_token_estimate,
            extra,
            output_filter: None,
        })
    }
}

impl ChatMessage {
    pub fn new(role: String, content: String) -> Self {
        ChatMessage {
            role,
            content: ChatContent::SimpleText(content),
            ..Default::default()
        }
    }
}

fn default_gradient_type_value() -> i32 {
    -1
}

#[derive(Debug, Serialize, Clone, PartialEq)]
pub struct ContextFile {
    pub file_name: String,
    pub file_content: String,
    pub line1: usize,
    pub line2: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_rev: Option<String>,
    #[serde(default, skip_serializing)]
    pub symbols: Vec<String>,
    #[serde(default = "default_gradient_type_value", skip_serializing)]
    pub gradient_type: i32,
    #[serde(default, skip_serializing)]
    pub usefulness: f32,
    #[serde(default, skip_serializing)]
    pub skip_pp: bool,
}

/// Normalize path separators to forward slashes for cross-platform consistency.
/// All ContextFile.file_name values use forward slashes regardless of OS.
/// This is unconditional (not cfg-gated) so serialized paths are always portable.
pub fn normalize_file_name(name: String) -> String {
    name.replace('\\', "/")
}

impl<'de> Deserialize<'de> for ContextFile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Raw {
            file_name: String,
            file_content: String,
            #[serde(default)]
            line1: usize,
            #[serde(default)]
            line2: usize,
            #[serde(default)]
            file_rev: Option<String>,
            #[serde(default)]
            symbols: Vec<String>,
            #[serde(default = "default_gradient_type_value")]
            gradient_type: i32,
            #[serde(default)]
            usefulness: f32,
            #[serde(default)]
            skip_pp: bool,
        }
        let raw = Raw::deserialize(deserializer)?;
        Ok(ContextFile {
            file_name: normalize_file_name(raw.file_name),
            file_content: raw.file_content,
            line1: raw.line1,
            line2: raw.line2,
            file_rev: raw.file_rev,
            symbols: raw.symbols,
            gradient_type: raw.gradient_type,
            usefulness: raw.usefulness,
            skip_pp: raw.skip_pp,
        })
    }
}

impl ContextFile {
    /// Set file_name with cross-platform normalization (backslash → forward slash).
    pub fn set_file_name(&mut self, name: String) {
        self.file_name = normalize_file_name(name);
    }
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
    pub fn parse_args(
        &self,
    ) -> Result<std::collections::HashMap<String, serde_json::Value>, serde_json::Error> {
        let trimmed = self.arguments.trim();
        let args_str = if trimmed.starts_with('{') {
            trimmed
        } else {
            "{}"
        };
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_content: Option<serde_json::Value>,
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

impl ChatContent {
    pub fn content_text_only(&self) -> String {
        match self {
            ChatContent::SimpleText(text) => text.clone(),
            ChatContent::Multimodal(elements) => elements
                .iter()
                .filter(|el| el.m_type == "text")
                .map(|el| el.m_content.clone())
                .collect::<Vec<_>>()
                .join("\n\n"),
            ChatContent::ContextFiles(files) => files
                .iter()
                .map(|f| {
                    format!(
                        "{}:{}-{}\n{}",
                        f.file_name, f.line1, f.line2, f.file_content
                    )
                })
                .collect::<Vec<_>>()
                .join("\n\n"),
        }
    }

    pub fn to_text_with_image_placeholders(&self) -> String {
        match self {
            ChatContent::SimpleText(_) | ChatContent::ContextFiles(_) => self.content_text_only(),
            ChatContent::Multimodal(elements) => {
                let parts: Vec<String> = elements
                    .iter()
                    .map(|el| {
                        if el.is_text() {
                            el.m_content.clone()
                        } else if el.is_image() {
                            "[image]".to_string()
                        } else {
                            format!("[unsupported:{}]", el.m_type)
                        }
                    })
                    .filter(|s| !s.is_empty())
                    .collect();
                parts.join("\n\n")
            }
        }
    }

    pub fn size_estimate(
        &self,
        tokenizer: Option<Arc<Tokenizer>>,
        style: &Option<String>,
    ) -> usize {
        match self {
            ChatContent::SimpleText(text) => text.len(),
            ChatContent::Multimodal(_elements) => {
                let tcnt = self.count_tokens(tokenizer, style).unwrap_or(0);
                (tcnt as f32 * 2.618) as usize
            }
            ChatContent::ContextFiles(files) => files
                .iter()
                .map(|f| f.file_content.len() + f.file_name.len())
                .sum(),
        }
    }

    pub fn count_tokens(
        &self,
        tokenizer: Option<Arc<Tokenizer>>,
        style: &Option<String>,
    ) -> Result<i32, String> {
        match self {
            ChatContent::SimpleText(text) => Ok(count_text_tokens(tokenizer, text)? as i32),
            ChatContent::Multimodal(elements) => elements
                .iter()
                .map(|e| e.count_tokens(tokenizer.clone(), style))
                .collect::<Result<Vec<_>, _>>()
                .map(|counts| counts.iter().sum()),
            ChatContent::ContextFiles(files) => {
                let total: i32 = files
                    .iter()
                    .map(|f| {
                        count_text_tokens(tokenizer.clone(), &f.file_content).unwrap_or(0) as i32
                    })
                    .sum();
                Ok(total)
            }
        }
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
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "cache_creation_input_tokens",
        alias = "cache_creation_tokens"
    )]
    pub cache_creation_tokens: Option<usize>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "cache_read_input_tokens",
        alias = "cache_read_tokens"
    )]
    pub cache_read_tokens: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metering_usd: Option<MeteringUsd>,
}

fn default_limit_lines() -> usize {
    50
}
fn default_limit_chars() -> usize {
    8000
}
fn default_valuable_top_or_bottom() -> String {
    "top".to_string()
}
fn default_grep() -> String {
    "(?i)(error|failed|exception|warning|fatal|panic|traceback)".to_string()
}
fn default_grep_context_lines() -> usize {
    3
}
fn default_remove_from_output() -> String {
    String::new()
}
fn default_limit_tokens() -> Option<usize> {
    Some(8000)
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct OutputFilter {
    #[serde(default = "default_limit_lines")]
    pub limit_lines: usize,
    #[serde(default = "default_limit_chars")]
    pub limit_chars: usize,
    #[serde(default = "default_valuable_top_or_bottom")]
    pub valuable_top_or_bottom: String,
    #[serde(default = "default_grep")]
    pub grep: String,
    #[serde(default = "default_grep_context_lines")]
    pub grep_context_lines: usize,
    #[serde(default = "default_remove_from_output")]
    pub remove_from_output: String,
    #[serde(default = "default_limit_tokens")]
    pub limit_tokens: Option<usize>,
    #[serde(default)]
    pub skip: bool,
}

impl Default for OutputFilter {
    fn default() -> Self {
        OutputFilter {
            limit_lines: default_limit_lines(),
            limit_chars: default_limit_chars(),
            valuable_top_or_bottom: default_valuable_top_or_bottom(),
            grep: default_grep(),
            grep_context_lines: default_grep_context_lines(),
            remove_from_output: default_remove_from_output(),
            limit_tokens: default_limit_tokens(),
            skip: false,
        }
    }
}

impl OutputFilter {
    pub fn no_limits() -> Self {
        OutputFilter {
            limit_lines: usize::MAX,
            limit_chars: usize::MAX,
            limit_tokens: None,
            grep: String::new(),
            remove_from_output: String::new(),
            skip: true,
            ..Default::default()
        }
    }
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
    pub preserve: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<ChatUsage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checkpoints: Vec<Checkpoint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_blocks: Option<Vec<serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub citations: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub server_content_blocks: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summarized_range: Option<(usize, usize)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summarization_tier: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summarized_token_estimate: Option<usize>,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty", flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
    #[serde(skip)]
    pub output_filter: Option<OutputFilter>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

pub fn format_search_results(query: &str, results: &[SearchResult]) -> String {
    if results.is_empty() {
        return format!("No web search results found for \"{}\".", query);
    }
    let mut output = format!("Web search results for \"{}\":\n\n", query);
    for (i, result) in results.iter().enumerate() {
        output.push_str(&format!("{}. [{}]({})\n", i + 1, result.title, result.url));
        if !result.snippet.is_empty() {
            output.push_str(&format!("   {}\n", result.snippet));
        }
        output.push('\n');
    }
    output
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
#[serde(default)]
pub struct PostprocessSettings {
    pub use_ast_based_pp: bool,
    pub useful_background: f32,
    pub useful_symbol_default: f32,
    pub downgrade_parent_coef: f32,
    pub downgrade_body_coef: f32,
    pub comments_propagate_up_coef: f32,
    pub close_small_gaps: bool,
    pub take_floor: f32,
    pub max_files_n: usize,
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

pub fn normalize_mode_id(mode: &str) -> Result<String, String> {
    let trimmed = mode.trim();

    if trimmed.is_empty() {
        return Ok("agent".to_string());
    }

    if !trimmed
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    {
        let normalized = trimmed.to_lowercase();
        if !normalized
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
        {
            return Err(format!(
                "Invalid mode ID: '{}' contains invalid characters",
                trimmed
            ));
        }
        return Ok(normalized);
    }

    Ok(trimmed.to_string())
}

pub fn canonical_mode_id(mode: &str) -> Result<String, String> {
    let trimmed = mode.trim();

    if trimmed.is_empty() {
        return Ok("agent".to_string());
    }

    if trimmed.len() > 128 {
        return Err(format!(
            "Mode ID too long: {} chars (max 128)",
            trimmed.len()
        ));
    }

    let normalized = normalize_mode_id(trimmed)?;

    let canonical = match normalized.to_uppercase().as_str() {
        "NO_TOOLS" => "explore".to_string(),
        "EXPLORE" => "explore".to_string(),
        "AGENT" => "agent".to_string(),
        "CONFIGURE" | "CONFIGURATOR" => "configurator".to_string(),
        "PLAN" => "plan".to_string(),
        "TASK_PLANNER" => "task_planner".to_string(),
        "TASK_AGENT" => "task_agent".to_string(),
        _ => normalized,
    };

    Ok(canonical)
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
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
    pub thinking: Option<Value>,
    #[serde(default)]
    pub enable_thinking: Option<bool>,
}

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
    #[serde(default, skip_deserializing)]
    pub cache_salt: String,
    #[serde(default, skip_deserializing)]
    pub cache_generation: u64,
}
