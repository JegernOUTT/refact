use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum, DiffChunk};
use crate::files_correction::{check_if_its_inside_a_workspace_or_config, get_unscoped_project_dirs};
use crate::files_in_workspace::get_file_text_from_memory_or_disk;
use crate::global_context::GlobalContext;
use crate::integrations::integr_abstract::IntegrationConfirmation;
use crate::privacy::{check_file_privacy, load_privacy_if_needed, FilePrivacyLevel};
use crate::tools::file_edit::auxiliary::{
    check_scope_guard, convert_edit_to_diffchunks, normalize_line_endings, resolve_path_with_scope,
    restore_line_endings, ResolvedToolPath,
};
use crate::worktrees::scope::ExecutionScope;
use crate::tools::file_edit::openai_apply_patch::{
    apply_update_chunks, parse_patch, validate_relative_path, FileOperation, ParsedPatch,
};
use crate::tools::tools_description::{
    MatchConfirmDeny, MatchConfirmDenyResult, Tool, ToolDesc, ToolSource, ToolSourceType,
    json_schema_from_params,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex as AMutex;
use crate::knowledge_index::format_related_memories_section;

pub struct ToolApplyPatch {
    pub config_path: String,
}

#[derive(Debug)]
pub struct ApplyPatchResult {
    pub file_results: Vec<SingleFileResult>,
    pub all_chunks: Vec<DiffChunk>,
    pub warnings: Vec<String>,
}

#[derive(Debug)]
pub struct SingleFileResult {
    pub path: PathBuf,
    pub action: &'static str,
    pub before: String,
    pub after: String,
    pub chunks: Vec<DiffChunk>,
}

fn parse_patch_arg(args: &HashMap<String, Value>) -> Result<ParsedPatch, String> {
    let patch_text = match args.get("patch") {
        Some(Value::String(s)) => s,
        _ => return Err("Missing 'patch' argument".to_string()),
    };
    parse_patch(patch_text).map_err(|e| e.to_string())
}

fn absolute_path_inside_workspace(path: &str, project_dirs: &[PathBuf]) -> Option<PathBuf> {
    let path = PathBuf::from(path);
    if !path.is_absolute() && !path.has_root() {
        return None;
    }

    let normalized_path = dunce::simplified(&path);
    project_dirs
        .iter()
        .any(|dir| {
            let dir = dunce::simplified(dir);
            normalized_path
                .strip_prefix(dir)
                .is_ok_and(|relative| !relative.as_os_str().is_empty())
        })
        .then_some(path)
}

async fn resolve_patch_path(
    gcx: Arc<GlobalContext>,
    rel_path: &str,
    must_exist: bool,
    execution_scope: Option<&ExecutionScope>,
) -> Result<ResolvedToolPath, String> {
    let requested_path = rel_path.trim();
    let privacy_settings = load_privacy_if_needed(gcx.clone()).await;
    if let Some(resolved) = resolve_path_with_scope(
        Path::new(requested_path),
        privacy_settings.clone(),
        execution_scope,
        must_exist,
    ) {
        return resolved;
    }

    let project_dirs = get_unscoped_project_dirs(gcx.clone()).await;
    if project_dirs.is_empty() {
        return Err("No workspace found".to_string());
    }

    let full_path = if let Some(absolute_path) =
        absolute_path_inside_workspace(requested_path, &project_dirs)
    {
        if must_exist && !absolute_path.exists() {
            return Err(format!(
                "File '{}' not found: {:?}",
                rel_path, absolute_path
            ));
        }
        absolute_path
    } else {
        let rel_path_buf = validate_relative_path(requested_path)?;
        if project_dirs.len() == 1 {
            project_dirs[0].join(&rel_path_buf)
        } else if must_exist {
            let existing: Vec<_> = project_dirs
                .iter()
                .map(|d| d.join(&rel_path_buf))
                .filter(|p| p.exists())
                .collect();
            if existing.len() == 1 {
                existing.into_iter().next().unwrap()
            } else if existing.is_empty() {
                return Err(format!(
                    "File '{}' not found in any workspace: {:?}",
                    rel_path, project_dirs
                ));
            } else {
                return Err(format!(
                    "File '{}' exists in multiple workspaces: {:?}",
                    rel_path, existing
                ));
            }
        } else {
            let active = crate::files_correction::get_active_project_path(gcx.clone())
                .await
                .ok_or_else(|| "No active workspace found for new file".to_string())?;
            active.join(&rel_path_buf)
        }
    };

    let canonical = if full_path.exists() {
        full_path
            .canonicalize()
            .map(|path| dunce::simplified(&path).to_path_buf())
            .map_err(|e| format!("Failed to canonicalize: {}", e))?
    } else if let Some(parent) = full_path.parent() {
        if parent.exists() {
            let canonical_parent = parent
                .canonicalize()
                .map(|path| dunce::simplified(&path).to_path_buf())
                .map_err(|e| format!("Failed to canonicalize parent: {}", e))?;
            canonical_parent.join(full_path.file_name().unwrap())
        } else {
            full_path.clone()
        }
    } else {
        full_path.clone()
    };

    check_if_its_inside_a_workspace_or_config(gcx.clone(), &canonical).await?;

    let privacy_settings = gcx.privacy_settings.read().unwrap().clone();
    if check_file_privacy(
        privacy_settings,
        &canonical,
        &FilePrivacyLevel::AllowToSendAnywhere,
    )
    .is_err()
    {
        return Err(format!(
            "Cannot access {:?} (blocked by privacy)",
            canonical
        ));
    }

    Ok(ResolvedToolPath {
        path: canonical,
        warnings: Vec::new(),
    })
}

enum OverlayState {
    Present(String),
    Deleted,
}

pub async fn tool_apply_patch_exec(
    gcx: Arc<GlobalContext>,
    args: &HashMap<String, Value>,
    dry: bool,
    execution_scope: Option<&ExecutionScope>,
    scope_guard_context: Option<&Arc<AMutex<AtCommandsContext>>>,
) -> Result<ApplyPatchResult, String> {
    let plan = plan_patch(gcx.clone(), args, execution_scope, scope_guard_context).await?;
    if !dry {
        super::patch_transaction::apply(gcx, &plan).await?;
    }
    Ok(plan)
}

async fn plan_patch(
    gcx: Arc<GlobalContext>,
    args: &HashMap<String, Value>,
    execution_scope: Option<&ExecutionScope>,
    scope_guard_context: Option<&Arc<AMutex<AtCommandsContext>>>,
) -> Result<ApplyPatchResult, String> {
    let parsed = parse_patch_arg(args)?;

    if let Some(ccx) = scope_guard_context {
        for operation in &parsed.operations {
            let (path, destination) = match operation {
                FileOperation::Add { path, .. } | FileOperation::Delete { path } => (path, None),
                FileOperation::Update { path, move_to, .. } => (path, move_to.as_ref()),
            };
            for path in std::iter::once(path).chain(destination) {
                let resolved =
                    resolve_patch_path(gcx.clone(), path, false, execution_scope).await?;
                check_scope_guard(ccx, &resolved.path).await?;
            }
        }
    }

    let mut file_results = Vec::new();
    let mut all_chunks = Vec::new();
    let mut warnings = Vec::new();
    let mut overlay: HashMap<PathBuf, OverlayState> = HashMap::new();

    for op in parsed.operations {
        match op {
            FileOperation::Add { path, contents } => {
                let resolved_path =
                    resolve_patch_path(gcx.clone(), &path, false, execution_scope).await?;
                warnings.extend(resolved_path.warnings);
                let full_path = resolved_path.path;
                if let Some(ccx) = scope_guard_context {
                    check_scope_guard(ccx, &full_path).await?;
                }

                let exists = match overlay.get(&full_path) {
                    Some(OverlayState::Present(_)) => true,
                    Some(OverlayState::Deleted) => false,
                    None => full_path.exists(),
                };
                if exists {
                    return Err(format!("File already exists: {:?}", full_path));
                }

                overlay.insert(full_path.clone(), OverlayState::Present(contents.clone()));

                let chunks =
                    convert_edit_to_diffchunks(full_path.clone(), &String::new(), &contents)?;
                all_chunks.extend(chunks.clone());
                file_results.push(SingleFileResult {
                    path: full_path,
                    action: "add",
                    before: String::new(),
                    after: contents,
                    chunks,
                });
            }

            FileOperation::Delete { path } => {
                let resolved_path =
                    resolve_patch_path(gcx.clone(), &path, false, execution_scope).await?;
                warnings.extend(resolved_path.warnings);
                let full_path = resolved_path.path;
                if let Some(ccx) = scope_guard_context {
                    check_scope_guard(ccx, &full_path).await?;
                }

                let file_content = match overlay.get(&full_path) {
                    Some(OverlayState::Present(content)) => content.clone(),
                    Some(OverlayState::Deleted) => {
                        return Err(format!("File was already deleted: {:?}", full_path));
                    }
                    None => {
                        if !full_path.exists() {
                            return Err(format!("File not found: {:?}", full_path));
                        }
                        get_file_text_from_memory_or_disk(gcx.clone(), &full_path).await?
                    }
                };

                overlay.insert(full_path.clone(), OverlayState::Deleted);

                let chunk = DiffChunk {
                    file_name: full_path.to_string_lossy().to_string(),
                    file_action: "remove".to_string(),
                    line1: 1,
                    line2: file_content.lines().count(),
                    lines_remove: file_content.clone(),
                    lines_add: String::new(),
                    lines_before: None,
                    lines_after: None,
                    file_name_rename: None,
                    is_file: true,
                    application_details: String::new(),
                };

                all_chunks.push(chunk.clone());
                file_results.push(SingleFileResult {
                    path: full_path,
                    action: "delete",
                    before: file_content,
                    after: String::new(),
                    chunks: vec![chunk],
                });
            }

            FileOperation::Update {
                path,
                move_to,
                chunks,
            } => {
                let resolved_path =
                    resolve_patch_path(gcx.clone(), &path, false, execution_scope).await?;
                warnings.extend(resolved_path.warnings);
                let full_path = resolved_path.path;
                if let Some(ccx) = scope_guard_context {
                    check_scope_guard(ccx, &full_path).await?;
                }

                let file_content = match overlay.get(&full_path) {
                    Some(OverlayState::Present(content)) => content.clone(),
                    Some(OverlayState::Deleted) => {
                        return Err(format!("File was deleted: {:?}", full_path));
                    }
                    None => {
                        if !full_path.exists() {
                            return Err(format!("File not found: {:?}", full_path));
                        }
                        get_file_text_from_memory_or_disk(gcx.clone(), &full_path).await?
                    }
                };

                let has_crlf = file_content.contains("\r\n");
                let normalized = normalize_line_endings(&file_content);
                let new_content = apply_update_chunks(&normalized, &chunks).map_err(|error| {
                    format!("Failed to update '{}': {}", full_path.display(), error)
                })?;
                let new_file_content = restore_line_endings(&new_content, has_crlf);

                if let Some(move_path) = move_to {
                    let resolved_dest =
                        resolve_patch_path(gcx.clone(), &move_path, false, execution_scope).await?;
                    warnings.extend(resolved_dest.warnings);
                    let dest_path = resolved_dest.path;
                    if let Some(ccx) = scope_guard_context {
                        check_scope_guard(ccx, &dest_path).await?;
                    }

                    let dest_exists = match overlay.get(&dest_path) {
                        Some(OverlayState::Present(_)) => true,
                        Some(OverlayState::Deleted) => false,
                        None => dest_path.exists(),
                    };
                    if dest_exists {
                        return Err(format!("Move destination exists: {:?}", dest_path));
                    }

                    overlay.insert(full_path.clone(), OverlayState::Deleted);
                    overlay.insert(
                        dest_path.clone(),
                        OverlayState::Present(new_file_content.clone()),
                    );

                    let chunk = DiffChunk {
                        file_name: full_path.to_string_lossy().to_string(),
                        file_action: "rename".to_string(),
                        line1: 1,
                        line2: file_content.lines().count(),
                        lines_remove: file_content.clone(),
                        lines_add: new_file_content.clone(),
                        lines_before: None,
                        lines_after: None,
                        file_name_rename: Some(dest_path.to_string_lossy().to_string()),
                        is_file: true,
                        application_details: String::new(),
                    };
                    all_chunks.push(chunk.clone());
                    file_results.push(SingleFileResult {
                        path: dest_path,
                        action: "rename",
                        before: file_content,
                        after: new_file_content,
                        chunks: vec![chunk],
                    });
                } else {
                    overlay.insert(
                        full_path.clone(),
                        OverlayState::Present(new_file_content.clone()),
                    );

                    let diff_chunks = convert_edit_to_diffchunks(
                        full_path.clone(),
                        &file_content,
                        &new_file_content,
                    )?;
                    all_chunks.extend(diff_chunks.clone());
                    file_results.push(SingleFileResult {
                        path: full_path,
                        action: "update",
                        before: file_content,
                        after: new_file_content,
                        chunks: diff_chunks,
                    });
                }
            }
        }
    }

    Ok(ApplyPatchResult {
        file_results,
        all_chunks,
        warnings,
    })
}

#[async_trait]
impl Tool for ToolApplyPatch {
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

        let result = tool_apply_patch_exec(
            gcx.clone(),
            args,
            false,
            execution_scope.as_ref(),
            Some(&ccx),
        )
        .await?;

        let related_section = {
            let idx_arc = { gcx.knowledge_index.clone() };
            let idx_guard = idx_arc.lock().await;
            let mut files: Vec<String> = result
                .file_results
                .iter()
                .map(|r| r.path.to_string_lossy().to_string())
                .collect();
            files.sort();
            files.dedup();
            let mut cards = idx_guard.related_for_files(&files, 8);
            if cards.is_empty() {
                cards = idx_guard.related_for_related_files(&files, 8);
            }
            format_related_memories_section(&cards, &memory_reader)
        };

        let mut snapshots: std::collections::BTreeMap<
            String,
            (Option<String>, Option<String>, String),
        > = std::collections::BTreeMap::new();
        for file in &result.file_results {
            if file.action == "rename" {
                if let Some(source) = file.chunks.first().map(|chunk| chunk.file_name.clone()) {
                    let entry = snapshots
                        .entry(source)
                        .or_insert_with(|| (Some(file.before.clone()), None, "moved from".into()));
                    entry.1 = None;
                    entry.2 = format!("moved to {}", file.path.display());
                }
            }
            let path = file.path.to_string_lossy().to_string();
            let entry = snapshots.entry(path).or_insert_with(|| {
                (
                    if file.action == "add" || file.action == "rename" {
                        None
                    } else {
                        Some(file.before.clone())
                    },
                    None,
                    file.action.into(),
                )
            });
            entry.1 = if file.action == "delete" {
                None
            } else {
                Some(file.after.clone())
            };
        }
        let mut model_content = String::new();
        for (path, (before, after, action)) in snapshots {
            if before.is_none() && after.is_none() {
                model_content.push_str(&format!(
                    "{action}: {path}\nTransient path; absent before and after this patch.\n\n"
                ));
                continue;
            }
            let net_chunks = convert_edit_to_diffchunks(
                PathBuf::from(&path),
                &before.clone().unwrap_or_default(),
                &after.clone().unwrap_or_default(),
            )?;
            model_content.push_str(&super::edit_output::edit_model_content(
                &path,
                &action,
                before.as_deref(),
                after.as_deref(),
                &net_chunks,
            ));
            model_content.push('\n');
        }

        let mut out = vec![ContextEnum::ChatMessage(ChatMessage {
            role: "diff".to_string(),
            content: ChatContent::SimpleText(json!(result.all_chunks).to_string()),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            extra: serde_json::Map::from_iter([(
                "diff_state".to_string(),
                Value::String("applied".to_string()),
            )]),
            ..Default::default()
        })];

        if !result.warnings.is_empty() {
            out.push(ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(result.warnings.join("\n")),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            }));
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

        super::edit_output::finish_edit_output(&mut out, model_content);
        Ok((false, out))
    }

    async fn match_against_confirm_deny(
        &self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        args: &HashMap<String, Value>,
    ) -> Result<MatchConfirmDeny, String> {
        let msgs_len = ccx.lock().await.messages.len();
        let can_parse = parse_patch_arg(args).is_ok();

        if msgs_len != 0 && !can_parse {
            return Ok(MatchConfirmDeny {
                result: MatchConfirmDenyResult::PASS,
                command: "apply_patch".to_string(),
                rule: "".to_string(),
            });
        }

        Ok(MatchConfirmDeny {
            result: MatchConfirmDenyResult::CONFIRMATION,
            command: "apply_patch".to_string(),
            rule: "default".to_string(),
        })
    }

    async fn command_to_match_against_confirm_deny(
        &self,
        _ccx: Arc<AMutex<AtCommandsContext>>,
        _args: &HashMap<String, Value>,
    ) -> Result<String, String> {
        Ok("apply_patch".to_string())
    }

    fn confirm_deny_rules(&self) -> Option<IntegrationConfirmation> {
        Some(IntegrationConfirmation {
            ask_user: vec!["apply_patch*".to_string()],
            deny: vec![],
        })
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "apply_patch".to_string(),
            display_name: "Apply Patch".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: APPLY_PATCH_DESCRIPTION.to_string(),
            input_schema: json_schema_from_params(
                &[("patch", "string", APPLY_PATCH_PARAM_DESCRIPTION)],
                &["patch"],
            ),
            output_schema: None,
            annotations: None,
        }
    }
}

