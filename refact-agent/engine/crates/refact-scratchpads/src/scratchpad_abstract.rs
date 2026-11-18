use std::sync::Arc;
use serde_json::Value;
use tokenizers::Tokenizer;
use tracing::warn;

use refact_core::chunk_utils::count_text_tokens;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FinishReason {
    None,
    Stop,
    Length,
    ScratchpadStop,
}

impl FinishReason {
    pub fn from_str(s: &str) -> FinishReason {
        match s {
            "" => FinishReason::None,
            "stop" => FinishReason::Stop,
            "stop-eot" => FinishReason::Stop,
            "stop-lf" => FinishReason::Stop,
            "tool_calls" => FinishReason::Stop,
            "length" => FinishReason::Length,
            "scratchpad-stop" => FinishReason::ScratchpadStop,
            _ => {
                warn!("Unknown finish reason: {}, interpreting it as a stop", s);
                FinishReason::Stop
            }
        }
    }

    pub fn from_json_val(json: &Value) -> Result<FinishReason, String> {
        if json.is_null() {
            return Ok(FinishReason::None);
        }
        if let Some(val) = json.as_str() {
            Ok(FinishReason::from_str(val))
        } else {
            Err(format!("expected string, got {}", json))
        }
    }

    pub fn to_string(&self) -> String {
        match self {
            FinishReason::None => "".to_string(),
            FinishReason::Stop => "stop".to_string(),
            FinishReason::Length => "length".to_string(),
            FinishReason::ScratchpadStop => "scratchpad-stop".to_string(),
        }
    }

    pub fn to_json_val(&self) -> Value {
        match self {
            FinishReason::None => Value::Null,
            _ => Value::String(self.to_string()),
        }
    }

    pub fn is_finished(&self) -> bool {
        self != &FinishReason::None
    }
}

#[derive(Debug, Clone)]
pub struct HasTokenizerAndEot {
    pub tokenizer: Option<Arc<Tokenizer>>,
    pub eot: String,
    pub eos: String,
    pub context_format: String,
    pub rag_ratio: f64,
}

impl HasTokenizerAndEot {
    pub fn new(tokenizer: Option<Arc<Tokenizer>>) -> Self {
        HasTokenizerAndEot {
            tokenizer,
            eot: String::new(),
            eos: String::new(),
            context_format: String::new(),
            rag_ratio: 0.5,
        }
    }

    pub fn count_tokens(&self, text: &str) -> Result<i32, String> {
        count_text_tokens(self.tokenizer.clone(), text).map(|t| t as i32)
    }

    pub fn assert_one_token(&self, text: &str) -> Result<(), String> {
        if self.tokenizer.is_none() {
            return Err("assert_one_token: no tokenizer".to_string());
        }

        let token_count = count_text_tokens(self.tokenizer.clone(), text)?;

        if token_count != 1 {
            Err(format!(
                "assert_one_token: expected 1 token for \"{text}\", got {token_count}"
            ))
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FinishReason;

    #[test]
    fn scratchpad_stop_round_trips() {
        let finish = FinishReason::from_str("scratchpad-stop");

        assert_eq!(finish, FinishReason::ScratchpadStop);
        assert_eq!(finish.to_string(), "scratchpad-stop");
        assert_eq!(
            FinishReason::from_json_val(&finish.to_json_val()).unwrap(),
            FinishReason::ScratchpadStop
        );
    }
}
