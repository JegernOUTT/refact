pub use refact_agentic::mode_transition::{
    ConversationMetadata,
    FileReference,
    ParsedDecisions,
    TransitionContextBudget,
    assemble_new_chat as assemble_new_chat_pure,
    calculate_transition_context_budget,
    context_file_rendered_symbols,
    count_images_in_messages,
    extract_conversation_metadata,
    format_annotated_messages,
    format_budget_summary,
    format_file_list,
    format_memory_list,
    message_symbols,
    parse_llm_response,
    push_context_file_with_budget,
    text_symbols,
    truncate_utf8,
};

use std::path::PathBuf;
use std::sync::Arc;

use refact_context_api::PathsAccess;
use tokio::sync::RwLock as ARwLock;

use crate::call_validation::{ChatContent, ChatMessage};
use crate::global_context::GlobalContext;
use crate::subchat::run_subchat_once;
use crate::yaml_configs::customization_registry::get_subagent_config;

const SUBAGENT_ID: &str = "mode_transition";

#[derive(Clone, Debug)]
pub struct AgenticPathContext {
    cache_dir: PathBuf,
    config_dir: PathBuf,
    workspace_folders: Vec<PathBuf>,
}

impl AgenticPathContext {
    pub fn from_context<T: PathsAccess + ?Sized>(context: &T) -> Self {
        Self {
            cache_dir: context.cache_dir(),
            config_dir: context.config_dir(),
            workspace_folders: context.workspace_folders(),
        }
    }
}

impl PathsAccess for AgenticPathContext {
    fn cache_dir(&self) -> PathBuf {
        self.cache_dir.clone()
    }

    fn config_dir(&self) -> PathBuf {
        self.config_dir.clone()
    }

    fn workspace_folders(&self) -> Vec<PathBuf> {
        self.workspace_folders.clone()
    }
}

pub async fn analyze_mode_transition(
    gcx: Arc<GlobalContext>,
    messages: &[ChatMessage],
    target_mode: &str,
    target_mode_description: &str,
) -> Result<ParsedDecisions, String> {
    if messages.is_empty() {
        return Err("The provided chat is empty".to_string());
    }

    let subagent_config = get_subagent_config(gcx.clone(), SUBAGENT_ID, None)
        .await
        .ok_or_else(|| format!("subagent config '{}' not found", SUBAGENT_ID))?;

    let user_template = subagent_config
        .messages
        .user_template
        .as_ref()
        .ok_or_else(|| {
            format!(
                "messages.user_template not defined for subagent '{}'",
                SUBAGENT_ID
            )
        })?;

    let metadata = extract_conversation_metadata(messages);
    let budget = calculate_transition_context_budget(messages);

    let annotated_message_list = format_annotated_messages(&metadata);
    let file_list = format_file_list(&metadata);
    let memory_list = format_memory_list(&metadata);
    let budget_summary = format_budget_summary(budget, messages);

    let user_prompt = user_template
        .replace("{target_mode}", target_mode)
        .replace("{target_mode_description}", target_mode_description)
        .replace("{annotated_message_list}", &annotated_message_list)
        .replace("{file_list}", &file_list)
        .replace("{memory_list}", &memory_list)
        .replace("{budget_summary}", &budget_summary);

    let analysis_messages = vec![ChatMessage {
        role: "user".to_string(),
        content: ChatContent::SimpleText(user_prompt),
        ..Default::default()
    }];

    let result = run_subchat_once(gcx, SUBAGENT_ID, analysis_messages)
        .await
        .map_err(|e| format!("Error analyzing mode transition: {}", e))?;

    let response_text = result
        .messages
        .last()
        .and_then(|msg| match &msg.content {
            ChatContent::SimpleText(text) => Some(text.clone()),
            _ => None,
        })
        .ok_or("No analysis response was generated".to_string())?;

    Ok(parse_llm_response(&response_text))
}

