use crate::ast::ast_indexer_thread::{ast_indexer_block_until_finished, ast_indexer_enqueue_files};
use crate::at_commands::at_file::{file_repair_candidates, return_one_candidate_or_a_good_error};
use crate::call_validation::DiffChunk;
use crate::files_correction::{
    canonicalize_normalized_path, check_if_its_inside_a_workspace_or_config,
    correct_to_nearest_dir_path, get_project_dirs, preprocess_path_for_normalization,
};
use crate::files_in_workspace::get_file_text_from_memory_or_disk;
use crate::global_context::GlobalContext;
use crate::privacy::{check_file_privacy, FilePrivacyLevel, PrivacySettings};
use regex::{Match, Regex};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock as ARwLock;
use tracing::warn;

fn resolve_path_with_workdir(path: &PathBuf, code_workdir: &Option<PathBuf>) -> PathBuf {
    let Some(workdir) = code_workdir else {
        return path.clone();
    };

    if !path.is_absolute() {
        return workdir.join(path);
    }

    if path.starts_with(&workdir) {
        return path.clone();
    }

    if let Some(workspace_root) = workdir
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
    {
        if path.starts_with(&workspace_root) {
            if let Ok(relative) = path.strip_prefix(&workspace_root) {
                return workdir.join(relative);
            }
        }
    }

    warn!("Cannot properly resolve {:?} to worktree, using filename only", path);
    workdir.join(path.file_name().unwrap_or_default())
}

pub async fn parse_path_for_update(
    gcx: Arc<ARwLock<GlobalContext>>,
    args: &HashMap<String, Value>,
    privacy_settings: Arc<PrivacySettings>,
    code_workdir: &Option<PathBuf>,
) -> Result<PathBuf, String> {
    let s = parse_string_arg(args, "path", "Provide absolute path to file")?;
    let raw_path = preprocess_path_for_normalization(s.trim().to_string());
    let candidates = file_repair_candidates(gcx.clone(), &raw_path, 3, false).await;
    let path = return_one_candidate_or_a_good_error(
        gcx.clone(),
        &raw_path,
        &candidates,
        &get_project_dirs(gcx.clone()).await,
        false,
    )
    .await
    .map(|f| canonicalize_normalized_path(PathBuf::from(f)))?;

    let resolved_path = resolve_path_with_workdir(&path, code_workdir);

    if check_file_privacy(
        privacy_settings,
        &resolved_path,
        &FilePrivacyLevel::AllowToSendAnywhere,
    )
    .is_err()
    {
        return Err(format!(
            "⚠️ Cannot update {:?} (blocked by privacy). 💡 Choose file in allowed directory",
            resolved_path
        ));
    }
    if !resolved_path.exists() {
        return Err(format!(
            "⚠️ File {:?} not found. 💡 Use create_textdoc() for new files",
            resolved_path
        ));
    }
    Ok(resolved_path)
}

pub async fn parse_path_for_create(
    gcx: Arc<ARwLock<GlobalContext>>,
    args: &HashMap<String, Value>,
    privacy_settings: Arc<PrivacySettings>,
    code_workdir: &Option<PathBuf>,
) -> Result<PathBuf, String> {
    let s = parse_string_arg(args, "path", "Provide absolute path for new file")?;
    let raw_path = PathBuf::from(preprocess_path_for_normalization(s.trim().to_string()));

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
                &get_project_dirs(gcx.clone()).await,
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

    let resolved_path = resolve_path_with_workdir(&path, code_workdir);

    if check_file_privacy(
        privacy_settings,
        &resolved_path,
        &FilePrivacyLevel::AllowToSendAnywhere,
    )
    .is_err()
    {
        return Err(format!(
            "⚠️ Cannot create {:?} (blocked by privacy). 💡 Choose path in allowed directory",
            resolved_path
        ));
    }
    Ok(resolved_path)
}

pub fn parse_string_arg(
    args: &HashMap<String, Value>,
    name: &str,
    hint: &str,
) -> Result<String, String> {
    match args.get(name) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(v) => Err(format!("⚠️ '{}' must be a string, got: {:?}", name, v)),
        None => Err(format!("⚠️ Missing '{}'. 💡 {}", name, hint)),
    }
}

pub fn parse_bool_arg(
    args: &HashMap<String, Value>,
    name: &str,
    default: bool,
) -> Result<bool, String> {
    match args.get(name) {
        Some(Value::Bool(b)) => Ok(*b),
        Some(Value::String(s)) => match s.to_lowercase().as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(format!("⚠️ '{}' must be true/false, got: {}", name, s)),
        },
        Some(v) => Err(format!("⚠️ '{}' must be a boolean, got: {:?}", name, v)),
        None => Ok(default),
    }
}

