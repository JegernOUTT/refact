use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use serde_json::Value;
use tokio::fs;
use async_trait::async_trait;
use tokio::sync::Mutex as AMutex;
use serde_json::json;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::at_commands::at_file::return_one_candidate_or_a_good_error;
use crate::call_validation::{ChatMessage, ChatContent, ContextEnum, DiffChunk};
use crate::files_correction::{
    canonical_path, correct_to_nearest_dir_path, correct_to_nearest_filename, get_project_dirs,
    preprocess_path_for_normalization,
};
use crate::files_in_workspace::get_file_text_from_memory_or_disk;
use crate::tools::tools_description::{
    MatchConfirmDeny, MatchConfirmDenyResult, Tool, ToolDesc, ToolSource, ToolSourceType,
    json_schema_from_params,
};
use crate::integrations::integr_abstract::IntegrationConfirmation;
use crate::privacy::{FilePrivacyLevel, load_privacy_if_needed, check_file_privacy};
use crate::tools::file_edit::auxiliary::resolve_path_with_scope;

pub struct ToolMv {
    pub config_path: String,
}

impl ToolMv {
    fn preformat_path(path: &String) -> String {
        let trimmed = path.trim_end_matches(&['/', '\\'][..]);
        if trimmed.is_empty() {
            path.clone()
        } else {
            trimmed.to_string()
        }
    }

    // Parse the overwrite flag.
    fn parse_overwrite(args: &HashMap<String, Value>) -> Result<bool, String> {
        match args.get("overwrite") {
            Some(Value::Bool(b)) => Ok(*b),
            Some(Value::String(s)) => {
                let lower = s.to_lowercase();
                Ok(lower == "true")
            }
            None => Ok(false),
            Some(other) => Err(format!("Expected boolean for 'overwrite', got {:?}", other)),
        }
    }
}

#[async_trait]
impl Tool for ToolMv {
    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let src_str = match args.get("source") {
            Some(Value::String(s)) if !s.trim().is_empty() => {
                Self::preformat_path(&s.trim().to_string())
            }
            _ => return Err("Missing required argument `source`".to_string()),
        };
        let dst_str = match args.get("destination") {
            Some(Value::String(s)) if !s.trim().is_empty() => {
                Self::preformat_path(&s.trim().to_string())
            }
            _ => return Err("Missing required argument `destination`".to_string()),
        };
        let src_str = preprocess_path_for_normalization(src_str);
        let dst_str = preprocess_path_for_normalization(dst_str);
        let overwrite = Self::parse_overwrite(args)?;

        let (gcx, execution_scope, top_n) = {
            let ccx_locked = ccx.lock().await;
            (
                ccx_locked.global_context.clone(),
                ccx_locked.execution_scope.clone(),
                ccx_locked.top_n,
            )
        };

        let privacy_settings = load_privacy_if_needed(gcx.clone()).await;
        let scoped_src = resolve_path_with_scope(
            Path::new(&src_str),
            privacy_settings.clone(),
            execution_scope.as_ref(),
            true,
        )
        .transpose()?;
        let scoped_dst = resolve_path_with_scope(
            Path::new(&dst_str),
            privacy_settings.clone(),
            execution_scope.as_ref(),
            false,
        )
        .transpose()?;

