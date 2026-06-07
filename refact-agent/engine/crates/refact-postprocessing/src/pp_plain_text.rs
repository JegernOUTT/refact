use std::sync::Arc;
use tokenizers::Tokenizer;

use refact_core::chat_types::{ChatContent, ChatMessage, MultimodalElement};
use refact_ast::ast::chunk_utils::count_text_tokens_with_fallback;
use crate::pp_command_output::output_mini_postprocessing;

const MIN_PLAIN_TEXT_MESSAGE_TOKENS: usize = 50;
const TRUNCATION_MARKER: &str = "Truncated: too many tokens";

fn truncate_text_to_token_budget(
    tokenizer: Option<Arc<Tokenizer>>,
    text: &str,
    limit_tokens: usize,
) -> String {
    if count_text_tokens_with_fallback(tokenizer.clone(), text) <= limit_tokens {
        return text.to_string();
    }

    let marker = format!("\n{TRUNCATION_MARKER}");
    if count_text_tokens_with_fallback(tokenizer.clone(), &marker) >= limit_tokens {
        return TRUNCATION_MARKER.to_string();
    }

    let chars: Vec<char> = text.chars().collect();
    let mut low = 0usize;
    let mut high = chars.len();
    let mut best = 0usize;

    while low <= high {
        let mid = low + (high - low) / 2;
        let prefix: String = chars[..mid].iter().collect();
        let candidate = format!("{prefix}{marker}");
        let tokens = count_text_tokens_with_fallback(tokenizer.clone(), &candidate);
        if tokens <= limit_tokens {
            best = mid;
            low = mid.saturating_add(1);
        } else if mid == 0 {
            break;
        } else {
            high = mid - 1;
        }
    }

    let prefix: String = chars[..best].iter().collect();
    format!("{prefix}{marker}")
}

fn limit_text_by_tokens(
    tokenizer: Option<Arc<Tokenizer>>,
    text: &str,
    limit_tokens: usize,
) -> (String, usize) {
    let mut new_text_lines = vec![];
    let mut tok_used = 0;
    for line in text.lines() {
        let line_tokens = count_text_tokens_with_fallback(tokenizer.clone(), line);
        if tok_used + line_tokens > limit_tokens {
            if new_text_lines.is_empty() {
                let truncated =
                    truncate_text_to_token_budget(tokenizer.clone(), line, limit_tokens);
                let used = count_text_tokens_with_fallback(tokenizer.clone(), &truncated);
                return (truncated, used.min(limit_tokens));
            }
            new_text_lines.push(TRUNCATION_MARKER);
            break;
        }
        tok_used += line_tokens;
        new_text_lines.push(line);
    }
    let result = new_text_lines.join("\n");
    let used = count_text_tokens_with_fallback(tokenizer, &result);
    (result, used.min(limit_tokens))
}

fn fair_message_token_limit(
    remaining_budget: usize,
    messages_left: usize,
    per_message_limit: Option<usize>,
) -> usize {
    if messages_left <= 1 {
        return per_message_limit
            .unwrap_or(remaining_budget)
            .min(remaining_budget);
    }

    let fair_share = remaining_budget / messages_left;
    per_message_limit
        .unwrap_or(fair_share)
        .min(fair_share)
        .min(remaining_budget)
}