pub fn edit_result_summary(before: &str, after: &str, path: &PathBuf) -> String {
    let before_lines = before.lines().count();
    let after_lines = after.lines().count();
    let diff = after_lines as i64 - before_lines as i64;
    let sign = if diff >= 0 { "+" } else { "" };
    format!(
        "✅ Updated {:?}: {} → {} lines ({}{})",
        path.file_name().unwrap_or_default(),
        before_lines,
        after_lines,
        sign,
        diff
    )
}

pub fn convert_edit_to_diffchunks(
    path: PathBuf,
    before: &String,
    after: &String,
) -> Result<Vec<DiffChunk>, String> {
    let diffs = diff::lines(before, after);
    let mut line_num = 0;
    let mut current_chunk_lines_remove = Vec::new();
    let mut current_chunk_lines_add = Vec::new();
    let mut current_chunk_line_nums = Vec::new();
    let mut current_chunk_is_plus = Vec::new();
    let mut diff_chunks = Vec::new();

    let flush_changes = |lines_remove: &Vec<String>,
                         lines_add: &Vec<String>,
                         line_nums: &Vec<usize>,
                         is_plus: &Vec<bool>|
     -> Option<DiffChunk> {
        if lines_remove.is_empty() && lines_add.is_empty() {
            return None;
        }

        let lines_remove = lines_remove.join("");
        let lines_add = lines_add.join("");

        let line1 = line_nums.iter().min().map(|&x| x + 1).unwrap_or(1);

        let line2 = line_nums
            .iter()
            .zip(is_plus.iter())
            .map(|(&num, &is_plus)| if is_plus { num + 1 } else { num + 2 })
            .max()
            .unwrap_or(1);

        Some(DiffChunk {
            file_name: path.to_string_lossy().to_string(),
            file_name_rename: None,
            file_action: "edit".to_string(),
            line1,
            line2,
            lines_remove,
            lines_add,
            ..Default::default()
        })
    };

    for diff in diffs {
        match diff {
            diff::Result::Left(l) => {
                current_chunk_lines_remove.push(format!("{}\n", l));
                current_chunk_line_nums.push(line_num);
                current_chunk_is_plus.push(false);
                line_num += 1;
            }
            diff::Result::Right(r) => {
                current_chunk_lines_add.push(format!("{}\n", r));
                current_chunk_line_nums.push(line_num);
                current_chunk_is_plus.push(true);
            }
            diff::Result::Both(_, _) => {
                if let Some(chunk) = flush_changes(
                    &current_chunk_lines_remove,
                    &current_chunk_lines_add,
                    &current_chunk_line_nums,
                    &current_chunk_is_plus,
                ) {
                    diff_chunks.push(chunk);
                }
                current_chunk_lines_remove.clear();
                current_chunk_lines_add.clear();
                current_chunk_line_nums.clear();
                current_chunk_is_plus.clear();
                line_num += 1;
            }
        }
    }

    if let Some(chunk) = flush_changes(
        &current_chunk_lines_remove,
        &current_chunk_lines_add,
        &current_chunk_line_nums,
        &current_chunk_is_plus,
    ) {
        diff_chunks.push(chunk);
    }

    Ok(diff_chunks)
}

pub fn normalize_line_endings(content: &str) -> String {
    content.replace("\r\n", "\n")
}

pub fn restore_line_endings(content: &str, original_had_crlf: bool) -> String {
    if original_had_crlf {
        content.replace("\n", "\r\n")
    } else {
        content.to_string()
    }
}

pub async fn await_ast_indexing(gcx: Arc<ARwLock<GlobalContext>>) -> Result<(), String> {
    let ast_service_mb = gcx.read().await.ast_service.clone();
    if let Some(ast_service) = &ast_service_mb {
        ast_indexer_block_until_finished(ast_service.clone(), 20_000, true).await;
    }
    Ok(())
}

pub async fn sync_documents_ast(
    gcx: Arc<ARwLock<GlobalContext>>,
    doc: &PathBuf,
) -> Result<(), String> {
    let ast_service_mb = gcx.read().await.ast_service.clone();
    if let Some(ast_service) = &ast_service_mb {
        ast_indexer_enqueue_files(
            ast_service.clone(),
            &vec![doc.to_string_lossy().to_string()],
            true,
        )
        .await;
    }
    Ok(())
}

