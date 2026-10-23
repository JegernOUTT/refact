use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_commands::at_file::{file_repair_candidates, return_one_candidate_or_a_good_error};
use crate::call_validation::DiffChunk;
use crate::files_correction::{
    canonicalize_normalized_path, check_if_its_inside_a_workspace_or_config,
    correct_to_nearest_dir_path, get_unscoped_project_dirs, preprocess_path_for_normalization,
    registered_worktree_path_mappings, resolve_codegraph_queue_path,
};
use crate::files_in_workspace::{get_file_text_from_memory_or_disk, remove_memory_document_for_path};
use crate::global_context::GlobalContext;
use crate::privacy::{check_file_privacy, FilePrivacyLevel, PrivacySettings};
use crate::worktrees::scope::ExecutionScope;
use regex::{Match, Regex};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex as AMutex;
use tracing::warn;

pub use refact_file_edit_core::text_edit::{
    edit_result_summary, find_match_lines, insert_at_anchor, normalize_line_endings,
    parse_bool_arg, parse_line_ranges, parse_string_arg, replace_between_anchors,
    restore_line_endings, strip_line_number_prefixes, AnchorMode, LineRange,
};
pub use refact_scope_utils::{
    append_scope_warnings, scope_warnings_to_tool_message, scoped_path_warnings,
};

#[derive(Debug, Clone)]
pub struct ResolvedToolPath {
    pub path: PathBuf,
    pub warnings: Vec<String>,
}

pub fn active_execution_scope(scope: Option<&ExecutionScope>) -> Option<&ExecutionScope> {
    scope.filter(|scope| scope.is_enforced())
}

pub fn resolve_path_with_scope(
    raw_path: &Path,
    privacy_settings: Arc<PrivacySettings>,
    execution_scope: Option<&ExecutionScope>,
    require_existing: bool,
) -> Option<Result<ResolvedToolPath, String>> {
    if let Some(scope) = execution_scope {
        if let Err(error) = scope.ensure_active_root() {
            return Some(Err(error));
        }
    }
    let scope = active_execution_scope(execution_scope)?;
    let scoped = if require_existing {
        scope.resolve_existing_path(raw_path)
    } else {
        scope.resolve_creatable_path(raw_path)
    };
    let scoped = match scoped {
        Ok(scoped) => scoped,
        Err(e) => {
            return Some(Err(format!(
                "⚠️ Cannot resolve '{}' in active worktree '{}': {}",
                raw_path.display(),
                scope.effective_root().display(),
                e
            )))
        }
    };
    if let Err(e) = check_file_privacy(
        privacy_settings,
        &scoped.path,
        &FilePrivacyLevel::AllowToSendAnywhere,
    ) {
        return Some(Err(format!(
            "⚠️ Cannot access '{}' (blocked by privacy: {}). Active worktree root: '{}'",
            scoped.path.display(),
            e,
            scope.effective_root().display()
        )));
    }
    let warnings = scoped_path_warnings(&scoped, scope);
    Some(Ok(ResolvedToolPath {
        path: scoped.path,
        warnings,
    }))
}

pub async fn check_scope_guard(
    ccx: &Arc<AMutex<AtCommandsContext>>,
    edit_path: &Path,
) -> Result<(), String> {
    use crate::agents::path_scope::{self, ScopeOutcome};

    let (app, task_id, card_id, root) = {
        let ccx_lock = ccx.lock().await;
        ccx_lock.check_native_edit_scope(edit_path)?;
        let meta = match ccx_lock.task_meta.as_ref() {
            Some(meta) if meta.role == "agents" => meta.clone(),
            _ => return Ok(()),
        };
        let card_id = match meta.card_id.as_ref() {
            Some(card_id) => card_id.clone(),
            None => return Ok(()),
        };
        let root = active_execution_scope(ccx_lock.execution_scope.as_ref())
            .map(|scope| scope.effective_root().to_path_buf())
            .or_else(|| {
                ccx_lock
                    .app
                    .gcx
                    .documents_state
                    .workspace_folders
                    .lock()
                    .unwrap()
                    .first()
                    .cloned()
            });
        (ccx_lock.app.clone(), meta.task_id, card_id, root)
    };

    let board = crate::tasks::storage::load_board(app.gcx.clone(), &task_id).await?;
    let card = match board.get_card(&card_id) {
        Some(card) => card,
        None => return Ok(()),
    };

    let limits = crate::file_activity::settings();
    let declared = path_scope::scope_for_mode(card.scope_guard_mode, &card.target_files, &limits);
    let effective = app
        .agent_executions
        .narrow_path_scope(&format!("{task_id}/{card_id}"), declared);
    let root = match root {
        Some(root) => root,
        None if effective.is_enforced() => {
            return Err(
                "an enforced path scope needs an active project or worktree root, and none is available"
                    .into(),
            )
        }
        None => return Ok(()),
    };

    match path_scope::decide(card.scope_guard_mode, &effective, &root, edit_path, &limits)? {
        ScopeOutcome::Allowed => Ok(()),
        ScopeOutcome::Warned { path, .. } => {
            tracing::warn!(
                "Scope warning: edited {} not in target_files for card {}",
                path,
                card_id
            );
            Ok(())
        }
        ScopeOutcome::Refused { path, scope } => {
            Err(path_scope::refusal_message(&path, &scope, effective.strict))
        }
    }
}

pub async fn parse_path_for_update(
    gcx: Arc<GlobalContext>,
    args: &HashMap<String, Value>,
    privacy_settings: Arc<PrivacySettings>,
    execution_scope: Option<&ExecutionScope>,
) -> Result<ResolvedToolPath, String> {
    let s = parse_string_arg(args, "path", "Provide absolute path to file")?;
    let raw_path = preprocess_path_for_normalization(s.trim().to_string());
    if let Some(resolved) = resolve_path_with_scope(
        Path::new(&raw_path),
        privacy_settings.clone(),
        execution_scope,
        true,
    ) {
        return resolved;
    }

    let candidates = file_repair_candidates(gcx.clone(), &raw_path, 3, false).await;
    let path = return_one_candidate_or_a_good_error(
        gcx.clone(),
        &raw_path,
        &candidates,
        &get_unscoped_project_dirs(gcx.clone()).await,
        false,
    )
    .await
    .map(|f| canonicalize_normalized_path(PathBuf::from(f)))?;

    if check_file_privacy(
        privacy_settings,
        &path,
        &FilePrivacyLevel::AllowToSendAnywhere,
    )
    .is_err()
    {
        return Err(format!(
            "⚠️ Cannot update {:?} (blocked by privacy). 💡 Choose file in allowed directory",
            path
        ));
    }
    if !path.exists() {
        return Err(format!(
            "⚠️ File {:?} not found. 💡 Use create_textdoc() for new files",
            path
        ));
    }
    Ok(ResolvedToolPath {
        path,
        warnings: Vec::new(),
    })
}

pub async fn parse_path_for_create(
    gcx: Arc<GlobalContext>,
    args: &HashMap<String, Value>,
    privacy_settings: Arc<PrivacySettings>,
    execution_scope: Option<&ExecutionScope>,
) -> Result<ResolvedToolPath, String> {
    let s = parse_string_arg(args, "path", "Provide absolute path for new file")?;
    let raw_string = preprocess_path_for_normalization(s.trim().to_string());
    let raw_path = PathBuf::from(&raw_string);

    raw_path.file_name().ok_or_else(|| {
        format!(
            "⚠️ Path '{}' has no filename. 💡 Include filename: /path/to/file.ext",
            s.trim()
        )
    })?;

    if let Some(resolved) =
        resolve_path_with_scope(&raw_path, privacy_settings.clone(), execution_scope, false)
    {
        return resolved;
    }

    let filename = raw_path
        .file_name()
        .ok_or_else(|| {
            format!(
                "⚠️ Path '{}' has no filename. 💡 Include filename: /path/to/file.ext",
                s.trim()
            )
        })?
        .to_string_lossy()
        .to_string();

    let path = if !raw_path.is_absolute() {
        if let Some(parent) = raw_path.parent().filter(|p| !p.as_os_str().is_empty()) {
            let parent_str = parent.to_string_lossy().to_string();
            let candidates = correct_to_nearest_dir_path(gcx.clone(), &parent_str, false, 3).await;
            let parent_dir = return_one_candidate_or_a_good_error(
                gcx.clone(),
                &parent_str,
                &candidates,
                &get_unscoped_project_dirs(gcx.clone()).await,
                true,
            )
            .await?;
            canonicalize_normalized_path(PathBuf::from(parent_dir).join(&filename))
        } else {
            return Err(format!(
                "⚠️ Path '{}' is not absolute. 💡 Use full path like /project/src/file.ext",
                s.trim()
            ));
        }
    } else {
        let path = canonicalize_normalized_path(raw_path);
        check_if_its_inside_a_workspace_or_config(gcx.clone(), &path).await?;
        path
    };

    if check_file_privacy(
        privacy_settings,
        &path,
        &FilePrivacyLevel::AllowToSendAnywhere,
    )
    .is_err()
    {
        return Err(format!(
            "⚠️ Cannot create {:?} (blocked by privacy). 💡 Choose path in allowed directory",
            path
        ));
    }
    Ok(ResolvedToolPath {
        path,
        warnings: Vec::new(),
    })
}

pub fn convert_edit_to_diffchunks(
    path: PathBuf,
    before: &String,
    after: &String,
) -> Result<Vec<DiffChunk>, String> {
    let before_lines: Vec<&str> = before.lines().collect();
    let after_lines: Vec<&str> = after.lines().collect();
    let diffs = diff::lines(before, after);
    let mut line_num = 0;
    let mut old_line_num = 0;
    let mut new_line_num = 0;
    let mut current_chunk_lines_remove = Vec::new();
    let mut current_chunk_lines_add = Vec::new();
    let mut current_chunk_line_nums = Vec::new();
    let mut current_chunk_old_line_nums = Vec::new();
    let mut current_chunk_new_line_nums = Vec::new();
    let mut current_chunk_is_plus = Vec::new();
    let mut diff_chunks = Vec::new();

    let context_line = |lines: &[&str], index: usize| -> Option<String> {
        lines.get(index).map(|line| format!("{}\n", line))
    };

    let flush_changes = |lines_remove: &Vec<String>,
                         lines_add: &Vec<String>,
                         line_nums: &Vec<usize>,
                         old_line_nums: &Vec<usize>,
                         new_line_nums: &Vec<usize>,
                         is_plus: &Vec<bool>|
     -> Option<DiffChunk> {
        if lines_remove.is_empty() && lines_add.is_empty() {
            return None;
        }

        let lines_remove = lines_remove.join("");
        let lines_add = lines_add.join("");

        let min_line_index = line_nums.iter().min().copied().unwrap_or(0);
        let line1 = min_line_index + 1;

        let max_old_line_index = old_line_nums.iter().max().copied();
        let max_new_line_index = new_line_nums.iter().max().copied();

        let line2 = line_nums
            .iter()
            .zip(is_plus.iter())
            .map(|(&num, &is_plus)| if is_plus { num + 1 } else { num + 2 })
            .max()
            .unwrap_or(1);

        let lines_before = old_line_nums
            .iter()
            .min()
            .or_else(|| new_line_nums.iter().min())
            .and_then(|index| index.checked_sub(1))
            .and_then(|index| context_line(&before_lines, index));
        let after_context_index = max_new_line_index
            .map(|index| index + 1)
            .or_else(|| max_old_line_index.map(|index| index));
        let lines_after = after_context_index.and_then(|index| context_line(&after_lines, index));

        Some(DiffChunk {
            file_name: path.to_string_lossy().to_string(),
            file_name_rename: None,
            file_action: "edit".to_string(),
            line1,
            line2,
            lines_remove,
            lines_add,
            lines_before,
            lines_after,
            ..Default::default()
        })
    };

    for diff in diffs {
        match diff {
            diff::Result::Left(l) => {
                current_chunk_lines_remove.push(format!("{}\n", l));
                current_chunk_line_nums.push(line_num);
                current_chunk_old_line_nums.push(old_line_num);
                current_chunk_is_plus.push(false);
                old_line_num += 1;
                line_num += 1;
            }
            diff::Result::Right(r) => {
                current_chunk_lines_add.push(format!("{}\n", r));
                current_chunk_line_nums.push(line_num);
                current_chunk_new_line_nums.push(new_line_num);
                current_chunk_is_plus.push(true);
                new_line_num += 1;
            }
            diff::Result::Both(_, _) => {
                if let Some(chunk) = flush_changes(
                    &current_chunk_lines_remove,
                    &current_chunk_lines_add,
                    &current_chunk_line_nums,
                    &current_chunk_old_line_nums,
                    &current_chunk_new_line_nums,
                    &current_chunk_is_plus,
                ) {
                    diff_chunks.push(chunk);
                }
                current_chunk_lines_remove.clear();
                current_chunk_lines_add.clear();
                current_chunk_line_nums.clear();
                current_chunk_old_line_nums.clear();
                current_chunk_new_line_nums.clear();
                current_chunk_is_plus.clear();
                old_line_num += 1;
                new_line_num += 1;
                line_num += 1;
            }
        }
    }

    if let Some(chunk) = flush_changes(
        &current_chunk_lines_remove,
        &current_chunk_lines_add,
        &current_chunk_line_nums,
        &current_chunk_old_line_nums,
        &current_chunk_new_line_nums,
        &current_chunk_is_plus,
    ) {
        diff_chunks.push(chunk);
    }

    Ok(diff_chunks)
}

