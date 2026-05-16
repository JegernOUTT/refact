pub use refact_agentic::generate_follow_up_message::{FollowUpResponse, make_conversation};

use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;

use crate::global_context::GlobalContext;
use crate::subchat::run_subchat_once;
use crate::call_validation::{ChatContent, ChatMessage};
use crate::json_utils;
use crate::yaml_configs::customization_registry::get_subagent_config;
use crate::custom_error::MapErrToString;

const SUBAGENT_ID: &str = "follow_up";

pub async fn generate_follow_up_message(
    messages: Vec<ChatMessage>,
    gcx: Arc<ARwLock<GlobalContext>>,
    _model_id: &str,
    _chat_id: &str,
) -> Result<FollowUpResponse, String> {
    let gcx2 = gcx.clone();
    crate::buddy::workflows::buddy_wrap_workflow(
        gcx,
        "follow_up",
        "💡",
        3,
        |_: &FollowUpResponse| "Follow-up suggested".to_string(),
        move || async move {
            let subagent_config = get_subagent_config(gcx2.clone(), SUBAGENT_ID, None)
                .await
                .ok_or_else(|| format!("subagent config '{}' not found", SUBAGENT_ID))?;

            let system_prompt = subagent_config
                .messages
                .system_prompt
                .as_ref()
                .ok_or_else(|| {
                    format!(
                        "messages.system_prompt not defined for subagent '{}'",
                        SUBAGENT_ID
                    )
                })?
                .clone();

            let result = run_subchat_once(
                gcx2,
                SUBAGENT_ID,
                make_conversation(&messages, &system_prompt),
            )
            .await?;

            let response = result
                .messages
                .last()
                .and_then(|last_m| match &last_m.content {
                    ChatContent::SimpleText(text) => Some(text.clone()),
                    _ => None,
                })
                .ok_or("No follow-up message was generated".to_string())?;

            tracing::info!("follow-up model says {:?}", response);

            let response: FollowUpResponse = json_utils::extract_json_object(&response)
                .map_err_with_prefix("Failed to parse json:")?;
            Ok(response)
        },
    )
    .await
}
