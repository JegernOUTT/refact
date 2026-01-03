use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum, DiffChunk};
use crate::global_context::GlobalContext;
use crate::integrations::integr_abstract::IntegrationConfirmation;
use crate::privacy::load_privacy_if_needed;
use crate::tools::file_edit::auxiliary::{
    await_ast_indexing, convert_edit_to_diffchunks, edit_result_summary, normalize_line_endings,
    parse_path_for_update, parse_string_arg, restore_line_endings, sync_documents_ast, write_file,
};
use crate::tools::tools_description::{
    MatchConfirmDeny, MatchConfirmDenyResult, Tool, ToolDesc, ToolParam, ToolSource, ToolSourceType,
};
use crate::files_in_workspace::get_file_text_from_memory_or_disk;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex as AMutex;
use tokio::sync::RwLock as ARwLock;

pub struct ToolApplyPatch {
    pub config_path: String,
}

struct Args {
    path: PathBuf,
    patch: String,
}

async fn parse_args(
    gcx: Arc<ARwLock<GlobalContext>>,
    args: &HashMap<String, Value>,
    code_workdir: &Option<std::path::PathBuf>,
) -> Result<Args, String> {
    let privacy = load_privacy_if_needed(gcx.clone()).await;
    let path = parse_path_for_update(gcx, args, privacy, code_workdir).await?;
    let patch = parse_string_arg(args, "patch", "Provide unified diff patch")?;
    Ok(Args { path, patch })
}

struct Hunk {
    old_start: usize,
    old_count: usize,
    old_lines: Vec<String>,
    new_lines: Vec<String>,
}

fn parse_unified_diff(patch: &str) -> Result<Vec<Hunk>, String> {
    let patch = normalize_line_endings(patch);
    let lines: Vec<&str> = patch.lines().collect();
    let mut hunks = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        if lines[i].starts_with("@@") {
            let (old_start, old_count) = parse_hunk_header(lines[i])?;
            let mut old_lines = Vec::new();
            let mut new_lines = Vec::new();
            i += 1;

            while i < lines.len() && !lines[i].starts_with("@@") {
                let line = lines[i];
                if line.starts_with('-') {
                    old_lines.push(line[1..].to_string());
                } else if line.starts_with('+') {
                    new_lines.push(line[1..].to_string());
                } else if line.starts_with(' ') {
                    let content = line[1..].to_string();
                    old_lines.push(content.clone());
                    new_lines.push(content);
                } else if line.is_empty() {
                    old_lines.push(String::new());
                    new_lines.push(String::new());
                } else if line.starts_with("---")
                    || line.starts_with("+++")
                    || line.starts_with("\\")
                {
                    i += 1;
                    continue;
                } else {
                    break;
                }
                i += 1;
            }

            if old_lines.is_empty() && new_lines.is_empty() {
                continue;
            }
            if old_start != 0 && old_lines.len() != old_count {
                return Err(format!(
                    "⚠️ Hunk header says {} old lines but body has {}. 💡 Regenerate patch",
                    old_count,
                    old_lines.len()
                ));
            }
            hunks.push(Hunk {
                old_start,
                old_count,
                old_lines,
                new_lines,
            });
        } else {
            i += 1;
        }
    }

    if hunks.is_empty() {
        return Err(
            "⚠️ No valid hunks found. 💡 Use unified diff: @@ -line,count +line,count @@"
                .to_string(),
        );
    }
    Ok(hunks)
}

fn parse_hunk_header(header: &str) -> Result<(usize, usize), String> {
    let header = header.trim_start_matches("@@").trim();
    let parts: Vec<&str> = header.split_whitespace().collect();
    if parts.is_empty() {
        return Err("⚠️ Invalid hunk header: missing line info".to_string());
    }

    let old_range = parts[0].trim_start_matches('-');
    let (start, count) = if old_range.contains(',') {
        let p: Vec<&str> = old_range.split(',').collect();
        let s = p[0]
            .parse::<usize>()
            .map_err(|_| format!("⚠️ Invalid start '{}' in hunk header", p[0]))?;
        let c = p[1]
            .parse::<usize>()
            .map_err(|_| format!("⚠️ Invalid count '{}' in hunk header", p[1]))?;
        (s, c)
    } else {
        let s = old_range
            .parse::<usize>()
            .map_err(|_| format!("⚠️ Invalid line '{}' in hunk header", old_range))?;
        (s, 1)
    };

    if start == 0 && count != 0 {
        return Err(
            "⚠️ Line 0 only valid with count 0 (insert at top). 💡 Use @@ -0,0 +1,N @@".to_string(),
        );
    }
    Ok((start, count))
}