pub async fn fast_enqueue_for_edit(
    gcx: Arc<GlobalContext>,
    docs: &[PathBuf],
) -> Result<(), String> {
    if docs.is_empty() {
        return Ok(());
    }
    let codegraph_mb = gcx.codegraph.lock().await.clone();
    if let Some(service) = codegraph_mb {
        let worktree_mappings = registered_worktree_path_mappings(gcx.cache_dir.as_path());
        let paths: Vec<_> = docs
            .iter()
            .map(|doc| resolve_codegraph_queue_path(doc, &worktree_mappings))
            .collect();
        service.enqueue_paths_with_read_paths(&paths);
    }
    Ok(())
}

pub async fn write_file(
    gcx: Arc<GlobalContext>,
    path: &PathBuf,
    file_text: &String,
    dry: bool,
    expected_preimage: Option<&str>,
) -> Result<(String, String), String> {
    let intent = if expected_preimage.is_some() {
        crate::file_activity::WriteIntent::Contextual
    } else {
        crate::file_activity::WriteIntent::Overwrite { authorized: false }
    };
    write_file_with_intent(gcx, path, file_text, dry, expected_preimage, intent).await
}

pub async fn write_file_with_intent(
    gcx: Arc<GlobalContext>,
    path: &PathBuf,
    file_text: &String,
    dry: bool,
    expected_preimage: Option<&str>,
    intent: crate::file_activity::WriteIntent,
) -> Result<(String, String), String> {
    use crate::tools::file_edit::undo_history::record_edit;

    let parent = path.parent().ok_or(format!(
        "Failed to Add: {:?}. Path is invalid.\nReason: path must have had a parent directory",
        path
    ))?;

    if !parent.exists() {
        if !dry {
            tokio::fs::create_dir_all(&parent).await.map_err(|e| {
                let err = format!("Failed to Add: {:?}; Its parent dir {:?} did not exist and attempt to create it failed.\nERROR: {}", path, parent, e);
                warn!("{err}");
                err
            })?;
        }
    }

    let write_guard = if dry {
        None
    } else {
        let actor = crate::file_activity::current_actor();
        Some(
            crate::file_activity::begin_write(
                actor.as_deref(),
                path,
                if actor.is_some() || matches!(intent, crate::file_activity::WriteIntent::Create) {
                    intent
                } else {
                    crate::file_activity::WriteIntent::Contextual
                },
                "file_edit",
            )
            .await?,
        )
    };
    let disk_preimage = if dry {
        None
    } else {
        match tokio::fs::read_to_string(path).await {
            Ok(text) => Some(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(format!("Failed to inspect file: {:?}\nERROR: {}", path, e)),
        }
    };
    let before_text = if path.exists() {
        get_file_text_from_memory_or_disk(gcx.clone(), path).await?
    } else {
        "".to_string()
    };

    if !dry {
        let disk_text = disk_preimage.as_deref().unwrap_or("");
        if before_text != disk_text
            || expected_preimage.is_some_and(|expected| {
                normalize_line_endings(disk_text) != normalize_line_endings(expected)
            })
        {
            return Err(format!(
                "⚠️ {:?} was modified since last read or its buffer differs from disk. 💡 Use cat() to re-read the file and retry",
                path
            ));
        }
    }

    if let Some(expected) = expected_preimage {
        if normalize_line_endings(&before_text) != normalize_line_endings(expected) {
            return Err(format!(
                "⚠️ {:?} was modified since last read. 💡 Use cat() to re-read the file and retry",
                path
            ));
        }
    }

    if let Some(write_guard) = write_guard {
        let mode = if disk_preimage.is_some() {
            crate::file_activity::ReplaceMode::Replace
        } else {
            crate::file_activity::ReplaceMode::CreateNew
        };
        crate::file_activity::write_file_durably(path, file_text.as_bytes(), mode)
            .await
            .map_err(|e| {
                let err = format!("Failed to write file: {:?}\nERROR: {}", path, e);
                warn!("{err}");
                err
            })?;
        record_edit(path, disk_preimage.as_deref(), Some(file_text));
        remove_memory_document_for_path(gcx.clone(), path).await;
        write_guard.commit(
            if disk_preimage.is_some() {
                crate::file_activity::WriteKind::Modified
            } else {
                crate::file_activity::WriteKind::Created
            },
            Some(file_text),
        );
    }

    Ok((before_text, file_text.to_string()))
}

pub async fn str_replace(
    gcx: Arc<GlobalContext>,
    path: &PathBuf,
    old_str: &String,
    new_str: &String,
    replace_multiple: bool,
    dry: bool,
) -> Result<(String, String), String> {
    if old_str.is_empty() {
        return Err("⚠️ old_str cannot be empty. 💡 Provide the exact text to replace".to_string());
    }
    let file_content = get_file_text_from_memory_or_disk(gcx.clone(), path).await?;
    let has_crlf = file_content.contains("\r\n");

    let normalized_content = normalize_line_endings(&file_content);
    let normalized_old_str = strip_line_number_prefixes(&normalize_line_endings(old_str));
    if normalized_old_str.is_empty() {
        return Err("⚠️ old_str is empty after stripping line-number prefixes. 💡 Provide actual source content, not just line numbers".to_string());
    }

    let occurrences = normalized_content.matches(&normalized_old_str).count();
    if occurrences == 0 {
        let trimmed_old = normalized_old_str.trim();
        let trimmed_match = normalized_content.contains(trimmed_old) && !trimmed_old.is_empty();
        let hint = if trimmed_match {
            "Whitespace mismatch detected. 💡 Check leading/trailing spaces, or use update_textdoc_anchored()"
        } else {
            "💡 Use cat() to verify content, or try update_textdoc_anchored() with shorter anchors"
        };
        return Err(format!("⚠️ old_str not found in {:?}. {}", path, hint));
    }
    if !replace_multiple && occurrences > 1 {
        let lines = find_match_lines(&normalized_content, &normalized_old_str);
        return Err(format!(
            "⚠️ {} occurrences at lines {:?}. 💡 Add surrounding context to make unique, or set multiple:true",
            occurrences, lines
        ));
    }

    let normalized_new_str = normalize_line_endings(new_str);
    let new_content = if replace_multiple {
        normalized_content.replace(&normalized_old_str, &normalized_new_str)
    } else {
        normalized_content.replacen(&normalized_old_str, &normalized_new_str, 1)
    };
    let new_file_content = restore_line_endings(&new_content, has_crlf);
    write_file(
        gcx.clone(),
        path,
        &new_file_content,
        dry,
        Some(&file_content),
    )
    .await?;
    Ok((file_content, new_file_content))
}

pub async fn str_replace_anchored(
    gcx: Arc<GlobalContext>,
    path: &PathBuf,
    mode: AnchorMode,
    anchor1: &str,
    anchor2: Option<&str>,
    content: &str,
    multiple: bool,
    dry: bool,
) -> Result<(String, String), String> {
    if anchor1.is_empty() {
        return Err(
            "⚠️ Anchor cannot be empty. 💡 Provide unique text to locate edit position".to_string(),
        );
    }
    let file_content = get_file_text_from_memory_or_disk(gcx.clone(), path).await?;
    let has_crlf = file_content.contains("\r\n");

    let normalized = normalize_line_endings(&file_content);
    let anchor1_n = normalize_line_endings(anchor1);
    let content_n = normalize_line_endings(content);

    let result = match mode {
        AnchorMode::ReplaceBetween => {
            let anchor2_str = anchor2.ok_or("⚠️ anchor_after required for replace_between mode")?;
            if anchor2_str.is_empty() {
                return Err("⚠️ anchor_after cannot be empty".to_string());
            }
            let anchor2_n = normalize_line_endings(anchor2_str);
            replace_between_anchors(&normalized, &anchor1_n, &anchor2_n, &content_n, multiple)?
        }
        AnchorMode::InsertAfter => {
            insert_at_anchor(&normalized, &anchor1_n, &content_n, multiple, true)?
        }
        AnchorMode::InsertBefore => {
            insert_at_anchor(&normalized, &anchor1_n, &content_n, multiple, false)?
        }
    };

    let new_file_content = restore_line_endings(&result, has_crlf);
    write_file(
        gcx.clone(),
        path,
        &new_file_content,
        dry,
        Some(&file_content),
    )
    .await?;
    Ok((file_content, new_file_content))
}

pub async fn str_replace_lines(
    gcx: Arc<GlobalContext>,
    path: &PathBuf,
    new_content: &String,
    ranges_str: &str,
    dry: bool,
) -> Result<(String, String), String> {
    let file_content = get_file_text_from_memory_or_disk(gcx.clone(), path).await?;
    let has_crlf = file_content.contains("\r\n");

    let normalized_content = normalize_line_endings(&file_content);
    let mut lines: Vec<String> = normalized_content.lines().map(|s| s.to_string()).collect();
    let total_lines = lines.len();

    let ranges = parse_line_ranges(ranges_str, total_lines)?;
    let normalized_new_content = normalize_line_endings(new_content);

    if ranges.len() == 1 {
        let range = &ranges[0];
        if range.end > total_lines && !(total_lines == 0 && range.start == 1) {
            return Err(format!(
                "⚠️ Range end {} exceeds file length ({} lines). 💡 Use cat() to check file, or ':' for end",
                range.end, total_lines
            ));
        }
        let start_idx = range.start - 1;
        let end_idx = range.end.min(total_lines);
        let new_lines: Vec<String> = normalized_new_content
            .lines()
            .map(|s| s.to_string())
            .collect();
        lines.splice(start_idx..end_idx, new_lines);
    } else {
        let content_parts: Vec<&str> = normalized_new_content
            .split("---RANGE_SEPARATOR---")
            .collect();

        if content_parts.len() != ranges.len() {
            return Err(format!(
                "⚠️ {} content parts but {} ranges. 💡 Separate content with '---RANGE_SEPARATOR---'",
                content_parts.len(), ranges.len()
            ));
        }

        let mut indexed: Vec<(usize, LineRange)> = ranges.into_iter().enumerate().collect();
        indexed.sort_by(|a, b| b.1.start.cmp(&a.1.start));

        for (orig_idx, range) in indexed {
            if range.end > lines.len() {
                return Err(format!(
                    "⚠️ Range {}:{} exceeds current length ({} lines). 💡 Check ranges",
                    range.start,
                    range.end,
                    lines.len()
                ));
            }
            let start_idx = range.start - 1;
            let end_idx = range.end;
            let new_lines: Vec<String> = content_parts[orig_idx]
                .lines()
                .map(|s| s.to_string())
                .collect();
            lines.splice(start_idx..end_idx, new_lines);
        }
    }

    let new_content_joined = lines.join("\n");
    let new_file_content = if normalized_content.ends_with('\n') {
        restore_line_endings(&format!("{}\n", new_content_joined), has_crlf)
    } else {
        restore_line_endings(&new_content_joined, has_crlf)
    };

    write_file(
        gcx.clone(),
        path,
        &new_file_content,
        dry,
        Some(&file_content),
    )
    .await?;
    Ok((file_content, new_file_content))
}

pub async fn str_replace_regex(
    gcx: Arc<GlobalContext>,
    path: &PathBuf,
    pattern: &Regex,
    replacement: &String,
    multiple: bool,
    expected_matches: Option<usize>,
    literal_replacement: bool,
    dry: bool,
) -> Result<(String, String), String> {
    let file_content = get_file_text_from_memory_or_disk(gcx.clone(), path).await?;
    let has_crlf = file_content.contains("\r\n");

    let normalized_content = normalize_line_endings(&file_content);
    let normalized_replacement = normalize_line_endings(replacement);
    let matches: Vec<Match> = pattern.find_iter(&normalized_content).collect();
    let occurrences = matches.len();

    if occurrences == 0 {
        return Err(format!(
            "⚠️ Pattern not found in {:?}. 💡 Use cat() to check content, try update_textdoc_anchored()",
            path
        ));
    }
    if let Some(expected) = expected_matches {
        if occurrences != expected {
            return Err(format!(
                "⚠️ Expected {} matches, found {}. 💡 Adjust pattern or expected_matches",
                expected, occurrences
            ));
        }
    }
    if !multiple && occurrences > 1 {
        let lines: Vec<usize> = matches
            .iter()
            .map(|m| normalized_content[..m.start()].lines().count() + 1)
            .collect();
        return Err(format!(
            "⚠️ {} matches at lines {:?}. 💡 Make pattern more specific, or set multiple:true",
            occurrences, lines
        ));
    }

    let new_content = if literal_replacement {
        let rep = regex::NoExpand(normalized_replacement.as_str());
        if multiple {
            pattern.replace_all(&normalized_content, rep).to_string()
        } else {
            pattern.replace(&normalized_content, rep).to_string()
        }
    } else if multiple {
        pattern
            .replace_all(&normalized_content, normalized_replacement.as_str())
            .to_string()
    } else {
        pattern
            .replace(&normalized_content, normalized_replacement.as_str())
            .to_string()
    };
    let new_file_content = restore_line_endings(&new_content, has_crlf);
    write_file(
        gcx.clone(),
        path,
        &new_file_content,
        dry,
        Some(&file_content),
    )
    .await?;
    Ok((file_content, new_file_content))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subprocess_stale_edit() {
        let Ok(root) = std::env::var("REFACT_TEST_FILE_RACE_ROOT") else {
            return;
        };
        let id = std::env::var("REFACT_TEST_FILE_RACE_ID").unwrap();
        let root = PathBuf::from(root);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let gcx = crate::global_context::tests::make_test_gcx().await;
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![root.clone()];
            let path = root.join("contended.txt");
            std::fs::write(root.join(format!("ready-{id}")), "ready").unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while !root.join("go").exists() {
                assert!(std::time::Instant::now() < deadline);
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            let result = write_file(gcx, &path, &format!("edit-{id}"), false, Some("before")).await;
            std::fs::write(
                root.join(format!("result-{id}")),
                match result {
                    Ok(_) => "won".to_string(),
                    Err(error) => format!("refused: {error}"),
                },
            )
            .unwrap();
        });
    }

    #[test]
    fn cooperating_processes_cannot_both_apply_one_stale_edit() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("contended.txt");
        std::fs::write(&path, "before").unwrap();
        let locks = root.path().join("locks");
        let spawn = |id: &str| {
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "tools::file_edit::auxiliary::tests::subprocess_stale_edit",
                    "--nocapture",
                ])
                .env("REFACT_TEST_FILE_RACE_ROOT", root.path())
                .env("REFACT_TEST_FILE_RACE_ID", id)
                .env("REFACT_FILE_ACTIVITY_LOCK_DIR", &locks)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap()
        };
        let mut a = spawn("a");
        let mut b = spawn("b");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !root.path().join("ready-a").exists() || !root.path().join("ready-b").exists() {
            if std::time::Instant::now() >= deadline {
                let _ = a.kill();
                let _ = b.kill();
                let _ = a.wait();
                let _ = b.wait();
                panic!("file edit subprocesses did not reach barrier");
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        std::fs::write(root.path().join("go"), "go").unwrap();
        assert!(a.wait().unwrap().success());
        assert!(b.wait().unwrap().success());
        let a = std::fs::read_to_string(root.path().join("result-a")).unwrap();
        let b = std::fs::read_to_string(root.path().join("result-b")).unwrap();
        let (winner, refusal) = match (a.as_str(), b.as_str()) {
            ("won", refusal) => ("a", refusal.to_string()),
            (refusal, "won") => ("b", refusal.to_string()),
            _ => panic!("expected exactly one winner: {a} / {b}"),
        };
        assert!(refusal.contains("modified since last read"), "{refusal}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("edit-{winner}")
        );
        assert!(std::fs::read_dir(&locks).unwrap().next().is_some());
    }

    #[tokio::test]
    async fn write_file_serializes_cooperative_writers() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("contended.txt");
        std::fs::write(&path, "before").unwrap();
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let writer = |after: String| {
            let gcx = gcx.clone();
            let path = path.clone();
            let barrier = barrier.clone();
            async move {
                barrier.wait().await;
                write_file(gcx, &path, &after, false, Some("before")).await
            }
        };
        let (first, second) = tokio::join!(writer("first".into()), writer("second".into()));
        assert_ne!(first.is_ok(), second.is_ok());
        let (success, failure) = if first.is_ok() {
            (first.unwrap(), second.unwrap_err())
        } else {
            (second.unwrap(), first.unwrap_err())
        };
        assert!(failure.contains("modified since last read"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), success.1);
        let observations = crate::file_activity::observations(Some(&path.to_string_lossy()));
        assert_eq!(observations.writes.len(), 1);
        assert_eq!(
            observations.writes[0].kind,
            crate::file_activity::WriteKind::Modified
        );
    }

    #[tokio::test]
    async fn write_file_rechecks_preimage_after_waiting_for_guard() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("locked.txt");
        std::fs::write(&path, "before").unwrap();
        let guard = crate::file_activity::begin_write(
            None,
            &path,
            crate::file_activity::WriteIntent::Contextual,
            "test_writer",
        )
        .await
        .unwrap();
        let after = "after".to_string();
        let pending = write_file(gcx, &path, &after, false, Some("before"));
        tokio::pin!(pending);
        assert!(futures::poll!(&mut pending).is_pending());
        std::fs::write(&path, "other").unwrap();
        guard.commit(crate::file_activity::WriteKind::Modified, Some("other"));
        assert!(pending
            .await
            .unwrap_err()
            .contains("modified since last read"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "other");
        let observations = crate::file_activity::observations(Some(&path.to_string_lossy()));
        assert_eq!(observations.writes.len(), 1);
        assert_eq!(observations.writes[0].tool, "test_writer");
    }

    #[tokio::test]
    async fn write_file_rejects_buffer_disk_divergence() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("buffer.txt");
        std::fs::write(&path, "disk").unwrap();
        let key = crate::files_correction::canonical_path(path.to_string_lossy());
        gcx.documents_state.memory_document_map.lock().await.insert(
            key.clone(),
            Arc::new(tokio::sync::RwLock::new(
                crate::files_in_workspace::Document {
                    doc_path: key,
                    doc_text: Some(ropey::Rope::from_str("buffer")),
                },
            )),
        );
        let error = write_file(gcx, &path, &"after".into(), false, Some("buffer"))
            .await
            .unwrap_err();
        assert!(error.contains("buffer differs from disk"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "disk");
        assert!(
            crate::file_activity::observations(Some(&path.to_string_lossy()))
                .writes
                .is_empty()
        );
    }

    #[tokio::test]
    async fn write_file_failed_operation_is_not_recorded() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("directory");
        std::fs::create_dir(&path).unwrap();
        assert!(write_file(gcx, &path, &"after".into(), false, None)
            .await
            .is_err());
        assert!(path.is_dir());
        assert!(
            crate::file_activity::observations(Some(&path.to_string_lossy()))
                .writes
                .is_empty()
        );
    }

    #[tokio::test]
    async fn write_file_dry_does_not_create_or_record() {
        let gcx = crate::global_context::tests::make_test_gcx().await;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("missing").join("new.txt");
        let result = write_file(gcx.clone(), &path, &"after".into(), true, None)
            .await
            .unwrap();
        assert_eq!(result, (String::new(), "after".into()));
        assert!(!path.parent().unwrap().exists());
        assert!(
            crate::file_activity::observations(Some(&path.to_string_lossy()))
                .writes
                .is_empty()
        );
        write_file(gcx, &path, &"after".into(), false, None)
            .await
            .unwrap();
        let observations = crate::file_activity::observations(Some(&path.to_string_lossy()));
        assert_eq!(observations.writes.len(), 1);
        assert_eq!(
            observations.writes[0].kind,
            crate::file_activity::WriteKind::Created
        );
    }

    #[test]
    fn test_convert_edit_to_diffchunks_add() {
        let before = "";
        let after = "line1\nline2\n";
        let chunks = convert_edit_to_diffchunks(
            PathBuf::from("test.txt"),
            &before.to_string(),
            &after.to_string(),
        )
        .unwrap();
        assert!(!chunks.is_empty());
    }

    #[test]
    fn test_convert_edit_to_diffchunks_modify() {
        let before = "line1\nold\nline3\n";
        let after = "line1\nnew\nline3\n";
        let chunks = convert_edit_to_diffchunks(
            PathBuf::from("test.txt"),
            &before.to_string(),
            &after.to_string(),
        )
        .unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].lines_remove.contains("old"));
        assert!(chunks[0].lines_add.contains("new"));
        assert_eq!(chunks[0].lines_before.as_deref(), Some("line1\n"));
        assert_eq!(chunks[0].lines_after.as_deref(), Some("line3\n"));
    }

    #[test]
    fn test_convert_edit_to_diffchunks_context_boundaries_and_insertions() {
        let before = "first\nanchor\nlast\n";
        let after = "first\nanchor\ninserted\nlast\n";
        let chunks = convert_edit_to_diffchunks(
            PathBuf::from("test.txt"),
            &before.to_string(),
            &after.to_string(),
        )
        .unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].lines_before.as_deref(), Some("anchor\n"));
        assert_eq!(chunks[0].lines_after.as_deref(), Some("last\n"));

        let before = "old\nnext\n";
        let after = "new\nnext\n";
        let chunks = convert_edit_to_diffchunks(
            PathBuf::from("test.txt"),
            &before.to_string(),
            &after.to_string(),
        )
        .unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].lines_before, None);
        assert_eq!(chunks[0].lines_after.as_deref(), Some("next\n"));
    }

    mod worktree_scope_tools {
        use crate::at_commands::at_commands::AtCommandsContext;
        use crate::call_validation::{ChatContent, ContextEnum};
        use crate::chat::types::TaskMeta;
        use crate::global_context::GlobalContext;
        use crate::privacy::{FilePrivacySettings, PrivacySettings};
        use crate::tasks::types::{BoardCard, ScopeGuardMode, TaskBoard};
        use crate::tools::file_edit::auxiliary::{check_scope_guard, fast_enqueue_for_edit};
        use crate::tools::file_edit::tool_apply_patch::tool_apply_patch_exec;
        use crate::tools::file_edit::tool_create_textdoc::tool_create_text_doc_exec;
        use crate::tools::file_edit::tool_undo_textdoc::{tool_undo_text_doc_exec, ToolUndoTextDoc};
        use crate::tools::file_edit::tool_update_textdoc::tool_update_text_doc_exec;
        use crate::tools::file_edit::tool_update_textdoc_anchored::tool_update_text_doc_anchored_exec;
        use crate::tools::file_edit::tool_update_textdoc_by_lines::tool_update_text_doc_by_lines_exec;
        use crate::tools::file_edit::tool_update_textdoc_regex::tool_update_text_doc_regex_exec;
        use crate::tools::tool_mv::ToolMv;
        use crate::tools::tool_rm::ToolRm;
        use crate::tools::tool_shell::ToolShell;
        use crate::tools::tools_description::Tool;
        use crate::worktrees::scope::ExecutionScope;
        use crate::worktrees::types::{WorktreeMeta, WorktreeRegistry, WorktreeRegistryRecord};
        use serde_json::{json, Value};
        use std::collections::HashMap;
        use std::fs;
        use std::path::{Path, PathBuf};
        use std::sync::Arc;
        use std::time::Duration;
        use tokio::sync::Mutex as AMutex;

        struct Fixture {
            _temp: tempfile::TempDir,
            root: PathBuf,
            source: PathBuf,
            outside: PathBuf,
            scope: ExecutionScope,
            worktree: WorktreeMeta,
            gcx: Arc<GlobalContext>,
        }

        fn now_plus_hour() -> u64 {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 3600
        }

        async fn set_privacy(gcx: Arc<GlobalContext>, blocked: Vec<String>) {
            *gcx.privacy_settings.write().unwrap() = Arc::new(PrivacySettings {
                privacy_rules: FilePrivacySettings {
                    only_send_to_servers_I_control: Vec::new(),
                    blocked,
                },
                loaded_ts: now_plus_hour(),
            });
        }

        async fn fixture() -> Fixture {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("worktree");
            let source = temp.path().join("source");
            let outside = temp.path().join("outside");
            fs::create_dir_all(root.join("src")).unwrap();
            fs::create_dir_all(source.join("src")).unwrap();
            fs::create_dir_all(&outside).unwrap();
            let root = dunce::simplified(&fs::canonicalize(root).unwrap()).to_path_buf();
            let source = dunce::simplified(&fs::canonicalize(source).unwrap()).to_path_buf();
            let outside = dunce::simplified(&fs::canonicalize(outside).unwrap()).to_path_buf();
            let worktree = WorktreeMeta {
                id: "wt-tools".to_string(),
                kind: "task_agent".to_string(),
                root: root.clone(),
                source_workspace_root: source.clone(),
                repo_root: source.clone(),
                branch: Some("feature".to_string()),
                base_branch: Some("main".to_string()),
                base_commit: Some("base".to_string()),
                task_id: Some("task".to_string()),
                card_id: Some("card".to_string()),
                agent_id: Some("agent".to_string()),
                enforce: true,
            };
            let scope = ExecutionScope::from_worktree(&worktree);
            let gcx = crate::global_context::tests::make_test_gcx().await;
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![source.clone()];
            set_privacy(gcx.clone(), Vec::new()).await;
            Fixture {
                _temp: temp,
                root,
                source,
                outside,
                scope,
                worktree,
                gcx,
            }
        }

        fn path_value(path: &Path) -> Value {
            json!(path.to_string_lossy().to_string())
        }

        fn args(entries: Vec<(&str, Value)>) -> HashMap<String, Value> {
            entries
                .into_iter()
                .map(|(key, value)| (key.to_string(), value))
                .collect()
        }

        async fn ccx(f: &Fixture) -> Arc<AMutex<AtCommandsContext>> {
            Arc::new(AMutex::new(
                AtCommandsContext::new_from_app(
                    crate::app_state::AppState::from_gcx(f.gcx.clone()).await,
                    4096,
                    20,
                    false,
                    Vec::new(),
                    "chat".to_string(),
                    None,
                    "model".to_string(),
                    None,
                    Some(f.worktree.clone()),
                )
                .await,
            ))
        }

        fn tool_text(messages: &[ContextEnum]) -> String {
            messages
                .iter()
                .filter_map(|message| match message {
                    ContextEnum::ChatMessage(message) => match &message.content {
                        ChatContent::SimpleText(text) => Some(text.clone()),
                        _ => None,
                    },
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
        }

        fn pwd_command() -> String {
            if cfg!(target_os = "windows") {
                "Get-Location | Select-Object -ExpandProperty Path".to_string()
            } else {
                "pwd".to_string()
            }
        }

        fn write_worktree_registry(cache_dir: &Path, source: &Path, worktree: &WorktreeMeta) {
            let source = dunce::simplified(&fs::canonicalize(source).unwrap()).to_path_buf();
            let hash = refact_worktrees::service::project_hash_for_path(&source);
            let registry_dir = cache_dir.join("worktrees").join(&hash);
            fs::create_dir_all(&registry_dir).unwrap();
            let registry = WorktreeRegistry {
                schema_version: 1,
                source_workspace_root: source,
                project_hash: hash,
                records: vec![WorktreeRegistryRecord {
                    meta: worktree.clone(),
                    created_at: "2026-01-01T00:00:00Z".to_string(),
                    updated_at: "2026-01-01T00:00:00Z".to_string(),
                    last_seen_at: None,
                    references: Vec::new(),
                    last_known_status: None,
                }],
            };
            fs::write(
                registry_dir.join("index.json"),
                serde_json::to_string_pretty(&registry).unwrap(),
            )
            .unwrap();
        }

        fn scope_guard_card(mode: ScopeGuardMode) -> BoardCard {
            BoardCard {
                id: "card".to_string(),
                title: "Card".to_string(),
                column: "doing".to_string(),
                priority: "P1".to_string(),
                depends_on: vec![],
                instructions: String::new(),
                assignee: None,
                agent_chat_id: Some("chat".to_string()),
                status_updates: vec![],
                comments: vec![],
                final_report: None,
                final_report_structured: None,
                verifier_report: None,
                created_at: "2026-05-22T00:00:00Z".to_string(),
                started_at: None,
                last_heartbeat_at: None,
                paused_at: None,
                paused_by: None,
                completed_at: None,
                agent_branch: None,
                agent_worktree: None,
                agent_worktree_name: None,
                base_branch: None,
                base_commit: None,
                ab_variants: None,
                team_members: vec![],
                target_files: vec!["src/allowed.rs".to_string()],
                scope_guard_mode: mode,
                attention: None,
                deliverable: None,
                verification: Vec::new(),
            }
        }

        async fn scope_guard_ccx(
            f: &Fixture,
            mode: ScopeGuardMode,
            target_files: Vec<String>,
            role: &str,
        ) -> Arc<AMutex<AtCommandsContext>> {
            let mut card = scope_guard_card(mode);
            card.target_files = target_files;
            let board = TaskBoard {
                cards: vec![card],
                ..TaskBoard::default()
            };
            let task_dir = f.source.join(".refact").join("tasks").join("task");
            tokio::fs::create_dir_all(&task_dir).await.unwrap();
            tokio::fs::write(
                task_dir.join("board.yaml"),
                serde_yaml::to_string(&board).unwrap(),
            )
            .await
            .unwrap();
            Arc::new(AMutex::new(
                AtCommandsContext::new_from_app(
                    crate::app_state::AppState::from_gcx(f.gcx.clone()).await,
                    4096,
                    20,
                    false,
                    Vec::new(),
                    "chat".to_string(),
                    None,
                    "model".to_string(),
                    Some(TaskMeta {
                        task_id: "task".to_string(),
                        role: role.to_string(),
                        agent_id: Some("agent".to_string()),
                        card_id: Some("card".to_string()),
                        planner_chat_id: None,
                    }),
                    Some(f.worktree.clone()),
                )
                .await,
            ))
        }

        #[tokio::test]
        async fn worktree_scope_tools_edit_helpers_modify_scoped_files() {
            let f = fixture().await;

            fs::write(f.root.join("src/update.txt"), "old\n").unwrap();
            fs::write(f.source.join("src/update.txt"), "source\n").unwrap();
            fs::write(f.outside.join("update.txt"), "sibling\n").unwrap();
            let update_args = args(vec![
                ("path", json!("src/update.txt")),
                ("old_str", json!("old")),
                ("replacement", json!("new")),
            ]);
            tool_update_text_doc_exec(f.gcx.clone(), &update_args, false, Some(&f.scope), None)
                .await
                .unwrap();
            assert_eq!(
                fs::read_to_string(f.root.join("src/update.txt")).unwrap(),
                "new\n"
            );
            assert_eq!(
                fs::read_to_string(f.source.join("src/update.txt")).unwrap(),
                "source\n"
            );
            assert_eq!(
                fs::read_to_string(f.outside.join("update.txt")).unwrap(),
                "sibling\n"
            );
            let update_source_args = args(vec![
                ("path", path_value(&f.source.join("src/update.txt"))),
                ("old_str", json!("new")),
                ("replacement", json!("mapped")),
            ]);

            let (_, _, _, summary) = tool_update_text_doc_exec(
                f.gcx.clone(),
                &update_source_args,
                false,
                Some(&f.scope),
                None,
            )
            .await
            .unwrap();
            assert!(summary.contains("mapped to active worktree"));
            assert_eq!(
                fs::read_to_string(f.root.join("src/update.txt")).unwrap(),
                "mapped\n"
            );
            assert_eq!(
                fs::read_to_string(f.source.join("src/update.txt")).unwrap(),
                "source\n"
            );

            let create_args = args(vec![
                ("path", json!("src/create_relative.txt")),
                ("content", json!("created")),
            ]);

            tool_create_text_doc_exec(f.gcx.clone(), &create_args, false, Some(&f.scope), None)
                .await
                .unwrap();
            assert_eq!(
                fs::read_to_string(f.root.join("src/create_relative.txt")).unwrap(),
                "created\n"
            );
            let nested_create_args = args(vec![
                ("path", json!("new_dir/deep/file.rs")),
                ("content", json!("nested")),
            ]);

            tool_create_text_doc_exec(
                f.gcx.clone(),
                &nested_create_args,
                false,
                Some(&f.scope),
                None,
            )
            .await
            .unwrap();
            assert_eq!(
                fs::read_to_string(f.root.join("new_dir/deep/file.rs")).unwrap(),
                "nested\n"
            );
            let escaped_create_args = args(vec![
                ("path", json!("../escaped/file.rs")),
                ("content", json!("escaped")),
            ]);

            assert!(tool_create_text_doc_exec(
                f.gcx.clone(),
                &escaped_create_args,
                false,
                Some(&f.scope),
                None,
            )
            .await
            .is_err());
            let create_source_args = args(vec![
                ("path", path_value(&f.source.join("src/create_source.txt"))),
                ("content", json!("created source")),
            ]);

            let (_, _, _, summary) = tool_create_text_doc_exec(
                f.gcx.clone(),
                &create_source_args,
                false,
                Some(&f.scope),
                None,
            )
            .await
            .unwrap();
            assert!(summary.contains("mapped to active worktree"));
            assert_eq!(
                fs::read_to_string(f.root.join("src/create_source.txt")).unwrap(),
                "created source\n"
            );
            assert!(!f.source.join("src/create_source.txt").exists());

            fs::write(f.root.join("src/anchored.txt"), "A\nanchor\nB\n").unwrap();
            fs::write(f.source.join("src/anchored.txt"), "source\n").unwrap();
            let anchored_args = args(vec![
                ("path", json!("src/anchored.txt")),
                ("mode", json!("insert_after")),
                ("anchor", json!("anchor")),
                ("content", json!("\nrelative")),
            ]);

            tool_update_text_doc_anchored_exec(
                f.gcx.clone(),
                &anchored_args,
                false,
                Some(&f.scope),
                None,
            )
            .await
            .unwrap();
            let anchored_source_args = args(vec![
                ("path", path_value(&f.source.join("src/anchored.txt"))),
                ("mode", json!("insert_before")),
                ("anchor", json!("anchor")),
                ("content", json!("source\n")),
            ]);

            let (_, _, _, summary) = tool_update_text_doc_anchored_exec(
                f.gcx.clone(),
                &anchored_source_args,
                false,
                Some(&f.scope),
                None,
            )
            .await
            .unwrap();
            assert!(summary.contains("mapped to active worktree"));
            assert!(fs::read_to_string(f.root.join("src/anchored.txt"))
                .unwrap()
                .contains("source\nanchor\nrelative"));
            assert_eq!(
                fs::read_to_string(f.source.join("src/anchored.txt")).unwrap(),
                "source\n"
            );

            fs::write(f.root.join("src/regex.txt"), "alpha\n").unwrap();
            fs::write(f.source.join("src/regex.txt"), "source\n").unwrap();
            let regex_args = args(vec![
                ("path", json!("src/regex.txt")),
                ("pattern", json!("alpha")),
                ("replacement", json!("beta")),
            ]);

            tool_update_text_doc_regex_exec(
                f.gcx.clone(),
                &regex_args,
                false,
                Some(&f.scope),
                None,
            )
            .await
            .unwrap();
            let regex_source_args = args(vec![
                ("path", path_value(&f.source.join("src/regex.txt"))),
                ("pattern", json!("beta")),
                ("replacement", json!("gamma")),
            ]);

            let (_, _, _, summary) = tool_update_text_doc_regex_exec(
                f.gcx.clone(),
                &regex_source_args,
                false,
                Some(&f.scope),
                None,
            )
            .await
            .unwrap();
            assert!(summary.contains("mapped to active worktree"));
            assert_eq!(
                fs::read_to_string(f.root.join("src/regex.txt")).unwrap(),
                "gamma\n"
            );
            assert_eq!(
                fs::read_to_string(f.source.join("src/regex.txt")).unwrap(),
                "source\n"
            );

            fs::write(f.root.join("src/lines.txt"), "one\ntwo\n").unwrap();
            fs::write(f.source.join("src/lines.txt"), "source\n").unwrap();
            let lines_args = args(vec![
                ("path", json!("src/lines.txt")),
                ("content", json!("TWO")),
                ("ranges", json!("2")),
            ]);

            tool_update_text_doc_by_lines_exec(
                f.gcx.clone(),
                &lines_args,
                false,
                Some(&f.scope),
                None,
            )
            .await
            .unwrap();
            let lines_source_args = args(vec![
                ("path", path_value(&f.source.join("src/lines.txt"))),
                ("content", json!("ONE")),
                ("ranges", json!("1")),
            ]);

            let (_, _, _, summary) = tool_update_text_doc_by_lines_exec(
                f.gcx.clone(),
                &lines_source_args,
                false,
                Some(&f.scope),
                None,
            )
            .await
            .unwrap();
            assert!(summary.contains("mapped to active worktree"));
            assert_eq!(
                fs::read_to_string(f.root.join("src/lines.txt")).unwrap(),
                "ONE\nTWO\n"
            );
            assert_eq!(
                fs::read_to_string(f.source.join("src/lines.txt")).unwrap(),
                "source\n"
            );

            fs::write(f.root.join("src/undo.txt"), "base\n").unwrap();
            fs::write(f.source.join("src/undo.txt"), "source\n").unwrap();
            let undo_update_args = args(vec![
                ("path", json!("src/undo.txt")),
                ("old_str", json!("base")),
                ("replacement", json!("changed")),
            ]);

            tool_update_text_doc_exec(
                f.gcx.clone(),
                &undo_update_args,
                false,
                Some(&f.scope),
                None,
            )
            .await
            .unwrap();
            let undo_args = args(vec![("path", path_value(&f.source.join("src/undo.txt")))]);
            let (_, _, _, summary) =
                tool_undo_text_doc_exec(f.gcx.clone(), &undo_args, Some(&f.scope))
                    .await
                    .unwrap();
            assert!(summary.contains("mapped to active worktree"));
            assert_eq!(
                fs::read_to_string(f.root.join("src/undo.txt")).unwrap(),
                "base\n"
            );
            assert_eq!(
                fs::read_to_string(f.source.join("src/undo.txt")).unwrap(),
                "source\n"
            );
        }

        #[tokio::test]
        async fn every_text_mutation_family_rejects_a_stale_model_visible_read() {
            let f = fixture().await;
            let path = f.root.join("src/stale.txt");
            let actor = format!("mutation-{}", uuid::Uuid::new_v4());
            let old = "head\nanchor\nbody\nend\n";
            let new = "head\nanchor\nbody\nexternal\n";
            fs::write(&path, old).unwrap();
            crate::file_activity::record_read(Some(&actor), &path, old, 1, 4, 4, None);
            fs::write(&path, new).unwrap();
            let requests = [
                args(vec![
                    ("path", json!("src/stale.txt")),
                    ("content", json!("replacement")),
                ]),
                args(vec![
                    ("path", json!("src/stale.txt")),
                    ("old_str", json!("body")),
                    ("replacement", json!("replacement")),
                ]),
                args(vec![
                    ("path", json!("src/stale.txt")),
                    ("pattern", json!("body")),
                    ("replacement", json!("replacement")),
                ]),
                args(vec![
                    ("path", json!("src/stale.txt")),
                    ("mode", json!("insert_after")),
                    ("anchor", json!("anchor")),
                    ("content", json!("replacement")),
                ]),
                args(vec![
                    ("path", json!("src/stale.txt")),
                    ("ranges", json!("3")),
                    ("content", json!("replacement")),
                ]),
            ];
            for (kind, request) in requests.into_iter().enumerate() {
                let result = crate::file_activity::TOOL_ACTOR
                    .scope(actor.clone(), async {
                        match kind {
                            0 => {
                                tool_create_text_doc_exec(
                                    f.gcx.clone(),
                                    &request,
                                    false,
                                    Some(&f.scope),
                                    None,
                                )
                                .await
                            }
                            1 => {
                                tool_update_text_doc_exec(
                                    f.gcx.clone(),
                                    &request,
                                    false,
                                    Some(&f.scope),
                                    None,
                                )
                                .await
                            }
                            2 => {
                                tool_update_text_doc_regex_exec(
                                    f.gcx.clone(),
                                    &request,
                                    false,
                                    Some(&f.scope),
                                    None,
                                )
                                .await
                            }
                            3 => {
                                tool_update_text_doc_anchored_exec(
                                    f.gcx.clone(),
                                    &request,
                                    false,
                                    Some(&f.scope),
                                    None,
                                )
                                .await
                            }
                            _ => {
                                tool_update_text_doc_by_lines_exec(
                                    f.gcx.clone(),
                                    &request,
                                    false,
                                    Some(&f.scope),
                                    None,
                                )
                                .await
                            }
                        }
                    })
                    .await;
                assert!(result.is_err(), "mutation kind {kind} accepted stale read");
                assert_eq!(fs::read_to_string(&path).unwrap(), new);
            }
            let patch = args(vec![("patch", json!("*** Begin Patch\n*** Update File: src/stale.txt\n@@\n-body\n+replacement\n*** End Patch"))]);
            assert!(crate::file_activity::TOOL_ACTOR
                .scope(
                    actor.clone(),
                    tool_apply_patch_exec(f.gcx.clone(), &patch, false, Some(&f.scope), None)
                )
                .await
                .is_err());
            let context = ccx(&f).await;
            context.lock().await.chat_id = actor;
            let mut remove = ToolRm {
                config_path: String::new(),
            };
            assert!(remove
                .tool_execute(
                    context.clone(),
                    &"rm".into(),
                    &args(vec![("path", json!("src/stale.txt"))])
                )
                .await
                .is_err());
            let mut mover = ToolMv {
                config_path: String::new(),
            };
            assert!(mover
                .tool_execute(
                    context,
                    &"mv".into(),
                    &args(vec![
                        ("source", json!("src/stale.txt")),
                        ("destination", json!("src/destination.txt"))
                    ])
                )
                .await
                .is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), new);
            assert!(!f.root.join("src/destination.txt").exists());
        }

        #[tokio::test]
        async fn whole_replacement_requires_a_complete_visible_read_and_new_file_race_has_one_winner(
        ) {
            let f = fixture().await;
            let path = f.root.join("src/whole.txt");
            fs::write(&path, "one\ntwo\n").unwrap();
            let actor = format!("partial-{}", uuid::Uuid::new_v4());
            crate::file_activity::record_read(
                Some(&actor),
                &path,
                "one\ntwo\n",
                1,
                1,
                2,
                Some("partial".into()),
            );
            let request = args(vec![
                ("path", json!("src/whole.txt")),
                ("content", json!("new")),
            ]);
            assert!(crate::file_activity::TOOL_ACTOR
                .scope(
                    actor.clone(),
                    tool_create_text_doc_exec(f.gcx.clone(), &request, false, Some(&f.scope), None)
                )
                .await
                .is_err());
            crate::file_activity::record_read(Some(&actor), &path, "one\ntwo\n", 1, 2, 2, None);
            crate::file_activity::TOOL_ACTOR
                .scope(
                    actor,
                    tool_create_text_doc_exec(f.gcx.clone(), &request, false, Some(&f.scope), None),
                )
                .await
                .unwrap();
            let path = f.root.join("src/race.txt");
            let barrier = Arc::new(tokio::sync::Barrier::new(2));
            let write = |text: &'static str| {
                let barrier = barrier.clone();
                let path = path.clone();
                let gcx = f.gcx.clone();
                async move {
                    barrier.wait().await;
                    crate::tools::file_edit::auxiliary::write_file_with_intent(
                        gcx,
                        &path,
                        &text.into(),
                        false,
                        None,
                        crate::file_activity::WriteIntent::Create,
                    )
                    .await
                }
            };
            let (one, two) = tokio::join!(write("one"), write("two"));
            assert_ne!(one.is_ok(), two.is_ok());
        }

        #[tokio::test]
        async fn edit_tools_enqueue_without_waiting_for_initial_codegraph_index() {
            let f = fixture().await;
            let service = Arc::new(
                refact_codegraph::CodeGraphService::open(f._temp.path().join("codegraph.sqlite"))
                    .unwrap(),
            );
            assert!(!service.is_initial_index_done());
            service.enqueue_files(&["queued.rs".to_string()]);
            *f.gcx.codegraph.lock().await = Some(service.clone());
            let create_args = args(vec![
                ("path", json!("src/fast.rs")),
                ("content", json!("fast")),
            ]);

            tokio::time::timeout(
                Duration::from_secs(2),
                tool_create_text_doc_exec(f.gcx.clone(), &create_args, false, Some(&f.scope), None),
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(
                fs::read_to_string(f.root.join("src").join("fast.rs")).unwrap(),
                "fast\n"
            );
            assert_eq!(service.queue_len(), 2);
        }

        #[tokio::test]
        async fn edit_fast_enqueue_maps_registered_worktree_path_to_source_path() {
            let gcx = crate::global_context::tests::make_test_gcx().await;
            let temp = tempfile::tempdir().unwrap();
            let source = temp.path().join("source");
            fs::create_dir_all(source.join("src")).unwrap();
            let source = dunce::simplified(&fs::canonicalize(source).unwrap()).to_path_buf();
            let hash = refact_worktrees::service::project_hash_for_path(&source);
            let worktree_root = gcx.cache_dir.join("worktrees").join(&hash).join("wt-tools");
            fs::create_dir_all(worktree_root.join("src")).unwrap();
            let worktree_root =
                dunce::simplified(&fs::canonicalize(worktree_root).unwrap()).to_path_buf();
            let source_file = source.join("src").join("fast.rs");
            let worktree_file = worktree_root.join("src").join("fast.rs");
            fs::write(&source_file, "fn source() {}\n").unwrap();
            fs::write(&worktree_file, "fn worktree() {}\n").unwrap();
            let worktree = WorktreeMeta {
                id: "wt-tools".to_string(),
                kind: "task_agent".to_string(),
                root: worktree_root,
                source_workspace_root: source.clone(),
                repo_root: source.clone(),
                branch: Some("feature".to_string()),
                base_branch: Some("main".to_string()),
                base_commit: Some("base".to_string()),
                task_id: Some("task".to_string()),
                card_id: Some("card".to_string()),
                agent_id: Some("agent".to_string()),
                enforce: true,
            };
            write_worktree_registry(&gcx.cache_dir, &source, &worktree);
            let service = Arc::new(refact_codegraph::CodeGraphService::open_in_memory().unwrap());
            *gcx.codegraph.lock().await = Some(service.clone());

            fast_enqueue_for_edit(gcx.clone(), &[worktree_file.clone()])
                .await
                .unwrap();

            assert_eq!(
                service.drain_batch(10),
                vec![source_file.to_string_lossy().to_string()]
            );
        }

        #[tokio::test]
        async fn edit_fast_enqueue_indexes_worktree_content_under_source_key() {
            let gcx = crate::global_context::tests::make_test_gcx().await;
            set_privacy(gcx.clone(), Vec::new()).await;
            let temp = tempfile::tempdir().unwrap();
            let source = temp.path().join("source");
            fs::create_dir_all(source.join("src")).unwrap();
            let source = dunce::simplified(&fs::canonicalize(source).unwrap()).to_path_buf();
            let hash = refact_worktrees::service::project_hash_for_path(&source);
            let worktree_root = gcx.cache_dir.join("worktrees").join(&hash).join("wt-tools");
            fs::create_dir_all(worktree_root.join("src")).unwrap();
            let worktree_root =
                dunce::simplified(&fs::canonicalize(worktree_root).unwrap()).to_path_buf();
            let source_file = source.join("src").join("fast.rs");
            let worktree_file = worktree_root.join("src").join("fast.rs");
            fs::write(&source_file, "fn source() {}\n").unwrap();
            fs::write(&worktree_file, "fn worktree() {}\n").unwrap();
            let worktree = WorktreeMeta {
                id: "wt-tools".to_string(),
                kind: "task_agent".to_string(),
                root: worktree_root,
                source_workspace_root: source.clone(),
                repo_root: source.clone(),
                branch: Some("feature".to_string()),
                base_branch: Some("main".to_string()),
                base_commit: Some("base".to_string()),
                task_id: Some("task".to_string()),
                card_id: Some("card".to_string()),
                agent_id: Some("agent".to_string()),
                enforce: true,
            };
            write_worktree_registry(&gcx.cache_dir, &source, &worktree);
            let service = Arc::new(refact_codegraph::CodeGraphService::open_in_memory().unwrap());
            *gcx.codegraph.lock().await = Some(service.clone());

            fast_enqueue_for_edit(gcx.clone(), &[worktree_file.clone()])
                .await
                .unwrap();
            let queued = service.drain_batch_entries(10);
            assert_eq!(
                queued,
                vec![refact_codegraph::QueuedPath::new(
                    source_file.to_string_lossy().to_string(),
                    worktree_file.to_string_lossy().to_string(),
                )]
            );
            crate::codegraph::cg_highlev::process_index_batch(gcx.clone(), service.clone(), queued)
                .await;

            assert_eq!(
                service.all_files_with_text().await.unwrap(),
                vec![(
                    source_file.to_string_lossy().to_string(),
                    "fn worktree() {}\n".to_string(),
                )]
            );
        }

        #[tokio::test]
        async fn edit_fast_enqueue_upgrades_pending_source_entry_to_worktree_read_path() {
            let gcx = crate::global_context::tests::make_test_gcx().await;
            set_privacy(gcx.clone(), Vec::new()).await;
            let temp = tempfile::tempdir().unwrap();
            let source = temp.path().join("source");
            fs::create_dir_all(source.join("src")).unwrap();
            let source = dunce::simplified(&fs::canonicalize(source).unwrap()).to_path_buf();
            let hash = refact_worktrees::service::project_hash_for_path(&source);
            let worktree_root = gcx.cache_dir.join("worktrees").join(&hash).join("wt-tools");
            fs::create_dir_all(worktree_root.join("src")).unwrap();
            let worktree_root =
                dunce::simplified(&fs::canonicalize(worktree_root).unwrap()).to_path_buf();
            let source_file = source.join("src").join("fast.rs");
            let worktree_file = worktree_root.join("src").join("fast.rs");
            fs::write(&source_file, "fn source() {}\n").unwrap();
            fs::write(&worktree_file, "fn worktree_edited() {}\n").unwrap();
            let worktree = WorktreeMeta {
                id: "wt-tools".to_string(),
                kind: "task_agent".to_string(),
                root: worktree_root,
                source_workspace_root: source.clone(),
                repo_root: source.clone(),
                branch: Some("feature".to_string()),
                base_branch: Some("main".to_string()),
                base_commit: Some("base".to_string()),
                task_id: Some("task".to_string()),
                card_id: Some("card".to_string()),
                agent_id: Some("agent".to_string()),
                enforce: true,
            };
            write_worktree_registry(&gcx.cache_dir, &source, &worktree);
            let service = Arc::new(refact_codegraph::CodeGraphService::open_in_memory().unwrap());
            *gcx.codegraph.lock().await = Some(service.clone());
            let source_key = source_file.to_string_lossy().to_string();
            let worktree_read_path = worktree_file.to_string_lossy().to_string();

            service.enqueue_files(&[source_key.clone()]);
            fast_enqueue_for_edit(gcx.clone(), &[worktree_file.clone()])
                .await
                .unwrap();

            let queued = service.drain_batch_entries(10);
            assert_eq!(
                queued,
                vec![refact_codegraph::QueuedPath::new(
                    source_key.clone(),
                    worktree_read_path,
                )]
            );
            crate::codegraph::cg_highlev::process_index_batch(gcx.clone(), service.clone(), queued)
                .await;

            assert_eq!(
                service.all_files_with_text().await.unwrap(),
                vec![(source_key, "fn worktree_edited() {}\n".to_string())]
            );
        }

        #[tokio::test]
        async fn worktree_only_create_fast_and_watcher_enqueue_share_source_key() {
            let gcx = crate::global_context::tests::make_test_gcx().await;
            let temp = tempfile::tempdir().unwrap();
            let source = temp.path().join("source");
            fs::create_dir_all(source.join("src")).unwrap();
            let source = dunce::simplified(&fs::canonicalize(source).unwrap()).to_path_buf();
            let hash = refact_worktrees::service::project_hash_for_path(&source);
            let worktree_root = gcx.cache_dir.join("worktrees").join(&hash).join("wt-tools");
            fs::create_dir_all(worktree_root.join("src")).unwrap();
            let worktree_root =
                dunce::simplified(&fs::canonicalize(worktree_root).unwrap()).to_path_buf();
            let source_file = source.join("src").join("new.rs");
            let worktree_file = worktree_root.join("src").join("new.rs");
            fs::write(&worktree_file, "fn worktree_only() {}\n").unwrap();
            let worktree = WorktreeMeta {
                id: "wt-tools".to_string(),
                kind: "task_agent".to_string(),
                root: worktree_root,
                source_workspace_root: source.clone(),
                repo_root: source.clone(),
                branch: Some("feature".to_string()),
                base_branch: Some("main".to_string()),
                base_commit: Some("base".to_string()),
                task_id: Some("task".to_string()),
                card_id: Some("card".to_string()),
                agent_id: Some("agent".to_string()),
                enforce: true,
            };
            write_worktree_registry(&gcx.cache_dir, &source, &worktree);
            *gcx.documents_state.workspace_folders.lock().unwrap() = vec![worktree.root.clone()];
            let service = Arc::new(refact_codegraph::CodeGraphService::open_in_memory().unwrap());
            *gcx.codegraph.lock().await = Some(service.clone());

            fast_enqueue_for_edit(gcx.clone(), &[worktree_file.clone()])
                .await
                .unwrap();
            crate::files_in_workspace::enqueue_some_docs_with_read_paths(
                gcx.clone(),
                &[refact_codegraph::QueuedPath::new(
                    worktree_file.to_string_lossy().to_string(),
                    worktree_file.to_string_lossy().to_string(),
                )],
                false,
            )
            .await;

            let queued = service.drain_batch_entries(10);
            assert_eq!(
                queued,
                vec![refact_codegraph::QueuedPath::new(
                    source_file.to_string_lossy().to_string(),
                    worktree_file.to_string_lossy().to_string(),
                )]
            );
            let worktree_key = worktree_file.to_string_lossy().to_string();
            assert!(!queued.iter().any(|path| path.store_path == worktree_key));
        }

        #[tokio::test]
        async fn worktree_scope_tools_absolute_warnings_and_privacy() {
            let f = fixture().await;
            fs::write(f.root.join("src/absolute.txt"), "old\n").unwrap();
            let absolute_args = args(vec![
                ("path", path_value(&f.root.join("src/absolute.txt"))),
                ("old_str", json!("old")),
                ("replacement", json!("absolute")),
            ]);
            let (_, _, _, summary) = tool_update_text_doc_exec(
                f.gcx.clone(),
                &absolute_args,
                false,
                Some(&f.scope),
                None,
            )
            .await
            .unwrap();
            assert!(summary.contains("absolute path was used"));
            assert_eq!(
                fs::read_to_string(f.root.join("src/absolute.txt")).unwrap(),
                "absolute\n"
            );

            let outside_file = f.outside.join("outside.txt");
            fs::write(&outside_file, "old\n").unwrap();
            let outside_args = args(vec![
                ("path", path_value(&outside_file)),
                ("old_str", json!("old")),
                ("replacement", json!("outside")),
            ]);

            let (_, _, _, summary) = tool_update_text_doc_exec(
                f.gcx.clone(),
                &outside_args,
                false,
                Some(&f.scope),
                None,
            )
            .await
            .unwrap();
            assert!(summary.contains("outside active worktree"));
            assert_eq!(fs::read_to_string(&outside_file).unwrap(), "outside\n");

            let blocked_file = f.outside.join("blocked.txt");
            fs::write(&blocked_file, "old\n").unwrap();
            set_privacy(
                f.gcx.clone(),
                vec![blocked_file.to_string_lossy().to_string()],
            )
            .await;
            let blocked_args = args(vec![
                ("path", path_value(&blocked_file)),
                ("old_str", json!("old")),
                ("replacement", json!("blocked")),
            ]);

            let error = tool_update_text_doc_exec(
                f.gcx.clone(),
                &blocked_args,
                false,
                Some(&f.scope),
                None,
            )
            .await
            .unwrap_err();
            assert!(error.contains("blocked by privacy"));
        }

        #[tokio::test]
        async fn worktree_scope_tools_apply_patch_resolves_all_path_kinds() {
            let f = fixture().await;
            fs::write(f.root.join("src/patch_source.txt"), "old\n").unwrap();
            fs::write(f.source.join("src/patch_source.txt"), "source\n").unwrap();
            let patch = format!(
                "*** Begin Patch\n*** Update File: {}\n@@\n-old\n+mapped\n*** End Patch",
                f.source.join("src/patch_source.txt").display()
            );

            let result = tool_apply_patch_exec(
                f.gcx.clone(),
                &args(vec![("patch", json!(patch))]),
                false,
                Some(&f.scope),
                None,
            )
            .await
            .unwrap();
            assert!(result
                .warnings
                .join("\n")
                .contains("mapped to active worktree"));
            assert_eq!(
                fs::read_to_string(f.root.join("src/patch_source.txt")).unwrap(),
                "mapped\n"
            );
            assert_eq!(
                fs::read_to_string(f.source.join("src/patch_source.txt")).unwrap(),
                "source\n"
            );

            let patch = format!(
                "*** Begin Patch\n*** Add File: {}\n+absolute\n*** End Patch",
                f.root.join("src/patch_absolute.txt").display()
            );

            let result = tool_apply_patch_exec(
                f.gcx.clone(),
                &args(vec![("patch", json!(patch))]),
                false,
                Some(&f.scope),
                None,
            )
            .await
            .unwrap();
            assert!(result
                .warnings
                .join("\n")
                .contains("absolute path was used"));
            assert_eq!(
                fs::read_to_string(f.root.join("src/patch_absolute.txt")).unwrap(),
                "absolute\n"
            );

            fs::write(f.root.join("src/patch_move.txt"), "move old\n").unwrap();
            fs::write(f.source.join("src/patch_move.txt"), "source\n").unwrap();
            let patch = format!(
                "*** Begin Patch\n*** Update File: {}\n*** Move to: {}\n@@\n-move old\n+move new\n*** End Patch",
                f.source.join("src/patch_move.txt").display(),
                f.source.join("src/patch_moved.txt").display()
            );

            let result = tool_apply_patch_exec(
                f.gcx.clone(),
                &args(vec![("patch", json!(patch))]),
                false,
                Some(&f.scope),
                None,
            )
            .await
            .unwrap();
            assert!(result
                .warnings
                .join("\n")
                .contains("mapped to active worktree"));
            assert!(!f.root.join("src/patch_move.txt").exists());
            assert_eq!(
                fs::read_to_string(f.root.join("src/patch_moved.txt")).unwrap(),
                "move new\n"
            );
            assert_eq!(
                fs::read_to_string(f.source.join("src/patch_move.txt")).unwrap(),
                "source\n"
            );
            assert!(!f.source.join("src/patch_moved.txt").exists());

            let outside_file = f.outside.join("patch_outside.txt");
            fs::write(&outside_file, "outside old\n").unwrap();
            let patch = format!(
                "*** Begin Patch\n*** Update File: {}\n@@\n-outside old\n+outside new\n*** End Patch",
                outside_file.display()
            );

            let result = tool_apply_patch_exec(
                f.gcx.clone(),
                &args(vec![("patch", json!(patch))]),
                false,
                Some(&f.scope),
                None,
            )
            .await
            .unwrap();
            assert!(result
                .warnings
                .join("\n")
                .contains("outside active worktree"));
            assert_eq!(fs::read_to_string(&outside_file).unwrap(), "outside new\n");
        }

        const MUTATION_FAMILIES: &[&str] = &[
            "create", "replace", "regex", "anchored", "line", "patch", "rename", "delete", "undo",
        ];

        const GUARDED: &str = "head\nanchor\nbody\n";

        async fn prepare_family(family: &str, path: &Path) {
            let _ = fs::remove_file(path);
            if family != "create" {
                fs::write(path, GUARDED).unwrap();
            }
            if family == "undo" {
                crate::tools::file_edit::undo_history::record_edit(
                    &path.to_path_buf(),
                    Some("older\n"),
                    Some(GUARDED),
                );
            }
        }

        async fn run_family(
            f: &Fixture,
            ccx: &Arc<AMutex<AtCommandsContext>>,
            family: &str,
            path: &Path,
        ) -> Result<(), String> {
            let value = path_value(path);
            let destination = path_value(&path.with_extension("moved"));
            match family {
                "create" => tool_create_text_doc_exec(
                    f.gcx.clone(),
                    &args(vec![("path", value), ("content", json!("created"))]),
                    false,
                    Some(&f.scope),
                    Some(ccx),
                )
                .await
                .map(|_| ()),
                "replace" => tool_update_text_doc_exec(
                    f.gcx.clone(),
                    &args(vec![
                        ("path", value),
                        ("old_str", json!("body")),
                        ("replacement", json!("changed")),
                    ]),
                    false,
                    Some(&f.scope),
                    Some(ccx),
                )
                .await
                .map(|_| ()),
                "regex" => tool_update_text_doc_regex_exec(
                    f.gcx.clone(),
                    &args(vec![
                        ("path", value),
                        ("pattern", json!("body")),
                        ("replacement", json!("changed")),
                    ]),
                    false,
                    Some(&f.scope),
                    Some(ccx),
                )
                .await
                .map(|_| ()),
                "anchored" => tool_update_text_doc_anchored_exec(
                    f.gcx.clone(),
                    &args(vec![
                        ("path", value),
                        ("mode", json!("insert_after")),
                        ("anchor", json!("anchor")),
                        ("content", json!("\nchanged")),
                    ]),
                    false,
                    Some(&f.scope),
                    Some(ccx),
                )
                .await
                .map(|_| ()),
                "line" => tool_update_text_doc_by_lines_exec(
                    f.gcx.clone(),
                    &args(vec![
                        ("path", value),
                        ("content", json!("changed")),
                        ("ranges", json!("3")),
                    ]),
                    false,
                    Some(&f.scope),
                    Some(ccx),
                )
                .await
                .map(|_| ()),
                "patch" => tool_apply_patch_exec(
                    f.gcx.clone(),
                    &args(vec![(
                        "patch",
                        json!(format!(
                            "*** Begin Patch\n*** Update File: {}\n@@\n-body\n+changed\n*** End Patch",
                            path.display()
                        )),
                    )]),
                    false,
                    Some(&f.scope),
                    Some(ccx),
                )
                .await
                .map(|_| ()),
                "rename" => ToolMv {
                    config_path: String::new(),
                }
                .tool_execute(
                    ccx.clone(),
                    &"mv".to_string(),
                    &args(vec![("source", value), ("destination", destination)]),
                )
                .await
                .map(|_| ()),
                "delete" => ToolRm {
                    config_path: String::new(),
                }
                .tool_execute(
                    ccx.clone(),
                    &"rm".to_string(),
                    &args(vec![("path", value)]),
                )
                .await
                .map(|_| ()),
                "undo" => ToolUndoTextDoc {
                    config_path: String::new(),
                }
                .tool_execute(ccx.clone(), &"undo".to_string(), &args(vec![("path", value)]))
                .await
                .map(|_| ()),
                other => panic!("unknown mutation family {other}"),
            }
        }

        #[tokio::test]
        async fn strict_scope_admits_and_refuses_every_built_in_mutation_family() {
            let f = fixture().await;
            fs::create_dir_all(f.root.join("src/scoped")).unwrap();
            let ccx = scope_guard_ccx(
                &f,
                ScopeGuardMode::Strict,
                vec!["src/scoped".to_string()],
                "agents",
            )
            .await;

            for family in MUTATION_FAMILIES.iter().copied() {
                let inside = f.root.join(format!("src/scoped/{family}.rs"));
                prepare_family(family, &inside).await;
                run_family(&f, &ccx, family, &inside)
                    .await
                    .unwrap_or_else(|error| panic!("in-scope {family} was refused: {error}"));

                let outside = f.root.join(format!("src/out_{family}.rs"));
                prepare_family(family, &outside).await;
                let error = run_family(&f, &ccx, family, &outside)
                    .await
                    .expect_err(family);
                assert!(
                    error.contains("enforced path scope"),
                    "{family}: refusal did not name enforcement: {error}"
                );
                assert!(
                    error.contains("src/scoped"),
                    "{family}: refusal did not name the violated scope: {error}"
                );
                if family == "create" {
                    assert!(!outside.exists(), "{family} created an out-of-scope file");
                } else {
                    assert_eq!(
                        fs::read_to_string(&outside).unwrap(),
                        GUARDED,
                        "{family} mutated an out-of-scope file"
                    );
                }
                assert!(
                    !outside.with_extension("moved").exists(),
                    "{family} produced an out-of-scope destination"
                );
            }
        }

        #[cfg(unix)]
        #[tokio::test]
        async fn strict_scope_cannot_be_escaped_through_a_symlink_alias() {
            let f = fixture().await;
            fs::create_dir_all(f.root.join("src/scoped")).unwrap();
            fs::write(f.outside.join("secret.rs"), GUARDED).unwrap();
            std::os::unix::fs::symlink(
                f.outside.join("secret.rs"),
                f.root.join("src/scoped/alias.rs"),
            )
            .unwrap();
            std::os::unix::fs::symlink(&f.outside, f.root.join("src/scoped/gate")).unwrap();
            let ccx = scope_guard_ccx(
                &f,
                ScopeGuardMode::Strict,
                vec!["src/scoped".to_string()],
                "agents",
            )
            .await;

            let aliased = run_family(&f, &ccx, "replace", &f.root.join("src/scoped/alias.rs"))
                .await
                .unwrap_err();
            assert!(aliased.contains("enforced path scope"), "{aliased}");
            assert_eq!(
                fs::read_to_string(f.outside.join("secret.rs")).unwrap(),
                GUARDED
            );

            // A symlinked directory is already refused one layer earlier, by
            // worktree path resolution, so the escape never reaches the scope
            // guard at all. Assert the refusal and that nothing was written.
            let through_dir = tool_create_text_doc_exec(
                f.gcx.clone(),
                &args(vec![
                    ("path", path_value(&f.root.join("src/scoped/gate/new.rs"))),
                    ("content", json!("escaped")),
                ]),
                false,
                Some(&f.scope),
                Some(&ccx),
            )
            .await
            .unwrap_err();
            assert!(
                through_dir.contains("escapes active worktree root"),
                "{through_dir}"
            );
            assert!(!f.outside.join("new.rs").exists());

            // Without the worktree layer the path scope guard alone still
            // refuses the same escape.
            assert!(matches!(
                crate::agents::path_scope::decide(
                    ScopeGuardMode::Strict,
                    &crate::agents::path_scope::PathScope::declared(
                        vec!["src/scoped".to_string()],
                        true,
                    ),
                    &f.root,
                    &f.root.join("src/scoped/gate/new.rs"),
                    &refact_work_core::CoordinationSettings::default(),
                )
                .unwrap(),
                crate::agents::path_scope::ScopeOutcome::Refused { .. }
            ));
        }

        #[tokio::test]
        async fn resume_may_narrow_the_enforced_scope_but_not_widen_it() {
            let f = fixture().await;
            fs::create_dir_all(f.root.join("src/scoped")).unwrap();
            fs::create_dir_all(f.root.join("src/extra")).unwrap();
            fs::write(f.root.join("src/scoped/a.rs"), GUARDED).unwrap();
            fs::write(f.root.join("src/extra/b.rs"), GUARDED).unwrap();

            let accepted = scope_guard_ccx(
                &f,
                ScopeGuardMode::Strict,
                vec!["src/scoped".to_string(), "src/extra".to_string()],
                "agents",
            )
            .await;
            run_family(&f, &accepted, "replace", &f.root.join("src/extra/b.rs"))
                .await
                .unwrap();

            fs::write(f.root.join("src/extra/b.rs"), GUARDED).unwrap();
            let narrowed = scope_guard_ccx(
                &f,
                ScopeGuardMode::Strict,
                vec!["src/scoped".to_string()],
                "agents",
            )
            .await;
            assert!(
                run_family(&f, &narrowed, "replace", &f.root.join("src/extra/b.rs"))
                    .await
                    .unwrap_err()
                    .contains("enforced path scope")
            );

            fs::create_dir_all(f.root.join("src/widened")).unwrap();
            fs::write(f.root.join("src/widened/c.rs"), GUARDED).unwrap();
            let widened = scope_guard_ccx(
                &f,
                ScopeGuardMode::Strict,
                vec![
                    "src/scoped".to_string(),
                    "src/extra".to_string(),
                    "src/widened".to_string(),
                ],
                "agents",
            )
            .await;
            for path in ["src/extra/b.rs", "src/widened/c.rs"] {
                assert!(
                    run_family(&f, &widened, "replace", &f.root.join(path))
                        .await
                        .unwrap_err()
                        .contains("enforced path scope"),
                    "{path} was re-granted by a widening resume"
                );
            }
            run_family(&f, &widened, "replace", &f.root.join("src/scoped/a.rs"))
                .await
                .unwrap();
        }

        #[tokio::test]
        async fn a_run_without_a_declared_scope_is_unchanged_by_strict_mode() {
            let f = fixture().await;
            let ccx = scope_guard_ccx(&f, ScopeGuardMode::Strict, vec![], "agents").await;
            let path = f.root.join("src/anywhere.rs");
            fs::write(&path, GUARDED).unwrap();

            run_family(&f, &ccx, "replace", &path).await.unwrap();
            assert!(fs::read_to_string(&path).unwrap().contains("changed"));
        }

        #[tokio::test]
        async fn non_strict_target_files_are_reported_as_declared_intent_only() {
            use crate::agents::path_scope::scope_for_mode;
            let limits = refact_work_core::CoordinationSettings::default();
            let files = vec!["src/scoped/a.rs".to_string()];

            let declared = scope_for_mode(ScopeGuardMode::Reject, &files, &limits);
            assert!(!declared.is_enforced());
            assert!(declared
                .describe()
                .contains("declared intent only, not enforced"));

            let enforced = scope_for_mode(ScopeGuardMode::Strict, &files, &limits);
            assert!(enforced.is_enforced());
            assert!(enforced
                .describe()
                .contains("enforced on every built-in mutation"));

            let f = fixture().await;
            let ccx = scope_guard_ccx(&f, ScopeGuardMode::Reject, files.clone(), "agents").await;
            let path = f.root.join("src/other.rs");
            fs::write(&path, GUARDED).unwrap();
            let error = run_family(&f, &ccx, "replace", &path).await.unwrap_err();
            assert!(error.contains("target_files scope"), "{error}");
            assert!(!error.contains("enforced path scope"), "{error}");
        }

        #[tokio::test]
        async fn scope_guard_off_allows_outside_target_files() {
            let f = fixture().await;
            let ccx = scope_guard_ccx(
                &f,
                ScopeGuardMode::Off,
                vec!["src/allowed.rs".to_string()],
                "agents",
            )
            .await;
            fs::write(f.root.join("src/outside.rs"), "old\n").unwrap();

            check_scope_guard(&ccx, &f.root.join("src/outside.rs"))
                .await
                .unwrap();
        }

        #[tokio::test]
        async fn scope_guard_reject_errors_outside_target_files() {
            let f = fixture().await;
            let ccx = scope_guard_ccx(
                &f,
                ScopeGuardMode::Reject,
                vec!["src/allowed.rs".to_string()],
                "agents",
            )
            .await;
            fs::write(f.root.join("src/outside.rs"), "old\n").unwrap();

            let error = check_scope_guard(&ccx, &f.root.join("src/outside.rs"))
                .await
                .unwrap_err();

            assert!(error.contains("outside the card's target_files scope"));
        }

        #[tokio::test]
        async fn scope_guard_warn_allows_outside_target_files() {
            let f = fixture().await;
            let ccx = scope_guard_ccx(
                &f,
                ScopeGuardMode::Warn,
                vec!["src/allowed.rs".to_string()],
                "agents",
            )
            .await;
            fs::write(f.root.join("src/outside.rs"), "old\n").unwrap();

            check_scope_guard(&ccx, &f.root.join("src/outside.rs"))
                .await
                .unwrap();
        }

        #[tokio::test]
        async fn scope_guard_reject_allows_inside_target_files() {
            let f = fixture().await;
            let ccx = scope_guard_ccx(
                &f,
                ScopeGuardMode::Reject,
                vec!["src/allowed.rs".to_string()],
                "agents",
            )
            .await;
            fs::write(f.root.join("src/allowed.rs"), "old\n").unwrap();

            check_scope_guard(&ccx, &f.root.join("src/allowed.rs"))
                .await
                .unwrap();
        }

        #[tokio::test]
        async fn scope_guard_reject_allows_test_files() {
            let f = fixture().await;
            let ccx = scope_guard_ccx(
                &f,
                ScopeGuardMode::Reject,
                vec!["src/allowed.rs".to_string()],
                "agents",
            )
            .await;
            fs::write(f.root.join("src/allowed_test.rs"), "old\n").unwrap();

            check_scope_guard(&ccx, &f.root.join("src/allowed_test.rs"))
                .await
                .unwrap();
        }

        #[tokio::test]
        async fn scope_guard_skips_non_agent_context() {
            let f = fixture().await;
            let ccx = scope_guard_ccx(
                &f,
                ScopeGuardMode::Reject,
                vec!["src/allowed.rs".to_string()],
                "planner",
            )
            .await;
            fs::write(f.root.join("src/outside.rs"), "old\n").unwrap();

            check_scope_guard(&ccx, &f.root.join("src/outside.rs"))
                .await
                .unwrap();
        }

        async fn native_scope_ccx(
            f: &Fixture,
            chat_id: &str,
            worktree_kind: &str,
        ) -> Arc<AMutex<AtCommandsContext>> {
            let mut worktree = f.worktree.clone();
            worktree.kind = worktree_kind.to_string();
            Arc::new(AMutex::new(
                AtCommandsContext::new_from_app(
                    crate::app_state::AppState::from_gcx(f.gcx.clone()).await,
                    4096,
                    20,
                    false,
                    Vec::new(),
                    chat_id.to_string(),
                    None,
                    "model".to_string(),
                    None,
                    Some(worktree),
                )
                .await,
            ))
        }

        #[tokio::test]
        async fn native_edit_scope_fails_closed_for_supervised_chat_without_live_grant() {
            let f = fixture().await;
            let ccx = native_scope_ccx(&f, "subchat-supervised-unbound", "task_agent").await;
            fs::write(f.root.join("src/allowed.rs"), "old\n").unwrap();
            fs::write(f.root.join("src/allowed_test.rs"), "old\n").unwrap();

            for path in ["src/allowed.rs", "src/allowed_test.rs"] {
                let error = check_scope_guard(&ccx, &f.root.join(path))
                    .await
                    .unwrap_err();
                assert!(
                    error.contains("supervised edit authority is unavailable"),
                    "{path}: {error}"
                );
            }
        }

        #[tokio::test]
        async fn native_edit_scope_fails_closed_in_supervised_worktree_without_live_grant() {
            let f = fixture().await;
            let ccx = native_scope_ccx(
                &f,
                "chat",
                crate::buddy::supervision_authority::SUPERVISED_WORKTREE_KIND,
            )
            .await;
            fs::write(f.root.join("src/allowed.rs"), "old\n").unwrap();

            let error = check_scope_guard(&ccx, &f.root.join("src/allowed.rs"))
                .await
                .unwrap_err();
            assert!(
                error.contains("supervised edit authority is unavailable"),
                "{error}"
            );
        }

        #[tokio::test]
        async fn native_edit_scope_leaves_unsupervised_subchat_untouched() {
            let f = fixture().await;
            let ccx = native_scope_ccx(&f, "subchat-plain", "subagent").await;
            fs::write(f.root.join("src/outside.rs"), "old\n").unwrap();

            check_scope_guard(&ccx, &f.root.join("src/outside.rs"))
                .await
                .unwrap();
        }

        #[tokio::test]
        async fn worktree_scope_tools_rm_mv_resolve_and_warn() {
            let f = fixture().await;
            let ccx = ccx(&f).await;
            let tool_call_id = "tool".to_string();

            fs::write(f.root.join("src/remove.txt"), "remove\n").unwrap();
            fs::write(f.source.join("src/remove.txt"), "source\n").unwrap();
            let mut rm = ToolRm {
                config_path: String::new(),
            };
            let (_, messages) = rm
                .tool_execute(
                    ccx.clone(),
                    &tool_call_id,
                    &args(vec![
                        ("path", path_value(&f.source.join("src/remove.txt"))),
                        ("dry_run", json!(true)),
                    ]),
                )
                .await
                .unwrap();
            assert!(tool_text(&messages).contains("mapped to active worktree"));
            assert!(f.root.join("src/remove.txt").exists());
            let (_, messages) = rm
                .tool_execute(
                    ccx.clone(),
                    &tool_call_id,
                    &args(vec![("path", path_value(&f.source.join("src/remove.txt")))]),
                )
                .await
                .unwrap();
            assert!(tool_text(&messages).contains("mapped to active worktree"));
            assert!(!f.root.join("src/remove.txt").exists());
            assert_eq!(
                fs::read_to_string(f.source.join("src/remove.txt")).unwrap(),
                "source\n"
            );

            fs::write(f.root.join("src/move.txt"), "move\n").unwrap();
            fs::write(f.source.join("src/move.txt"), "source\n").unwrap();
            let mut mv = ToolMv {
                config_path: String::new(),
            };
            let (_, messages) = mv
                .tool_execute(
                    ccx.clone(),
                    &tool_call_id,
                    &args(vec![
                        ("source", path_value(&f.source.join("src/move.txt"))),
                        ("destination", path_value(&f.source.join("src/moved.txt"))),
                    ]),
                )
                .await
                .unwrap();
            assert!(tool_text(&messages).contains("mapped to active worktree"));
            assert!(!f.root.join("src/move.txt").exists());
            assert_eq!(
                fs::read_to_string(f.root.join("src/moved.txt")).unwrap(),
                "move\n"
            );
            assert_eq!(
                fs::read_to_string(f.source.join("src/move.txt")).unwrap(),
                "source\n"
            );
            assert!(!f.source.join("src/moved.txt").exists());

            let outside_src = f.outside.join("outside_move.txt");
            let outside_dst = f.outside.join("outside_moved.txt");
            fs::write(&outside_src, "outside\n").unwrap();
            let (_, messages) = mv
                .tool_execute(
                    ccx,
                    &tool_call_id,
                    &args(vec![
                        ("source", path_value(&outside_src)),
                        ("destination", path_value(&outside_dst)),
                    ]),
                )
                .await
                .unwrap();
            assert!(tool_text(&messages).contains("outside active worktree"));
            assert!(!outside_src.exists());
            assert_eq!(fs::read_to_string(&outside_dst).unwrap(), "outside\n");
        }

        #[tokio::test]
        async fn worktree_scope_tools_shell_defaults_and_workdirs() {
            let f = fixture().await;
            let ccx = ccx(&f).await;
            let tool_call_id = "shell".to_string();
            let mut shell = ToolShell::default();

            fs::write(f.root.join("pwd_marker.txt"), "worktree\n").unwrap();
            fs::write(f.source.join("pwd_marker.txt"), "source\n").unwrap();

            let (_, messages) = shell
                .tool_execute(
                    ccx.clone(),
                    &tool_call_id,
                    &args(vec![
                        ("command", json!(pwd_command())),
                        ("description", json!("Print the active worktree directory")),
                    ]),
                )
                .await
                .unwrap();
            let text = tool_text(&messages);
            assert!(text.contains(&f.root.to_string_lossy().to_string()));
            assert!(text.contains("shell cwd/workdir is enforced"));
            assert_eq!(
                fs::read_to_string(f.root.join("pwd_marker.txt")).unwrap(),
                "worktree\n"
            );
            assert_eq!(
                fs::read_to_string(f.source.join("pwd_marker.txt")).unwrap(),
                "source\n"
            );

            let (_, messages) = shell
                .tool_execute(
                    ccx.clone(),
                    &tool_call_id,
                    &args(vec![
                        ("command", json!(pwd_command())),
                        ("description", json!("Print the mapped workdir directory")),
                        ("workdir", path_value(&f.source.join("src"))),
                    ]),
                )
                .await
                .unwrap();
            let text = tool_text(&messages);
            assert!(text.contains(&f.root.join("src").to_string_lossy().to_string()));
            assert!(text.contains("mapped to active worktree"));

            let (_, messages) = shell
                .tool_execute(
                    ccx,
                    &tool_call_id,
                    &args(vec![
                        ("command", json!(pwd_command())),
                        ("description", json!("Print the outside workdir directory")),
                        ("workdir", path_value(&f.outside)),
                    ]),
                )
                .await
                .unwrap();
            let text = tool_text(&messages);
            assert!(text.contains(&f.outside.to_string_lossy().to_string()));
            assert!(text.contains("outside active worktree"));
        }
    }
}
