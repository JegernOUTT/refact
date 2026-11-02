use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum, DiffChunk};
use crate::global_context::GlobalContext;
use crate::integrations::integr_abstract::IntegrationConfirmation;
use crate::privacy::load_privacy_if_needed;
use crate::tools::file_edit::auxiliary::{
    append_scope_warnings, convert_edit_to_diffchunks, fast_enqueue_for_edit,
    parse_path_for_update, scope_warnings_to_tool_message,
};
use crate::files_in_workspace::remove_memory_document_for_path;
use crate::tools::file_edit::undo_history::{get_undo_history, UndoEntry};
use crate::tools::tools_description::{
    MatchConfirmDeny, MatchConfirmDenyResult, Tool, ToolDesc, ToolSource, ToolSourceType,
    json_schema_from_params,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use crate::worktrees::scope::ExecutionScope;
use std::sync::Arc;
use tokio::sync::Mutex as AMutex;
use crate::knowledge_index::format_related_memories_section;

pub struct ToolUndoTextDoc {
    pub config_path: String,
}

struct Args {
    path: PathBuf,
    steps: usize,
    scope_warnings: Vec<String>,
}

async fn parse_args(
    gcx: Arc<GlobalContext>,
    args: &HashMap<String, Value>,
    execution_scope: Option<&ExecutionScope>,
) -> Result<Args, String> {
    let privacy = load_privacy_if_needed(gcx.clone()).await;
    let resolved = parse_path_for_update(gcx, args, privacy, execution_scope).await?;
    let path = resolved.path;
    let steps = match args.get("steps") {
        Some(Value::Number(n)) => n.as_u64().unwrap_or(1) as usize,
        Some(Value::String(s)) => s.parse().unwrap_or(1),
        _ => 1,
    };
    if steps == 0 {
        return Err("⚠️ steps must be >= 1".to_string());
    }
    Ok(Args {
        path,
        steps,
        scope_warnings: resolved.warnings,
    })
}

pub async fn tool_undo_text_doc_exec(
    gcx: Arc<GlobalContext>,
    args: &HashMap<String, Value>,
    execution_scope: Option<&ExecutionScope>,
) -> Result<(String, String, Vec<DiffChunk>, String), String> {
    let a = parse_args(gcx.clone(), args, execution_scope).await?;

    let actor = crate::file_activity::current_actor();
    let write_guard = crate::file_activity::begin_write(
        actor.as_deref(),
        &a.path,
        crate::file_activity::WriteIntent::Contextual,
        "undo_textdoc",
    )
    .await?;
    let history = get_undo_history();
    let entries: Vec<UndoEntry> = {
        let h = history.lock().unwrap();
        h.get(&a.path).cloned().unwrap_or_default()
    };

    if entries.is_empty() {
        return Err(format!(
            "⚠️ No undo history for {:?}. 💡 Only edits from this session can be undone",
            a.path
        ));
    }
    if a.steps > entries.len() {
        return Err(format!(
            "⚠️ Only {} undo steps available, requested {}. 💡 Use steps:{}",
            entries.len(),
            a.steps,
            entries.len()
        ));
    }

    let target_idx = entries.len() - a.steps;
    let target_content = &entries[target_idx].content;
    if entries[target_idx..]
        .iter()
        .any(|entry| entry.actor != actor)
    {
        return Err("undo history belongs to a different writer".into());
    }

    let current_content = tokio::fs::read_to_string(&a.path)
        .await
        .map_err(|e| format!("⚠️ Failed to read {:?}: {}", a.path, e))?;

    if entries.last().and_then(|entry| entry.after_digest.as_ref())
        != Some(&crate::file_activity::digest_of(&current_content))
    {
        return Err(
            "undo target changed since the recorded edit; reread and edit explicitly".into(),
        );
    }
    if crate::files_in_workspace::get_file_text_from_memory_or_disk(gcx.clone(), &a.path).await?
        != current_content
    {
        return Err("undo buffer differs from disk; save or reload before retrying".into());
    }
    let delete = !entries[target_idx].existed;
    if delete {
        tokio::fs::remove_file(&a.path)
            .await
            .map_err(|e| format!("⚠️ Failed to delete {:?}: {}", a.path, e))?;
    } else {
        tokio::fs::write(&a.path, target_content)
            .await
            .map_err(|e| format!("⚠️ Failed to write {:?}: {}", a.path, e))?;
    }

    {
        let mut h = history.lock().unwrap();
        if let Some(list) = h.get_mut(&a.path) {
            list.truncate(target_idx);
        }
    }

    remove_memory_document_for_path(gcx.clone(), &a.path).await;
    write_guard.commit(
        if delete {
            crate::file_activity::WriteKind::Deleted
        } else {
            crate::file_activity::WriteKind::Modified
        },
        (!delete).then_some(target_content.as_str()),
    );

    let summary = if delete {
        fast_enqueue_for_edit(gcx.clone(), &[a.path.clone()]).await?;
        format!(
            "✅ Undid {} step(s), deleted {:?}",
            a.steps,
            a.path.file_name().unwrap_or_default()
        )
    } else {
        fast_enqueue_for_edit(gcx.clone(), &[a.path.clone()]).await?;
        format!(
            "✅ Undid {} step(s) on {:?}",
            a.steps,
            a.path.file_name().unwrap_or_default()
        )
    };

    let chunks = convert_edit_to_diffchunks(a.path.clone(), &current_content, target_content)?;
    let summary = append_scope_warnings(summary, &a.scope_warnings);
    Ok((current_content, target_content.clone(), chunks, summary))
}

#[async_trait]
impl Tool for ToolUndoTextDoc {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (gcx, execution_scope, memory_reader) = {
            let cgcx = ccx.lock().await;
            (
                cgcx.app.gcx.clone(),
                cgcx.execution_scope.clone(),
                crate::tools::tool_task_memory::related_memory_access_context(
                    cgcx.task_meta.as_ref(),
                ),
            )
        };
        let resolved = parse_args(gcx.clone(), args, execution_scope.as_ref()).await?;
        crate::tools::file_edit::auxiliary::check_scope_guard(&ccx, &resolved.path).await?;
        let (before, after, chunks, summary) =
            tool_undo_text_doc_exec(gcx.clone(), args, execution_scope.as_ref()).await?;

        let related_section = {
            let idx_arc = { gcx.knowledge_index.clone() };
            let idx_guard = idx_arc.lock().await;
            let mut paths: Vec<String> = Vec::new();
            for c in chunks.iter() {
                if !c.file_name.is_empty() {
                    paths.push(c.file_name.clone());
                }
                if let Some(rename) = &c.file_name_rename {
                    if !rename.is_empty() {
                        paths.push(rename.clone());
                    }
                }
            }
            paths.sort();
            paths.dedup();
            let mut cards = idx_guard.related_for_files(&paths, 8);
            if cards.is_empty() {
                cards = idx_guard.related_for_related_files(&paths, 8);
            }
            format_related_memories_section(&cards, &memory_reader)
        };

        let mut out = vec![ContextEnum::ChatMessage(ChatMessage {
            role: "diff".to_string(),
            content: ChatContent::SimpleText(json!(chunks).to_string()),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            extra: serde_json::Map::from_iter([(
                "diff_state".to_string(),
                Value::String("applied".to_string()),
            )]),
            ..Default::default()
        })];

        if let Some(message) = scope_warnings_to_tool_message(&summary, tool_call_id) {
            out.push(message);
        }

        if !related_section.trim().is_empty() {
            out.push(ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(related_section),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            }));
        }

        let path = chunks
            .first()
            .map(|chunk| chunk.file_name.as_str())
            .unwrap_or_else(|| {
                args.get("path")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
            });
        let model_content = super::edit_output::edit_model_content(
            path,
            if summary.contains(", deleted ") { "delete (undo_textdoc)" } else { "undo_textdoc" },
            Some(&before),
            if summary.contains(", deleted ") {
                None
            } else {
                Some(after.as_str())
            },
            &chunks,
        );
        super::edit_output::finish_edit_output(&mut out, model_content);
        Ok((false, out))
    }

    async fn match_against_confirm_deny(
        &self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        args: &HashMap<String, Value>,
    ) -> Result<MatchConfirmDeny, String> {
        let (gcx, execution_scope) = {
            let cgcx = ccx.lock().await;
            (cgcx.app.gcx.clone(), cgcx.execution_scope.clone())
        };
        let can_exec = parse_args(gcx, args, execution_scope.as_ref())
            .await
            .is_ok();
        if !can_exec {
            return Ok(MatchConfirmDeny {
                result: MatchConfirmDenyResult::PASS,
                command: "undo_textdoc".to_string(),
                rule: "".to_string(),
            });
        }
        Ok(MatchConfirmDeny {
            result: MatchConfirmDenyResult::CONFIRMATION,
            command: "undo_textdoc".to_string(),
            rule: "default".to_string(),
        })
    }

    async fn command_to_match_against_confirm_deny(
        &self,
        _ccx: Arc<AMutex<AtCommandsContext>>,
        _args: &HashMap<String, Value>,
    ) -> Result<String, String> {
        Ok("undo_textdoc".to_string())
    }

    fn confirm_deny_rules(&self) -> Option<IntegrationConfirmation> {
        Some(IntegrationConfirmation {
            ask_user: vec!["undo_textdoc*".to_string()],
            deny: vec![],
        })
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "undo_textdoc".to_string(),
            display_name: "Undo Text Document".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Undo recent file edits from this session. Reverts to previous version."
                .to_string(),
            input_schema: json_schema_from_params(
                &[
                    ("path", "string", "Absolute path to the file to undo."),
                    ("steps", "integer", "Number of edits to undo (default: 1)."),
                ],
                &["path"],
            ),
            output_schema: None,
            annotations: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::file_edit::auxiliary::write_file;

    #[tokio::test]
    async fn undo_empty_file_is_not_delete_and_successive_undo_consumes_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.txt");
        std::fs::write(&path, "").unwrap();
        let path = crate::files_correction::canonicalize_normalized_path(path);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().into()];
        write_file(gcx.clone(), &path, &"one".into(), false, Some(""))
            .await
            .unwrap();
        write_file(gcx.clone(), &path, &"two".into(), false, Some("one"))
            .await
            .unwrap();
        let args = HashMap::from([("path".into(), json!(path))]);
        tool_undo_text_doc_exec(gcx.clone(), &args, None)
            .await
            .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "one");
        tool_undo_text_doc_exec(gcx.clone(), &args, None)
            .await
            .unwrap();
        assert!(path.exists());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "");
        assert!(tool_undo_text_doc_exec(gcx, &args, None).await.is_err());
    }

    #[tokio::test]
    async fn external_edit_cannot_be_overwritten_by_undo_even_without_read_stamp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file.txt");
        std::fs::write(&path, "before").unwrap();
        let path = crate::files_correction::canonicalize_normalized_path(path);
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().into()];
        write_file(gcx.clone(), &path, &"after".into(), false, Some("before"))
            .await
            .unwrap();
        std::fs::write(&path, "external").unwrap();
        let args = HashMap::from([("path".into(), json!(path))]);
        assert!(tool_undo_text_doc_exec(gcx, &args, None)
            .await
            .unwrap_err()
            .contains("changed since"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "external");
    }
}