        let (
            src_corrected_path,
            dst_corrected_path,
            src_true_path,
            dst_true_path,
            src_is_dir,
            scope_warnings,
            corrections,
        ) = if let (Some(src), Some(dst)) = (scoped_src, scoped_dst) {
            let mut scope_warnings = src.warnings;
            scope_warnings.extend(dst.warnings);
            let src_corrected_path = src.path.to_string_lossy().to_string();
            let dst_corrected_path = dst.path.to_string_lossy().to_string();
            let src_is_dir = src.path.is_dir();
            let corrections = src_str != src_corrected_path
                || dst_str != dst_corrected_path
                || !scope_warnings.is_empty();
            (
                src_corrected_path,
                dst_corrected_path,
                src.path,
                dst.path,
                src_is_dir,
                scope_warnings,
                corrections,
            )
        } else {
            let project_dirs = get_project_dirs(gcx.clone()).await;

            let src_file_candidates =
                correct_to_nearest_filename(gcx.clone(), &src_str, false, top_n).await;
            let src_dir_candidates =
                correct_to_nearest_dir_path(gcx.clone(), &src_str, false, top_n).await;
            let (src_corrected_path, src_is_dir) = if !src_file_candidates.is_empty() {
                (
                    return_one_candidate_or_a_good_error(
                        gcx.clone(),
                        &src_str,
                        &src_file_candidates,
                        &project_dirs,
                        false,
                    )
                    .await?,
                    false,
                )
            } else if !src_dir_candidates.is_empty() {
                (
                    return_one_candidate_or_a_good_error(
                        gcx.clone(),
                        &src_str,
                        &src_dir_candidates,
                        &project_dirs,
                        true,
                    )
                    .await?,
                    true,
                )
            } else {
                return Err(format!(
                    "⚠️ Source '{}' not found. 💡 Use tree() to explore or check spelling",
                    src_str
                ));
            };

            let dst_parent = if let Some(p) = std::path::Path::new(&dst_str).parent() {
                if cfg!(target_os = "windows") {
                    p.to_string_lossy().replace("/", "\\")
                } else {
                    p.to_string_lossy().to_string()
                }
            } else {
                dst_str.clone()
            };

            let dst_dir_candidates =
                correct_to_nearest_dir_path(gcx.clone(), &dst_parent, false, top_n).await;
            let dst_parent_path = if !dst_dir_candidates.is_empty() {
                return_one_candidate_or_a_good_error(
                    gcx.clone(),
                    &dst_parent,
                    &dst_dir_candidates,
                    &project_dirs,
                    true,
                )
                .await?
            } else {
                return Err(format!(
                    "⚠️ Destination directory '{}' not found. 💡 Use tree() to find valid path",
                    dst_parent
                ));
            };

            let dst_name = std::path::Path::new(&dst_str)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or(dst_str.clone());
            let dst_corrected_path = std::path::PathBuf::from(&dst_parent_path)
                .join(&dst_name)
                .to_string_lossy()
                .to_string();

            let src_true_path = canonical_path(&src_corrected_path);
            let dst_true_path = canonical_path(&dst_corrected_path);

            if let Err(e) = check_file_privacy(
                privacy_settings.clone(),
                &src_true_path,
                &FilePrivacyLevel::AllowToSendAnywhere,
            ) {
                return Err(format!("Cannot move '{}': {}", src_str, e));
            }
            if let Err(e) = check_file_privacy(
                privacy_settings.clone(),
                &dst_true_path,
                &FilePrivacyLevel::AllowToSendAnywhere,
            ) {
                return Err(format!("Cannot move to '{}': {}", src_str, e));
            }

            let src_within_project = project_dirs.iter().any(|p| src_true_path.starts_with(p));
            let dst_within_project = project_dirs.iter().any(|p| dst_true_path.starts_with(p));
            if !src_within_project && !gcx.read().await.cmdline.inside_container {
                return Err(format!(
                    "⚠️ Source '{}' is outside project. 💡 mv() only works within workspace",
                    src_str
                ));
            }
            if !dst_within_project && !gcx.read().await.cmdline.inside_container {
                return Err(format!(
                    "⚠️ Destination '{}' is outside project. 💡 mv() only works within workspace",
                    dst_str
                ));
            }

            let corrections = src_str != src_corrected_path || dst_str != dst_corrected_path;
            (
                src_corrected_path,
                dst_corrected_path,
                src_true_path,
                dst_true_path,
                src_is_dir,
                Vec::new(),
                corrections,
            )
        };

        let _src_metadata = fs::symlink_metadata(&src_true_path).await.map_err(|e| {
            format!(
                "⚠️ Cannot access '{}': {}. 💡 Check file exists and permissions",
                src_str, e
            )
        })?;

        let mut src_file_content = String::new();
        if !src_is_dir {
            src_file_content =
                get_file_text_from_memory_or_disk(gcx.clone(), &src_true_path).await?;
        }
        let mut dst_file_content = String::new();
        if let Ok(dst_metadata) = fs::metadata(&dst_true_path).await {
            if !overwrite {
                return Err(format!("⚠️ Destination '{}' exists. 💡 Use mv(source:'{}', destination:'{}', overwrite:true)", dst_str, src_str, dst_str));
            }
            if dst_metadata.is_dir() {
                fs::remove_dir_all(&dst_true_path).await.map_err(|e| {
                    format!("Failed to remove existing directory '{}': {}", dst_str, e)
                })?;
                // Invalidate cache entries for all files under the removed directory
                {
                    let mut gcx_write = gcx.write().await;
                    let paths_to_remove: Vec<_> = gcx_write
                        .documents_state
                        .memory_document_map
                        .keys()
                        .filter(|p| p.starts_with(&dst_true_path))
                        .cloned()
                        .collect();
                    for p in paths_to_remove {
                        gcx_write.documents_state.memory_document_map.remove(&p);
                    }
                }
            } else {
                if !dst_metadata.is_dir() {
                    dst_file_content = fs::read_to_string(&dst_true_path)
                        .await
                        .unwrap_or_else(|_| "".to_string());
                }
                fs::remove_file(&dst_true_path)
                    .await
                    .map_err(|e| format!("Failed to remove existing file '{}': {}", dst_str, e))?;
                // Invalidate cache entry for the removed file
                gcx.write()
                    .await
                    .documents_state
                    .memory_document_map
                    .remove(&dst_true_path);
            }
        }