const APPLY_PATCH_DESCRIPTION: &str = r#"Apply file operations using the patch format.

The patch format is a file-oriented diff format:

*** Begin Patch
[ one or more file sections ]
*** End Patch

File operations:
- *** Add File: <path> - create new file (lines start with +)
- *** Delete File: <path> - remove existing file
- *** Update File: <path> - modify file in place
  - Optional: *** Move to: <new_path> - rename after update

Update hunks use @@ for context hints:
@@ class BaseClass
@@ def method():
 context line (space prefix)
-old line (minus prefix)
+new line (plus prefix)

Rules:
- Show 3 lines of context above/below each change
- Use @@ to narrow scope when needed for uniqueness
- Multiple @@ can be chained for precision
- Use *** End of File for appending at end
- Hunks may be out of source order: each matches the original file snapshot
- Each hunk's target must be unique after its ordered anchors, including normalized matches; ambiguous or overlapping targets are rejected
- Pure insertions without @@ use the previous hunk's original end (or the file start); consecutive insertions retain patch order
- *** End of File on a replacement requires the expected lines to be a file suffix

Multiple file sections are applied one file at a time and are NOT atomic: there
is no journal, so a failure part-way leaves the earlier files already changed.
Planning failures change no files. Apply-time errors report observed changed,
unchanged, and unknown paths; a failed operation may already have changed disk.
Hard-linked files are written in place and do not have atomic-replacement guarantees."#;