pub async fn postprocess_plain_text(
    plain_text_messages: Vec<ChatMessage>,
    tokenizer: Option<Arc<Tokenizer>>,
    tokens_limit: usize,
    style: &Option<String>,
) -> (Vec<ChatMessage>, usize) {
    if plain_text_messages.is_empty() {
        return (vec![], tokens_limit);
    }

    let mut remaining_budget = tokens_limit;
    let mut new_messages = vec![];
    let total_messages = plain_text_messages.len();

    for (idx, mut msg) in plain_text_messages.into_iter().enumerate() {
        if let Some(ref filter) = msg.output_filter {
            if filter.limit_lines < usize::MAX
                || filter.limit_chars < usize::MAX
                || !filter.grep.is_empty()
                || !filter.remove_from_output.is_empty()
            {
                msg.content = match msg.content {
                    ChatContent::SimpleText(text) => {
                        ChatContent::SimpleText(output_mini_postprocessing(filter, &text))
                    }
                    ChatContent::Multimodal(elements) => {
                        let filtered_elements = elements
                            .into_iter()
                            .map(|mut el| {
                                if el.is_text() {
                                    el.m_content =
                                        output_mini_postprocessing(filter, &el.m_content);
                                }
                                el
                            })
                            .collect();
                        ChatContent::Multimodal(filtered_elements)
                    }
                    ChatContent::ContextFiles(files) => ChatContent::ContextFiles(files),
                };
            }
        }

        let per_msg_limit = msg.output_filter.as_ref().and_then(|f| f.limit_tokens);
        msg.output_filter = None;

        let messages_left = total_messages.saturating_sub(idx).max(1);
        let effective_limit =
            fair_message_token_limit(remaining_budget, messages_left, per_msg_limit);

        if effective_limit < MIN_PLAIN_TEXT_MESSAGE_TOKENS {
            msg.content =
                ChatContent::SimpleText("... truncated (token limit reached)".to_string());
            new_messages.push(msg);
            continue;
        }

        let tokens_used = match msg.content {
            ChatContent::SimpleText(ref text) => {
                let (new_content, used) =
                    limit_text_by_tokens(tokenizer.clone(), text, effective_limit);
                msg.content = ChatContent::SimpleText(new_content);
                used
            }
            ChatContent::Multimodal(ref elements) => {
                let mut new_content = vec![];
                let mut used_in_msg = 0;

                for element in elements {
                    if element.is_text() {
                        let remaining = effective_limit.saturating_sub(used_in_msg);
                        let (new_text, used) =
                            limit_text_by_tokens(tokenizer.clone(), &element.m_content, remaining);
                        used_in_msg += used;
                        new_content.push(MultimodalElement {
                            m_type: element.m_type.clone(),
                            m_content: new_text,
                        });
                    } else if element.is_image() {
                        let tokens = element.count_tokens(None, style).unwrap_or(0) as usize;
                        if used_in_msg + tokens > effective_limit {
                            new_content.push(MultimodalElement {
                                m_type: "text".to_string(),
                                m_content: "Image truncated: too many tokens".to_string(),
                            });
                        } else {
                            new_content.push(element.clone());
                            used_in_msg += tokens;
                        }
                    }
                }
                msg.content = ChatContent::Multimodal(new_content);
                used_in_msg
            }
            ChatContent::ContextFiles(_) => msg.content.size_estimate(tokenizer.clone(), style),
        };

        remaining_budget = remaining_budget.saturating_sub(tokens_used);
        new_messages.push(msg);
    }

    (new_messages, remaining_budget)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_message(id: &str, content: String) -> ChatMessage {
        ChatMessage {
            role: "tool".to_string(),
            tool_call_id: id.to_string(),
            content: ChatContent::SimpleText(content),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn parallel_plain_text_messages_share_budget() {
        let messages: Vec<ChatMessage> = (0..4)
            .map(|idx| {
                tool_message(
                    &format!("call_{idx}"),
                    format!("tool-{idx}: {}", "x".repeat(1000)),
                )
            })
            .collect();

        let (processed, remaining) = postprocess_plain_text(messages, None, 240, &None).await;

        assert_eq!(processed.len(), 4);
        assert!(remaining < MIN_PLAIN_TEXT_MESSAGE_TOKENS);
        for (idx, msg) in processed.iter().enumerate() {
            let text = msg.content.content_text_only();
            assert!(
                text.contains(&format!("tool-{idx}:")),
                "message {idx} should keep its own prefix, got {text:?}"
            );
            assert!(text.contains(TRUNCATION_MARKER));
            assert!(!text.contains("No content: tokens limit reached"));
        }
    }

    #[tokio::test]
    async fn single_huge_line_keeps_prefix_before_truncation_marker() {
        let content = format!("start:{}", "x".repeat(4_000));
        let messages = vec![tool_message("call_1", content)];

        let (processed, remaining) = postprocess_plain_text(messages, None, 100, &None).await;

        assert_eq!(processed.len(), 1);
        let text = processed[0].content.content_text_only();
        assert!(text.starts_with("start:"));
        assert!(text.contains(TRUNCATION_MARKER));
        assert!(!text.contains("No content: tokens limit reached"));
        assert_eq!(remaining, 0);
    }

    #[tokio::test]
    async fn single_plain_text_message_can_use_full_budget() {
        let content = "x".repeat(300);
        let messages = vec![tool_message("call_1", content.clone())];

        let (processed, remaining) = postprocess_plain_text(messages, None, 100, &None).await;

        assert_eq!(processed.len(), 1);
        assert_eq!(processed[0].content.content_text_only(), content);
        assert!(remaining > 0);
    }
}