        if let Some(parent) = dst_true_path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent).await.map_err(|e| {
                    format!("Failed to create parent directory for '{}': {}", dst_str, e)
                })?;
            }
            let parent_metadata = fs::metadata(parent)
                .await
                .map_err(|e| format!("Failed to check parent directory permissions: {}", e))?;
            if parent_metadata.permissions().readonly() {
                return Err(format!(
                    "No write permission to parent directory of '{}'",
                    dst_str
                ));
            }
        }

        fs::rename(&src_true_path, &dst_true_path)
            .await
            .map_err(|e| {
                format!(
                    "⚠️ Failed to move '{}' to '{}': {}. 💡 Check permissions and paths",
                    src_str, dst_str, e
                )
            })?;

        {
            let mut gcx_write = gcx.write().await;
            gcx_write
                .documents_state
                .memory_document_map
                .remove(&src_true_path);
            gcx_write
                .documents_state
                .memory_document_map
                .remove(&dst_true_path);
        }

        let mut messages = vec![];
        if !scope_warnings.is_empty() {
            messages.push(ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(scope_warnings.join("\n")),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            }));
        }

        if src_is_dir {
            messages.push(ContextEnum::ChatMessage(ChatMessage {
                role: "tool".to_string(),
                content: ChatContent::SimpleText(format!(
                    "Moved directory '{}' to '{}'",
                    src_corrected_path, dst_corrected_path
                )),
                tool_calls: None,
                tool_call_id: tool_call_id.clone(),
                ..Default::default()
            }));
        } else if !src_file_content.is_empty() {
            let diff_chunk = DiffChunk {
                file_name: src_corrected_path.clone(),
                file_action: "rename".to_string(),
                line1: 1,
                line2: src_file_content.lines().count(),
                lines_remove: src_file_content.clone(),
                lines_add: "".to_string(),
                file_name_rename: Some(dst_corrected_path.clone()),
                is_file: true,
                application_details: format!(
                    "File {} from '{}' to '{}'",
                    if src_true_path.parent() == dst_true_path.parent() {
                        "renamed"
                    } else {
                        "moved"
                    },
                    src_corrected_path,
                    dst_corrected_path
                ),
            };
            if !dst_file_content.is_empty() {
                let dst_diff_chunk = DiffChunk {
                    file_name: dst_corrected_path.clone(),
                    file_action: "edit".to_string(),
                    line1: 1,
                    line2: dst_file_content.lines().count(),
                    lines_remove: dst_file_content.clone(),
                    lines_add: src_file_content.clone(),
                    file_name_rename: None,
                    is_file: true,
                    application_details: format!(
                        "`{}` replaced with `{}`",
                        dst_corrected_path, src_corrected_path
                    ),
                };
                messages.push(ContextEnum::ChatMessage(ChatMessage {
                    role: "diff".to_string(),
                    content: ChatContent::SimpleText(
                        json!([diff_chunk, dst_diff_chunk]).to_string(),
                    ),
                    tool_calls: None,
                    tool_call_id: tool_call_id.clone(),
                    ..Default::default()
                }));
            } else {
                messages.push(ContextEnum::ChatMessage(ChatMessage {
                    role: "diff".to_string(),
                    content: ChatContent::SimpleText(json!([diff_chunk]).to_string()),
                    tool_calls: None,
                    tool_call_id: tool_call_id.clone(),
                    ..Default::default()
                }));
            }
        }

        Ok((corrections, messages))
    }

    async fn command_to_match_against_confirm_deny(
        &self,
        _ccx: Arc<AMutex<AtCommandsContext>>,
        args: &HashMap<String, Value>,
    ) -> Result<String, String> {
        let src = match args.get("source") {
            Some(Value::String(s)) if !s.trim().is_empty() => s.trim().to_string(),
            _ => return Ok("".to_string()),
        };
        let dst = match args.get("destination") {
            Some(Value::String(s)) if !s.trim().is_empty() => s.trim().to_string(),
            _ => return Ok("".to_string()),
        };
        let overwrite = Self::parse_overwrite(args).unwrap_or(false);
        Ok(format!(
            "mv {} {} {}",
            if overwrite { "--force" } else { "" },
            src,
            dst
        ))
    }

    fn confirm_deny_rules(&self) -> Option<IntegrationConfirmation> {
        Some(IntegrationConfirmation {
            ask_user: vec!["*".to_string()],
            deny: vec![],
        })
    }

    async fn match_against_confirm_deny(
        &self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        args: &HashMap<String, Value>,
    ) -> Result<MatchConfirmDeny, String> {
        let command_to_match = self
            .command_to_match_against_confirm_deny(ccx.clone(), &args)
            .await
            .map_err(|e| format!("Error getting tool command to match: {}", e))?;
        Ok(MatchConfirmDeny {
            result: MatchConfirmDenyResult::CONFIRMATION,
            command: command_to_match,
            rule: "default".to_string(),
        })
    }

    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "mv".to_string(),
            display_name: "mv".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: false,
            description: "Moves or renames files and directories. If a simple rename fails due to a cross-device error and the source is a file, it falls back to copying and deleting. Use overwrite=true to replace an existing target.".to_string(),
            input_schema: json_schema_from_params(&[("source", "string", "Path of the file or directory to move."), ("destination", "string", "Target path where the file or directory should be placed."), ("overwrite", "boolean", "If true and target exists, replace it. Defaults to false.")], &["source", "destination"]),
            output_schema: None,
            annotations: None,
        }
    }
}