pub async fn write_file(
    gcx: Arc<ARwLock<GlobalContext>>,
    path: &PathBuf,
    file_text: &String,
    dry: bool,
) -> Result<(String, String), String> {
    use crate::tools::file_edit::undo_history::record_before_edit;

    let parent = path.parent().ok_or(format!(
        "Failed to Add: {:?}. Path is invalid.\nReason: path must have had a parent directory",
        path
    ))?;

    if !parent.exists() {
        if !dry {
            fs::create_dir_all(&parent).map_err(|e| {
                let err = format!("Failed to Add: {:?}; Its parent dir {:?} did not exist and attempt to create it failed.\nERROR: {}", path, parent, e);
                warn!("{err}");
                err
            })?;
        }
    }

    let before_text = if path.exists() {
        get_file_text_from_memory_or_disk(gcx.clone(), path).await?
    } else {
        "".to_string()
    };

    if !dry {
        record_before_edit(path, &before_text);
        fs::write(&path, file_text).map_err(|e| {
            let err = format!("Failed to write file: {:?}\nERROR: {}", path, e);
            warn!("{err}");
            err
        })?;
        gcx.write()
            .await
            .documents_state
            .memory_document_map
            .remove(path);
    }

    Ok((before_text, file_text.to_string()))
}

pub async fn str_replace(
    gcx: Arc<ARwLock<GlobalContext>>,
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
    write_file(gcx.clone(), path, &new_file_content, dry).await?;
    Ok((file_content, new_file_content))
}

fn strip_line_number_prefixes(s: &str) -> String {
    let re = regex::Regex::new(r"(?m)^\d+[\t|:]\s?").unwrap();
    re.replace_all(s, "").to_string()
}