const APPLY_PATCH_PARAM_DESCRIPTION: &str = r#"The patch content in envelope format:
*** Begin Patch
*** Add File: path/to/new.txt
+content line 1
+content line 2
*** Update File: path/to/existing.txt
@@ function_name
 context
-old
+new
*** Delete File: path/to/remove.txt
*** End Patch"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn invalid_later_operation_leaves_all_earlier_files_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().into()];
        let path = dir.path().join("existing.txt");
        std::fs::write(&path, "before\n").unwrap();
        let args = HashMap::from([("patch".into(), json!("*** Begin Patch\n*** Update File: existing.txt\n@@\n-before\n+after\n*** Delete File: missing.txt\n*** End Patch"))]);
        assert!(tool_apply_patch_exec(gcx, &args, false, None, None)
            .await
            .is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "before\n");
    }

    #[tokio::test]
    async fn patch_plan_rechecks_every_preimage_under_the_complete_lock_set() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().into()];
        for file in ["a", "b"] {
            std::fs::write(dir.path().join(file), "before\n").unwrap();
        }
        let args = HashMap::from([("patch".into(), json!("*** Begin Patch\n*** Update File: a\n@@\n-before\n+after\n*** Update File: b\n@@\n-before\n+after\n*** End Patch"))]);
        let plan = plan_patch(gcx.clone(), &args, None, None).await.unwrap();
        std::fs::write(dir.path().join("b"), "external\n").unwrap();
        assert!(super::super::patch_transaction::apply(gcx, &plan)
            .await
            .is_err());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a")).unwrap(),
            "before\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("b")).unwrap(),
            "external\n"
        );
    }

    #[tokio::test]
    #[serial_test::serial(durable_fault)]
    async fn a_multi_file_patch_failing_midway_reports_exactly_which_files_changed() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().into()];
        for file in ["a", "b"] {
            std::fs::write(dir.path().join(file), "before\n").unwrap();
        }
        let args = HashMap::from([("patch".into(), json!("*** Begin Patch\n*** Update File: a\n@@\n-before\n+after\n*** Update File: b\n@@\n-before\n+after\n*** End Patch"))]);
        let plan = plan_patch(gcx.clone(), &args, None, None).await.unwrap();
        let armed = crate::file_activity::durable_fault::arm_for(
            crate::file_activity::durable_fault::AFTER_TEMP_WRITE,
            plan.file_results[1].path.to_string_lossy().to_string(),
        );

        let error = super::super::patch_transaction::apply(gcx, &plan)
            .await
            .unwrap_err();
        drop(armed);

        assert!(error.contains("not atomic"), "{error}");
        assert!(error.contains("Changed: ["), "{error}");
        assert!(error.contains("Unchanged: ["), "{error}");
        let changed = error.split("Changed: [").nth(1).unwrap();
        let (changed, rest) = changed.split_once(']').unwrap();
        let unchanged = rest.split("Unchanged: [").nth(1).unwrap();
        let (unchanged, _) = unchanged.split_once(']').unwrap();
        assert!(changed.ends_with("a"), "changed list was {changed:?}");
        assert!(unchanged.ends_with("b"), "unchanged list was {unchanged:?}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join("a")).unwrap(),
            "after\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("b")).unwrap(),
            "before\n",
            "the file that failed was left truncated instead of fully old"
        );
        let leftovers = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains(".durable-tmp"))
            .count();
        assert_eq!(leftovers, 0, "temp files were left behind");
    }

    #[tokio::test]
    #[serial_test::serial(durable_fault)]
    async fn failed_post_rename_patch_reports_the_failed_file_as_changed() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().into()];
        let path = dir.path().join("file");
        std::fs::write(&path, "before\n").unwrap();
        let args = HashMap::from([(
            "patch".into(),
            json!("*** Begin Patch\n*** Update File: file\n@@\n-before\n+after\n*** End Patch"),
        )]);
        let plan = plan_patch(gcx.clone(), &args, None, None).await.unwrap();
        let armed = crate::file_activity::durable_fault::arm_for(
            crate::file_activity::durable_fault::AFTER_RENAME,
            path.to_string_lossy().to_string(),
        );
        let error = super::super::patch_transaction::apply(gcx, &plan)
            .await
            .unwrap_err();
        drop(armed);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "after\n");
        assert!(
            error.contains(&format!("Changed: [{}]", path.display())),
            "{error}"
        );
        assert!(error.contains("Unchanged: []"), "{error}");
        assert!(error.contains("durability is unconfirmed"), "{error}");
    }

    #[tokio::test]
    async fn patch_overlay_supports_create_update_move_and_delete_in_one_request() {
        let dir = tempfile::tempdir().unwrap();
        let gcx = crate::global_context::tests::make_test_gcx().await;
        *gcx.documents_state.workspace_folders.lock().unwrap() = vec![dir.path().into()];
        let args = HashMap::from([("patch".into(), json!("*** Begin Patch\n*** Add File: a\n+one\n*** Update File: a\n*** Move to: b\n@@\n-one\n+two\n*** Delete File: b\n*** End Patch"))]);
        let result = tool_apply_patch_exec(gcx, &args, false, None, None)
            .await
            .unwrap();
        assert_eq!(result.file_results.len(), 3);
        assert!(!dir.path().join("a").exists());
        assert!(!dir.path().join("b").exists());
    }
}
