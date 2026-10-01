use std::collections::HashSet;

use crate::call_validation::ChatMessage;
use refact_chat_api::role_projection::{checkpoint_provenance, delta_revision, CHECKPOINT_SOURCE};

pub(crate) fn archived_delta_history(
    archive: &[ChatMessage],
    accepted_messages: &[ChatMessage],
    base_message_id: &str,
    subkind: &str,
) -> Vec<(u64, String)> {
    let start = archive
        .iter()
        .position(|message| message.message_id == base_message_id)
        .map_or(0, |index| index + 1);
    let accepted_start = accepted_messages
        .iter()
        .position(|message| message.message_id == base_message_id)
        .map_or(0, |index| index + 1);
    let mut revision_index = 0;
    let mut seen = HashSet::new();
    let mut history = Vec::new();
    for message in archive
        .iter()
        .skip(start)
        .chain(accepted_messages.iter().skip(accepted_start))
    {
        let event = message.extra.get("event");
        if message.role != crate::chat::internal_roles::EVENT_ROLE
            || event
                .and_then(|event| event.get("subkind"))
                .and_then(serde_json::Value::as_str)
                != Some(subkind)
            || event
                .and_then(|event| event.get("source"))
                .and_then(serde_json::Value::as_str)
                == Some(CHECKPOINT_SOURCE)
            || checkpoint_provenance(message).is_some()
            || !seen.insert(message.message_id.clone())
        {
            continue;
        }
        let revision = delta_revision(message, revision_index);
        revision_index += 1;
        history.push((revision, message.content.content_text_only()));
    }
    history
}