fn find_match_lines(content: &str, pattern: &str) -> Vec<usize> {
    let mut lines = Vec::new();
    let mut pos = 0;
    while let Some(idx) = content[pos..].find(pattern) {
        let abs_idx = pos + idx;
        let line_num = content[..abs_idx].lines().count() + 1;
        lines.push(line_num);
        pos = abs_idx + 1;
    }
    lines
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AnchorMode {
    ReplaceBetween,
    InsertAfter,
    InsertBefore,
}

pub async fn str_replace_anchored(
    gcx: Arc<ARwLock<GlobalContext>>,
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
    write_file(gcx.clone(), path, &new_file_content, dry).await?;
    Ok((file_content, new_file_content))
}

fn replace_between_anchors(
    content: &str,
    before: &str,
    after: &str,
    replacement: &str,
    multiple: bool,
) -> Result<String, String> {
    let before_positions: Vec<usize> = content.match_indices(before).map(|(i, _)| i).collect();
    if before_positions.is_empty() {
        return Err("⚠️ anchor_before not found. 💡 Use cat() to verify text exists".to_string());
    }

    let mut pairs: Vec<(usize, usize)> = Vec::new();
    for &b_start in &before_positions {
        let b_end = b_start + before.len();
        if let Some(rel_a) = content[b_end..].find(after) {
            pairs.push((b_start, b_end + rel_a));
        }
    }

    if pairs.is_empty() {
        return Err(
            "⚠️ anchor_after not found after anchor_before. 💡 Check anchor order".to_string(),
        );
    }
    if !multiple && pairs.len() > 1 {
        let lines: Vec<usize> = pairs
            .iter()
            .map(|(i, _)| content[..*i].lines().count() + 1)
            .collect();
        return Err(format!(
            "⚠️ {} anchor pairs at lines {:?}. 💡 Use more specific anchors, or set multiple:true",
            pairs.len(),
            lines
        ));
    }

    pairs.sort_by_key(|(start, _)| *start);
    for i in 1..pairs.len() {
        let prev_end = pairs[i - 1].1 + after.len();
        let curr_start = pairs[i].0;
        if curr_start < prev_end {
            let line1 = content[..pairs[i - 1].0].lines().count() + 1;
            let line2 = content[..curr_start].lines().count() + 1;
            return Err(format!(
                "⚠️ Overlapping anchor regions at lines {} and {}. 💡 Use more specific anchors",
                line1, line2
            ));
        }
    }

    let mut result = content.to_string();
    for (b_start, a_start) in pairs.into_iter().rev() {
        let b_end = b_start + before.len();
        let a_end = a_start + after.len();
        result = format!(
            "{}{}{}{}",
            &result[..b_end],
            replacement,
            after,
            &result[a_end..]
        );
    }
    Ok(result)
}

fn insert_at_anchor(
    content: &str,
    anchor: &str,
    insert: &str,
    multiple: bool,
    after: bool,
) -> Result<String, String> {
    let positions: Vec<usize> = content.match_indices(anchor).map(|(i, _)| i).collect();
    if positions.is_empty() {
        return Err("⚠️ Anchor not found. 💡 Use cat() to verify text exists".to_string());
    }
    if !multiple && positions.len() > 1 {
        let lines: Vec<usize> = positions
            .iter()
            .map(|i| content[..*i].lines().count() + 1)
            .collect();
        return Err(format!("⚠️ {} anchor occurrences at lines {:?}. 💡 Use more specific anchor, or set multiple:true", positions.len(), lines));
    }

    let mut result = content.to_string();
    for pos in positions.into_iter().rev() {
        let insert_pos = if after { pos + anchor.len() } else { pos };
        result.insert_str(insert_pos, insert);
    }
    Ok(result)
}

#[derive(Debug, Clone)]
pub struct LineRange {
    pub start: usize,
    pub end: usize,
}

pub fn parse_line_ranges(ranges_str: &str, total_lines: usize) -> Result<Vec<LineRange>, String> {
    let mut ranges = Vec::new();

    for part in ranges_str.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        let range = if part.contains(':') {
            let parts: Vec<&str> = part.splitn(2, ':').collect();
            let start_str = parts[0].trim();
            let end_str = parts[1].trim();

            let start = if start_str.is_empty() {
                1
            } else {
                start_str.parse::<usize>().map_err(|_| {
                    format!(
                        "⚠️ Invalid start '{}' in '{}'. 💡 Use numbers like '10:20'",
                        start_str, part
                    )
                })?
            };

            let end = if end_str.is_empty() {
                total_lines
            } else {
                end_str.parse::<usize>().map_err(|_| {
                    format!(
                        "⚠️ Invalid end '{}' in '{}'. 💡 Use numbers like '10:20'",
                        end_str, part
                    )
                })?
            };

            LineRange { start, end }
        } else {
            let line = part.parse::<usize>().map_err(|_| {
                format!(
                    "⚠️ Invalid line '{}'. 💡 Use number like '10' or range '10:20'",
                    part
                )
            })?;
            LineRange {
                start: line,
                end: line,
            }
        };

        if range.start == 0 {
            return Err("⚠️ Line numbers are 1-based, got 0. 💡 Use 1 for first line".to_string());
        }
        if range.end < range.start {
            return Err(format!(
                "⚠️ Invalid range '{}': end ({}) < start ({}). 💡 Use start:end format",
                part, range.end, range.start
            ));
        }
        if range.start > total_lines {
            return Err(format!(
                "⚠️ Line {} beyond EOF ({} lines). 💡 Use cat() to check file length",
                range.start, total_lines
            ));
        }

        ranges.push(range);
    }

    if ranges.is_empty() {
        return Err("⚠️ No ranges provided. 💡 Use format '10:20' or '5' or ':10,20:'".to_string());
    }

    let mut sorted: Vec<&LineRange> = ranges.iter().collect();
    sorted.sort_by_key(|r| r.start);

    for i in 1..sorted.len() {
        let prev = sorted[i - 1];
        let curr = sorted[i];
        if curr.start <= prev.end {
            return Err(format!(
                "⚠️ Overlapping ranges {}:{} and {}:{}. 💡 Ranges must not overlap",
                prev.start, prev.end, curr.start, curr.end
            ));
        }
    }

    Ok(ranges)
}

pub async fn str_replace_lines(
    gcx: Arc<ARwLock<GlobalContext>>,
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
        if range.end > total_lines {
            return Err(format!(
                "⚠️ Range end {} exceeds file length ({} lines). 💡 Use cat() to check file, or ':' for end",
                range.end, total_lines
            ));
        }
        let start_idx = range.start - 1;
        let end_idx = range.end;
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

    write_file(gcx.clone(), path, &new_file_content, dry).await?;
    Ok((file_content, new_file_content))
}

