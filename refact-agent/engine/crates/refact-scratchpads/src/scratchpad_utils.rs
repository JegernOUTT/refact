use serde_json::Value;

pub use refact_core::chat_types::{
    calculate_image_tokens_openai, image_reader_from_b64string,
    parse_image_b64_from_image_url_openai,
};
use refact_postprocessing::pp_context_files::RESERVE_FOR_QUESTION_AND_FOLLOWUP;

pub struct HasRagResults {
    was_sent: bool,
    pub in_json: Vec<Value>,
}

impl HasRagResults {
    pub fn new() -> Self {
        HasRagResults {
            was_sent: false,
            in_json: vec![],
        }
    }
}

impl HasRagResults {
    pub fn push_in_json(&mut self, value: Value) {
        self.in_json.push(value);
    }

    #[allow(dead_code)]
    fn response_streaming(&mut self) -> Result<Vec<Value>, String> {
        if self.was_sent == true || self.in_json.is_empty() {
            return Ok(vec![]);
        }
        self.was_sent = true;
        Ok(self.in_json.clone())
    }
}

pub fn max_tokens_for_rag_chat(n_ctx: usize, maxgen: usize) -> usize {
    (n_ctx / 4)
        .saturating_sub(maxgen)
        .saturating_sub(RESERVE_FOR_QUESTION_AND_FOLLOWUP)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_from_image_url_openai() {
        let image_url = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAUA";
        let expected_image_type = "image/png".to_string();
        let expected_encoding = "base64".to_string();
        let expected_base64 = "iVBORw0KGgoAAAANSUhEUgAAAAUA".to_string();
        assert_eq!(
            parse_image_b64_from_image_url_openai(image_url),
            Some((expected_image_type, expected_encoding, expected_base64))
        );

        let invalid_image_url = "data:image/png;base64,";
        assert_eq!(
            parse_image_b64_from_image_url_openai(invalid_image_url),
            None
        );

        let non_matching_url = "https://example.com/image.png";
        assert_eq!(
            parse_image_b64_from_image_url_openai(non_matching_url),
            None
        );
    }
}