fn apply_hunks(content: &str, hunks: Vec<Hunk>) -> Result<String, String> {
    let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();

    for (idx, hunk) in hunks.into_iter().enumerate().rev() {
        let start_idx = if hunk.old_start == 0 {
            0
        } else {
            hunk.old_start - 1
        };
        let end_idx = start_idx + hunk.old_lines.len();

        if hunk.old_start == 0 && hunk.old_count == 0 {
            lines.splice(0..0, hunk.new_lines);
            continue;
        }

        if start_idx > lines.len() {
            return Err(format!(
                "⚠️ Hunk {} starts at line {} but file has {} lines. 💡 Re-read with cat()",
                idx + 1,
                hunk.old_start,
                lines.len()
            ));
        }
        if end_idx > lines.len() {
            return Err(format!(
                "⚠️ Hunk {} extends to line {} but file has {} lines. 💡 Check boundaries",
                idx + 1,
                end_idx,
                lines.len()
            ));
        }

        let file_slice: Vec<&str> = lines[start_idx..end_idx]
            .iter()
            .map(|s| s.as_str())
            .collect();
        let expected: Vec<&str> = hunk.old_lines.iter().map(|s| s.as_str()).collect();
        if file_slice != expected {
            return Err(format!(
                "⚠️ Hunk {} mismatch at line {}. 💡 File changed, re-read with cat()",
                idx + 1,
                hunk.old_start
            ));
        }

        lines.splice(start_idx..end_idx, hunk.new_lines);
    }

    Ok(lines.join("\n"))
}

pub async fn tool_apply_patch_exec(
    gcx: Arc<ARwLock<GlobalContext>>,
    args: &HashMap<String, Value>,
    dry: bool,
    code_workdir: &Option<std::path::PathBuf>,
) -> Result<(String, String, Vec<DiffChunk>, String), String> {
    let a = parse_args(gcx.clone(), args, code_workdir).await?;
    await_ast_indexing(gcx.clone()).await?;

    let file_content = get_file_text_from_memory_or_disk(gcx.clone(), &a.path).await?;
    let has_crlf = file_content.contains("\r\n");
    let normalized = normalize_line_endings(&file_content);

    let hunks = parse_unified_diff(&a.patch)?;
    let new_content = apply_hunks(&normalized, hunks)?;

    let new_file_content = if normalized.ends_with('\n') && !new_content.ends_with('\n') {
        restore_line_endings(&format!("{}\n", new_content), has_crlf)
    } else {
        restore_line_endings(&new_content, has_crlf)
    };

    write_file(gcx.clone(), &a.path, &new_file_content, dry).await?;
    sync_documents_ast(gcx.clone(), &a.path).await?;
    let chunks = convert_edit_to_diffchunks(a.path.clone(), &file_content, &new_file_content)?;
    let summary = edit_result_summary(&file_content, &new_file_content, &a.path);
    Ok((file_content, new_file_content, chunks, summary))
}