pub async fn assemble_new_chat<T: PathsAccess + ?Sized>(
    context: &T,
    original_messages: &[ChatMessage],
    decisions: &ParsedDecisions,
) -> Result<Vec<ChatMessage>, String> {
    let workspace_dirs = context.workspace_folders();
    assemble_new_chat_pure(original_messages, decisions, &workspace_dirs).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_validation::{ChatContent, ChatMessage, ContextFile};

    struct TestPaths {
        workspace_folders: Vec<PathBuf>,
    }

    impl TestPaths {
        fn new(workspace_folders: Vec<PathBuf>) -> Self {
            Self { workspace_folders }
        }
    }

    impl PathsAccess for TestPaths {
        fn cache_dir(&self) -> PathBuf {
            PathBuf::new()
        }

        fn config_dir(&self) -> PathBuf {
            PathBuf::new()
        }

        fn workspace_folders(&self) -> Vec<PathBuf> {
            self.workspace_folders.clone()
        }
    }

    #[tokio::test]
    async fn test_assemble_new_chat_limits_message_budget_and_images() {
        use crate::scratchpads::multimodality::MultimodalElement;

        let paths = TestPaths::new(vec![]);
        let original_messages = vec![
            ChatMessage {
                role: "user".to_string(),
                content: ChatContent::SimpleText("baseline ".repeat(400)),
                ..Default::default()
            },
            ChatMessage {
                role: "user".to_string(),
                content: ChatContent::Multimodal(vec![
                    MultimodalElement {
                        m_type: "text".to_string(),
                        m_content: "first image context ".repeat(100),
                    },
                    MultimodalElement {
                        m_type: "image/png".to_string(),
                        m_content: "base64data-one".to_string(),
                    },
                ]),
                ..Default::default()
            },
            ChatMessage {
                role: "user".to_string(),
                content: ChatContent::Multimodal(vec![
                    MultimodalElement {
                        m_type: "text".to_string(),
                        m_content: "second image context ".repeat(100),
                    },
                    MultimodalElement {
                        m_type: "image/png".to_string(),
                        m_content: "base64data-two".to_string(),
                    },
                ]),
                ..Default::default()
            },
        ];
        let budget = calculate_transition_context_budget(&original_messages);
        let decisions = ParsedDecisions {
            summary: "summary ".repeat(200),
            messages_to_preserve: vec!["MSG_ID:1".to_string(), "MSG_ID:2".to_string()],
            handoff_message: "continue ".repeat(200),
            ..Default::default()
        };

        let new_messages = assemble_new_chat(&paths, &original_messages, &decisions)
            .await
            .unwrap();
        let message_symbols = new_messages
            .iter()
            .filter(|msg| msg.role != "context_file")
            .map(|msg| text_symbols(&msg.content.content_text_only()))
            .sum::<usize>();
        let image_count = count_images_in_messages(&new_messages);

        assert!(message_symbols <= budget.messages_symbols);
        assert_eq!(image_count, 1);
    }

    #[tokio::test]
    async fn test_assemble_new_chat_limits_file_budget() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("src/main.rs");
        std::fs::create_dir_all(file_path.parent().unwrap()).unwrap();
        std::fs::write(
            &file_path,
            "fn main() { println!(\"hello\"); }\n".repeat(300),
        )
        .unwrap();

        let paths = TestPaths::new(vec![dir.path().to_path_buf()]);

        let original_messages = vec![
            ChatMessage {
                role: "context_file".to_string(),
                content: ChatContent::ContextFiles(vec![ContextFile {
                    file_name: "src/main.rs".to_string(),
                    file_content: "old content\n".repeat(300),
                    line1: 1,
                    line2: 300,
                    ..Default::default()
                }]),
                ..Default::default()
            },
            ChatMessage {
                role: "user".to_string(),
                content: ChatContent::SimpleText("requirements ".repeat(500)),
                ..Default::default()
            },
        ];
        let budget = calculate_transition_context_budget(&original_messages);
        let decisions = ParsedDecisions {
            files_to_open: vec!["src/main.rs".to_string()],
            ..Default::default()
        };

        let new_messages = assemble_new_chat(&paths, &original_messages, &decisions)
            .await
            .unwrap();
        let file_symbols = new_messages
            .iter()
            .filter(|msg| msg.role == "context_file")
            .map(|msg| text_symbols(&msg.content.content_text_only()))
            .sum::<usize>();

        assert!(file_symbols <= budget.files_symbols);
        assert!(file_symbols > 0);
    }
}