pub async fn str_replace_regex(
    gcx: Arc<ARwLock<GlobalContext>>,
    path: &PathBuf,
    pattern: &Regex,
    replacement: &String,
    multiple: bool,
    expected_matches: Option<usize>,
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

    let new_content = if multiple {
        pattern
            .replace_all(&normalized_content, normalized_replacement.as_str())
            .to_string()
    } else {
        pattern
            .replace(&normalized_content, normalized_replacement.as_str())
            .to_string()
    };
    let new_file_content = restore_line_endings(&new_content, has_crlf);
    write_file(gcx.clone(), path, &new_file_content, dry).await?;
    Ok((file_content, new_file_content))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_line_ranges_single() {
        assert!(parse_line_ranges("5", 10).is_ok());
        assert!(parse_line_ranges("1:10", 10).is_ok());
        assert!(parse_line_ranges(":5", 10).is_ok());
        assert!(parse_line_ranges("5:", 10).is_ok());
    }

    #[test]
    fn test_parse_line_ranges_multiple() {
        let ranges = parse_line_ranges("1:3,7:9", 10).unwrap();
        assert_eq!(ranges.len(), 2);
    }

    #[test]
    fn test_parse_line_ranges_errors() {
        assert!(parse_line_ranges("0", 10).is_err());
        assert!(parse_line_ranges("5:3", 10).is_err());
        assert!(parse_line_ranges("15", 10).is_err());
        assert!(parse_line_ranges("abc", 10).is_err());
        assert!(parse_line_ranges("", 10).is_err());
    }

    #[test]
    fn test_parse_line_ranges_overlap() {
        assert!(parse_line_ranges("1:5,3:7", 10).is_err());
        assert!(parse_line_ranges("1:5,5:7", 10).is_err());
    }

    #[test]
    fn test_parse_line_ranges_preserves_order() {
        let ranges = parse_line_ranges("4:4,2:2", 10).unwrap();
        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[0].start, 4);
        assert_eq!(ranges[1].start, 2);
    }

    #[test]
    fn test_normalize_line_endings() {
        assert_eq!(normalize_line_endings("a\r\nb\r\n"), "a\nb\n");
        assert_eq!(normalize_line_endings("a\nb\n"), "a\nb\n");
    }

    #[test]
    fn test_restore_line_endings() {
        assert_eq!(restore_line_endings("a\nb\n", true), "a\r\nb\r\n");
        assert_eq!(restore_line_endings("a\nb\n", false), "a\nb\n");
    }

    #[test]
    fn test_strip_line_number_prefixes() {
        assert_eq!(strip_line_number_prefixes("1\tfoo\n2\tbar"), "foo\nbar");
        assert_eq!(strip_line_number_prefixes("10|foo\n20|bar"), "foo\nbar");
        assert_eq!(strip_line_number_prefixes("1: foo\n2: bar"), "foo\nbar");
        assert_eq!(strip_line_number_prefixes("no prefix"), "no prefix");
    }

    #[test]
    fn test_find_match_lines() {
        let content = "line1\nfoo\nline3\nfoo\nline5";
        let lines = find_match_lines(content, "foo");
        assert_eq!(lines, vec![2, 4]);
    }

    #[test]
    fn test_replace_between_anchors_single() {
        let content = "start\nBEGIN\nold\nEND\nfinish";
        let result = replace_between_anchors(content, "BEGIN\n", "END", "new\n", false).unwrap();
        assert_eq!(result, "start\nBEGIN\nnew\nEND\nfinish");
    }

    #[test]
    fn test_replace_between_anchors_multiple() {
        let content = "A\nBEGIN\nx\nEND\nB\nBEGIN\ny\nEND\nC";
        let result = replace_between_anchors(content, "BEGIN\n", "END", "z\n", true).unwrap();
        assert!(result.contains("z\n"));
    }

    #[test]
    fn test_replace_between_anchors_not_found() {
        let content = "no anchors here";
        assert!(replace_between_anchors(content, "BEGIN", "END", "x", false).is_err());
    }

    #[test]
    fn test_replace_between_anchors_overlap_error() {
        let content = "A{B{C}D}E";
        assert!(replace_between_anchors(content, "{", "}", "x", true).is_err());
    }

    #[test]
    fn test_insert_at_anchor_after() {
        let content = "line1\nANCHOR\nline3";
        let result = insert_at_anchor(content, "ANCHOR", "\ninserted", false, true).unwrap();
        assert_eq!(result, "line1\nANCHOR\ninserted\nline3");
    }

    #[test]
    fn test_insert_at_anchor_before() {
        let content = "line1\nANCHOR\nline3";
        let result = insert_at_anchor(content, "ANCHOR", "inserted\n", false, false).unwrap();
        assert_eq!(result, "line1\ninserted\nANCHOR\nline3");
    }

    #[test]
    fn test_insert_at_anchor_not_found() {
        assert!(insert_at_anchor("content", "MISSING", "x", false, true).is_err());
    }

    #[test]
    fn test_insert_at_anchor_multiple_error() {
        let content = "A\nA\nA";
        assert!(insert_at_anchor(content, "A", "x", false, true).is_err());
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
    }

    #[test]
    fn test_edit_result_summary() {
        let path = PathBuf::from("/path/to/file.rs");
        let summary = edit_result_summary("a\nb\nc", "a\nb\nc\nd\ne", &path);
        assert!(summary.contains("file.rs"));
        assert!(summary.contains("3"));
        assert!(summary.contains("5"));
        assert!(summary.contains("+2"));
    }
}
