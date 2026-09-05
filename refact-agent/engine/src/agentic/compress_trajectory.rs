use crate::call_validation::ChatMessage;
#[cfg(test)]
use crate::call_validation::ChatContent;
use crate::global_context::GlobalContext;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub struct CompressedTrajectory {
    pub text: String,
    pub records: Vec<refact_privacy::FileRecord>,
}

fn source_records(messages: &[ChatMessage]) -> Result<Vec<refact_privacy::FileRecord>, String> {
    crate::privacy::records::records_to_carry(messages).map_err(|error| error.to_string())
}

pub async fn compress_trajectory(
    gcx: Arc<GlobalContext>,
    messages: &Vec<ChatMessage>,
    parent_chat_id: Option<&str>,
) -> Result<CompressedTrajectory, String> {
    use refact_core::active_context::{active_context, legacy_rebuild_input, requires_explicit_rebuild};
    let input = if requires_explicit_rebuild(messages) {
        legacy_rebuild_input(messages)
    } else {
        active_context(messages).map(|view| view.messages)
    }
    .map_err(|e| e.to_string())?;
    let outcome = crate::agentic::mode_transition::reconstruct_context(
        gcx,
        crate::agentic::mode_transition::ReconstructionRequest {
            messages: &input,
            target_mode: "agent",
            target_mode_description: "Continue the current conversation",
            parent_chat_id,
            model_override: None,
            abort_flag: None,
            hints: None,
            target_budget_symbols: None,
            preserve_goal_messages: true,
        },
    )
    .await?;
    let text = outcome
        .messages
        .iter()
        .map(|m| format!("[{}]\n{}", m.role, m.content.content_text_only()))
        .collect::<Vec<_>>()
        .join("\n\n");
    Ok(CompressedTrajectory {
        text,
        records: source_records(&input)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use refact_privacy::{Attribution, PrivacyRecord};
    fn message(role: &str, text: &str) -> ChatMessage {
        ChatMessage {
            role: role.into(),
            content: ChatContent::SimpleText(text.into()),
            ..Default::default()
        }
    }
    #[test]
    fn source_records_unions_and_deduplicates_privacy_metadata() {
        let secret = refact_privacy::FileRecord {
            path: ".env".to_string(),
            zone: "secrets".to_string(),
            attribution: Attribution::Declared,
        };
        let internal = refact_privacy::FileRecord {
            path: "src/lib.rs".to_string(),
            zone: "internal".to_string(),
            attribution: Attribution::Observed,
        };
        let inert = refact_privacy::FileRecord {
            path: "src/main.rs".to_string(),
            zone: "normal".to_string(),
            attribution: Attribution::Observed,
        };
        let mut first = message("user", "first");
        first.extra.insert(
            "privacy".to_string(),
            serde_json::to_value(PrivacyRecord {
                files: vec![secret.clone(), internal.clone(), inert],
            })
            .unwrap(),
        );
        let mut second = message("assistant", "second");
        second.extra.insert(
            "privacy".to_string(),
            serde_json::to_value(PrivacyRecord {
                files: vec![secret.clone()],
            })
            .unwrap(),
        );

        assert_eq!(
            source_records(&[first, second]).unwrap(),
            vec![secret, internal]
        );
    }
}