#[async_trait]
impl Tool for ToolApplyPatch {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let (gcx, code_workdir) = {
            let ccx_locked = ccx.lock().await;
            (ccx_locked.global_context.clone(), ccx_locked.code_workdir.clone())
        };
        let (_, _, chunks, _) = tool_apply_patch_exec(gcx, args, false, &code_workdir).await?;
        Ok((
            false,
            vec![ContextEnum::ChatMessage(ChatMessage {
                role: "diff".to_string(),
                content: ChatContent::SimpleText(json!(chunks).to_string()),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            })],
        ))
    }

    async fn match_against_confirm_deny(
        &self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        args: &HashMap<String, Value>,
    ) -> Result<MatchConfirmDeny, String> {
        let (gcx, code_workdir) = {
            let ccx_locked = ccx.lock().await;
            (ccx_locked.global_context.clone(), ccx_locked.code_workdir.clone())
        };
        let can_exec = parse_args(gcx, args, &code_workdir).await.is_ok();
        let msgs_len = ccx.lock().await.messages.len();
        if msgs_len != 0 && !can_exec {
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
            agentic: false,
            experimental: false,
            description: "Apply a unified diff patch to a file. Best for OpenAI models. Use standard diff format with @@ -line,count +line,count @@ headers.".to_string(),
            parameters: vec![
                ToolParam {
                    name: "path".to_string(),
                    description: "Absolute path to the file to patch.".to_string(),
                    param_type: "string".to_string(),
                },
                ToolParam {
                    name: "patch".to_string(),
                    description: "Unified diff patch. Example: @@ -10,3 +10,4 @@\\n context\\n-old line\\n+new line".to_string(),
                    param_type: "string".to_string(),
                },
            ],
            parameters_required: vec!["path".to_string(), "patch".to_string()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hunk_header_basic() {
        let (start, count) = parse_hunk_header("@@ -10,3 +10,4 @@").unwrap();
        assert_eq!(start, 10);
        assert_eq!(count, 3);
    }

    #[test]
    fn test_parse_hunk_header_single_line() {
        let (start, count) = parse_hunk_header("@@ -5 +5,2 @@").unwrap();
        assert_eq!(start, 5);
        assert_eq!(count, 1);
    }

    #[test]
    fn test_parse_hunk_header_insert_at_top() {
        let (start, count) = parse_hunk_header("@@ -0,0 +1,3 @@").unwrap();
        assert_eq!(start, 0);
        assert_eq!(count, 0);
    }

    #[test]
    fn test_parse_hunk_header_invalid_zero() {
        assert!(parse_hunk_header("@@ -0,5 +1,5 @@").is_err());
    }

    #[test]
    fn test_parse_unified_diff_basic() {
        let patch = "@@ -1,2 +1,2 @@\n old1\n-old2\n+new2";
        let hunks = parse_unified_diff(patch).unwrap();
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].old_start, 1);
        assert_eq!(hunks[0].old_lines, vec!["old1", "old2"]);
        assert_eq!(hunks[0].new_lines, vec!["old1", "new2"]);
    }

    #[test]
    fn test_parse_unified_diff_insert_at_top() {
        let patch = "@@ -0,0 +1,2 @@\n+line1\n+line2";
        let hunks = parse_unified_diff(patch).unwrap();
        assert_eq!(hunks.len(), 1);
        assert_eq!(hunks[0].old_start, 0);
        assert_eq!(hunks[0].old_count, 0);
        assert!(hunks[0].old_lines.is_empty());
        assert_eq!(hunks[0].new_lines, vec!["line1", "line2"]);
    }

    #[test]
    fn test_parse_unified_diff_with_headers() {
        let patch = "--- a/file.txt\n+++ b/file.txt\n@@ -1,1 +1,1 @@\n-old\n+new";
        let hunks = parse_unified_diff(patch).unwrap();
        assert_eq!(hunks.len(), 1);
    }

    #[test]
    fn test_parse_unified_diff_count_mismatch() {
        let patch = "@@ -1,5 +1,1 @@\n-old\n+new";
        assert!(parse_unified_diff(patch).is_err());
    }

    #[test]
    fn test_apply_hunks_basic() {
        let content = "line1\nold\nline3";
        let hunks = vec![Hunk {
            old_start: 2,
            old_count: 1,
            old_lines: vec!["old".to_string()],
            new_lines: vec!["new".to_string()],
        }];
        let result = apply_hunks(content, hunks).unwrap();
        assert_eq!(result, "line1\nnew\nline3");
    }

    #[test]
    fn test_apply_hunks_insert_at_top() {
        let content = "existing";
        let hunks = vec![Hunk {
            old_start: 0,
            old_count: 0,
            old_lines: vec![],
            new_lines: vec!["new1".to_string(), "new2".to_string()],
        }];
        let result = apply_hunks(content, hunks).unwrap();
        assert_eq!(result, "new1\nnew2\nexisting");
    }

    #[test]
    fn test_apply_hunks_mismatch() {
        let content = "line1\nactual\nline3";
        let hunks = vec![Hunk {
            old_start: 2,
            old_count: 1,
            old_lines: vec!["expected".to_string()],
            new_lines: vec!["new".to_string()],
        }];
        assert!(apply_hunks(content, hunks).is_err());
    }

    #[test]
    fn test_apply_hunks_out_of_bounds() {
        let content = "line1\nline2";
        let hunks = vec![Hunk {
            old_start: 10,
            old_count: 1,
            old_lines: vec!["x".to_string()],
            new_lines: vec!["y".to_string()],
        }];
        assert!(apply_hunks(content, hunks).is_err());
    }

    #[test]
    fn test_apply_hunks_multiple() {
        let content = "a\nb\nc\nd\ne";
        let hunks = vec![
            Hunk {
                old_start: 2,
                old_count: 1,
                old_lines: vec!["b".to_string()],
                new_lines: vec!["B".to_string()],
            },
            Hunk {
                old_start: 4,
                old_count: 1,
                old_lines: vec!["d".to_string()],
                new_lines: vec!["D".to_string()],
            },
        ];
        let result = apply_hunks(content, hunks).unwrap();
        assert_eq!(result, "a\nB\nc\nD\ne");
    }
}
