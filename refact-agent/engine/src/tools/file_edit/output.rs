use crate::call_validation::{ContextEnum, DiffChunk};

#[derive(serde::Serialize, schemars::JsonSchema)]
pub(crate) struct EditOutput {
    pub files: Vec<EditFileOutput>,
}

#[derive(serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EditAction {
    Created,
    Modified,
    Deleted,
    Renamed,
}

#[derive(serde::Serialize, schemars::JsonSchema)]
pub(crate) struct EditFileOutput {
    pub path: String,
    pub action: EditAction,
    pub lines_added: usize,
    pub lines_removed: usize,
    pub renamed_from: Option<String>,
}

impl EditFileOutput {
    pub fn from_chunks(
        path: String,
        action: &str,
        chunks: &[DiffChunk],
        before: &str,
        after: &str,
    ) -> Self {
        let mut lines_added: usize = chunks.iter().map(|c| c.lines_add.lines().count()).sum();
        let mut lines_removed: usize = chunks.iter().map(|c| c.lines_remove.lines().count()).sum();
        // diff::lines includes a terminal empty sentinel for empty/nonempty transitions.
        let delta = after.lines().count() as isize - before.lines().count() as isize;
        let chunk_delta = lines_added as isize - lines_removed as isize;
        if chunk_delta > delta {
            lines_added = lines_added.saturating_sub((chunk_delta - delta) as usize);
        } else {
            lines_removed = lines_removed.saturating_sub((delta - chunk_delta) as usize);
        }
        Self {
            path,
            action: match action {
                "add" => EditAction::Created,
                "delete" | "remove" => EditAction::Deleted,
                "rename" => EditAction::Renamed,
                _ => EditAction::Modified,
            },
            lines_added,
            lines_removed,
            renamed_from: (action == "rename")
                .then(|| chunks.first().map(|c| c.file_name.clone()))
                .flatten(),
        }
    }
}

pub(crate) fn attach(
    messages: &mut [ContextEnum],
    _tool_call_id: &str,
    output: &impl serde::Serialize,
) {
    if let Some(message) = messages.iter_mut().find_map(|item| match item {
        ContextEnum::ChatMessage(message) if message.role == "diff" || message.role == "tool" => {
            Some(message)
        }
        _ => None,
    }) {
        message.extra.insert(
            refact_tool_api::tool_output::TOOL_OUTPUT_EXTRA_KEY.into(),
            serde_json::to_value(output).expect("tool output serialization"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::call_validation::ChatMessage;

    #[test]
    fn attachment_preserves_every_message_and_diff_byte() {
        let mut diff = ChatMessage::new("diff".into(), "[{\"file_action\":\"edit\"}]".into());
        diff.tool_call_id = "call".into();
        let note = ChatMessage::new("tool".into(), "scope warning".into());
        let mut messages = vec![
            ContextEnum::ChatMessage(diff),
            ContextEnum::ChatMessage(note),
        ];
        let before = serde_json::to_value(
            messages
                .iter()
                .filter_map(|item| match item {
                    ContextEnum::ChatMessage(message) => Some(message),
                    _ => None,
                })
                .collect::<Vec<_>>(),
        )
        .unwrap();
        attach(
            &mut messages,
            "call",
            &EditOutput {
                files: vec![],
            },
        );
        for item in &mut messages {
            if let ContextEnum::ChatMessage(message) = item {
                message
                    .extra
                    .remove(refact_tool_api::tool_output::TOOL_OUTPUT_EXTRA_KEY);
            }
        }
        let after = serde_json::to_value(
            messages
                .iter()
                .filter_map(|item| match item {
                    ContextEnum::ChatMessage(message) => Some(message),
                    _ => None,
                })
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert_eq!(before, after);
    }
}
