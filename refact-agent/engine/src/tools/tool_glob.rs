use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use glob::{MatchOptions, Pattern};
use serde_json::Value;
use tokio::sync::Mutex as AMutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::call_validation::{ChatContent, ChatMessage, ContextEnum};
use crate::files_correction::shortify_paths;
use crate::files_in_workspace::{check_file_privacy_with_context, prepare_file_read_context};
use crate::postprocessing::pp_command_output::OutputFilter;
use crate::privacy::{check_file_privacy, load_privacy_if_needed, FilePrivacyLevel};
use crate::tools::scope_utils::{
    format_scope_notices, resolve_scope_with_execution_scope_limited, validate_scope_files,
};
use crate::tools::tools_description::{
    json_schema_from_params, Tool, ToolDesc, ToolSource, ToolSourceType,
};

pub struct ToolGlob {
    pub config_path: String,
}

const DEFAULT_MAX_RESULTS: usize = 100;
const CAP_MAX_RESULTS: usize = 1000;
const MAX_GLOB_CANDIDATE_ENTRIES: usize = 20_000;

const ABORTED_ERROR: &str = "⚠️ glob aborted before completion (cancelled by caller).";

fn glob_match_options() -> MatchOptions {
    MatchOptions {
        case_sensitive: true,
        require_literal_separator: true,
        require_literal_leading_dot: false,
    }
}

fn path_matches_glob(pattern: &Pattern, path: &str, opts: &MatchOptions) -> bool {
    if pattern.matches_with(path, *opts) {
        return true;
    }
    let mut remainder = path;
    while let Some(idx) = remainder.find('/') {
        remainder = &remainder[idx + 1..];
        if pattern.matches_with(remainder, *opts) {
            return true;
        }
    }
    false
}

#[derive(Debug)]
struct GlobOutcome {
    matches: Vec<String>,
    total_matched: usize,
    truncated: bool,
}

fn run_glob(
    pattern: &Pattern,
    files_in_scope: &[String],
    max_results: usize,
    privacy_settings: Arc<crate::privacy::PrivacySettings>,
    abort_flag: &AtomicBool,
) -> Result<GlobOutcome, String> {
    let opts = glob_match_options();
    let mut matches: Vec<String> = Vec::new();

    for (idx, path) in files_in_scope.iter().enumerate() {
        if (idx & 0x3FF) == 0 && abort_flag.load(Ordering::Relaxed) {
            return Err(ABORTED_ERROR.to_string());
        }
        if !path_matches_glob(pattern, path, &opts) {
            continue;
        }
        if check_file_privacy(
            privacy_settings.clone(),
            Path::new(path),
            &FilePrivacyLevel::AllowToSendAnywhere,
        )
        .is_err()
        {
            continue;
        }
        matches.push(path.clone());
    }

    matches.sort();

    let total_matched = matches.len();
    let truncated = total_matched > max_results;
    if truncated {
        matches.truncate(max_results);
    }

    Ok(GlobOutcome {
        matches,
        total_matched,
        truncated,
    })
}

fn parse_max_results(args: &HashMap<String, Value>) -> Result<usize, String> {
    let value = match args.get("max_results") {
        None | Some(Value::Null) => return Ok(DEFAULT_MAX_RESULTS),
        Some(Value::Number(n)) => n
            .as_u64()
            .and_then(|v| usize::try_from(v).ok())
            .ok_or_else(|| "argument `max_results` must be a non-negative integer".to_string())?,
        Some(Value::String(s)) if s.trim().is_empty() => return Ok(DEFAULT_MAX_RESULTS),
        Some(Value::String(s)) => s
            .trim()
            .parse::<usize>()
            .map_err(|_| "argument `max_results` must be a non-negative integer".to_string())?,
        Some(v) => return Err(format!("argument `max_results` is not an integer: {:?}", v)),
    };
    Ok(value.clamp(1, CAP_MAX_RESULTS))
}

#[async_trait]
impl Tool for ToolGlob {
    fn tool_description(&self) -> ToolDesc {
        ToolDesc {
            name: "glob".to_string(),
            display_name: "Glob".to_string(),
            source: ToolSource {
                source_type: ToolSourceType::Builtin,
                config_path: self.config_path.clone(),
            },
            experimental: false,
            allow_parallel: true,
            description: "Find files whose PATHS match a glob pattern (e.g. `src/**/*.rs`, `*.toml`). Matches file paths only — it never opens or reads file contents. Use `regex_search` when you need to search inside files.".to_string(),
            input_schema: json_schema_from_params(
                &[
                    ("pattern", "string", "Glob pattern to match file paths, e.g. `**/*.rs`, `src/**/mod.rs`, `*.toml`. `*` matches within a path segment, `**` matches across directories."),
                    ("path", "string", "Optional scope: 'workspace' (default) for the whole workspace, 'dir/subdir/' (trailing slash) for a directory, or 'dir/file.ext' for a single file."),
                    ("max_results", "integer", "Maximum number of paths to return (default 100, hard cap 1000)."),
                ],
                &["pattern"],
            ),
            output_schema: None,
            annotations: None,
        }
    }

    async fn tool_execute(
        &mut self,
        ccx: Arc<AMutex<AtCommandsContext>>,
        tool_call_id: &String,
        args: &HashMap<String, Value>,
    ) -> Result<(bool, Vec<ContextEnum>), String> {
        let pattern_str = match args.get("pattern") {
            Some(Value::String(s)) if !s.trim().is_empty() => s.clone(),
            Some(Value::String(_)) | None => {
                return Err("Missing argument `pattern` in the `glob()` call.".to_string())
            }
            Some(v) => return Err(format!("argument `pattern` is not a string: {:?}", v)),
        };

        let scope = match args.get("path") {
            Some(Value::String(s)) if !s.trim().is_empty() => s.clone(),
            None | Some(Value::Null) | Some(Value::String(_)) => "workspace".to_string(),
            Some(v) => return Err(format!("argument `path` is not a string: {:?}", v)),
        };

        let max_results = parse_max_results(args)?;

        let pattern = Pattern::new(&pattern_str)
            .map_err(|e| format!("⚠️ Invalid glob pattern '{}': {}. 💡 Use `**` to match across directories and `*` within a segment.", pattern_str, e))?;

        let (gcx, execution_scope, abort_flag) = {
            let cgcx = ccx.lock().await;
            (
                cgcx.app.gcx.clone(),
                cgcx.execution_scope.clone(),
                cgcx.abort_flag.clone(),
            )
        };

        if abort_flag.load(Ordering::Relaxed) {
            return Err(ABORTED_ERROR.to_string());
        }

        let scoped_files = resolve_scope_with_execution_scope_limited(
            gcx.clone(),
            execution_scope.as_ref(),
            &scope,
            MAX_GLOB_CANDIDATE_ENTRIES,
            Some(&abort_flag),
        )
        .await?;
        let read_context = prepare_file_read_context(gcx.clone()).await;
        let files_in_scope = validate_scope_files(scoped_files.files, &scope)?
            .into_iter()
            .filter(|path| check_file_privacy_with_context(&read_context, Path::new(path)).is_ok())
            .collect::<Vec<_>>();
        let privacy_settings = load_privacy_if_needed(gcx.clone()).await;

        let outcome = run_glob(
            &pattern,
            &files_in_scope,
            max_results,
            privacy_settings,
            &abort_flag,
        )?;

        let mut content = format_scope_notices(&scoped_files.notices);
        content.push_str(&format!("Glob matches for pattern '{}':\n", pattern_str));

        if outcome.matches.is_empty() {
            content.push_str("  No files matched.\n");
            content.push_str("💡 Try a broader pattern (e.g. `**/*.rs`) or scope 'workspace'.\n");
        } else {
            let short_paths = shortify_paths(gcx.clone(), &outcome.matches).await;
            for (idx, path) in outcome.matches.iter().enumerate() {
                let display = short_paths.get(idx).unwrap_or(path);
                content.push_str(&format!("  {}\n", display));
            }
            if outcome.truncated {
                content.push_str(&format!(
                    "\n⚠️ Showing {} of {} matched paths (truncated). Raise max_results or narrow the pattern/scope for more.\n",
                    outcome.matches.len(),
                    outcome.total_matched
                ));
            }
        }

        let mut tool_message = ChatMessage {
            role: "tool".to_string(),
            content: ChatContent::SimpleText(content),
            tool_calls: None,
            tool_call_id: tool_call_id.clone(),
            output_filter: Some(OutputFilter::no_limits()),
            ..Default::default()
        };
        crate::privacy::load_privacy_if_needed(gcx.clone()).await;
        let records = crate::privacy::records::declared_file_records(
            &gcx,
            outcome.matches.iter().map(PathBuf::from),
        )?;
        crate::privacy::records::merge_records(&mut tool_message, records);
        let results = vec![ContextEnum::ChatMessage(tool_message)];

        Ok((false, results))
    }

    fn tool_depends_on(&self) -> Vec<String> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::AtomicBool;

    fn permissive_privacy() -> Arc<crate::privacy::PrivacySettings> {
        Arc::new(crate::privacy::PrivacySettings {
            privacy_rules: crate::privacy::FilePrivacySettings {
                only_send_to_servers_I_control: Vec::new(),
                blocked: Vec::new(),
            },
            loaded_ts: u64::MAX / 2,
        })
    }

    fn pat(p: &str) -> Pattern {
        Pattern::new(p).unwrap()
    }

    #[test]
    fn matches_by_filename_at_any_depth() {
        let opts = glob_match_options();
        let p = pat("*.rs");
        assert!(path_matches_glob(&p, "/root/src/deep/lib.rs", &opts));
        assert!(path_matches_glob(&p, "main.rs", &opts));
        assert!(!path_matches_glob(&p, "/root/src/lib.txt", &opts));
    }

    #[test]
    fn star_does_not_cross_path_separator_but_doublestar_does() {
        let opts = glob_match_options();
        assert!(!pat("src/*.rs").matches_with("src/a/b.rs", opts));
        assert!(pat("src/*.rs").matches_with("src/b.rs", opts));
        assert!(pat("src/**/*.rs").matches_with("src/a/b/c.rs", opts));
    }

    #[test]
    fn content_only_nonmatch_is_ignored() {
        let temp = tempfile::Builder::new()
            .prefix("refact-glob-content-")
            .tempdir()
            .unwrap();
        let file = temp.path().join("notes.txt");
        fs::write(&file, "this file mentions needle.rs inside its body").unwrap();

        let files = vec![file.to_string_lossy().to_string()];
        let outcome = run_glob(
            &pat("*.rs"),
            &files,
            100,
            permissive_privacy(),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(
            outcome.matches.is_empty(),
            "path-only glob must not match a .txt file even if its contents mention .rs"
        );
    }

    #[test]
    fn respects_scope_containment() {
        let files = vec!["/scope/a/one.rs".to_string(), "/scope/a/two.rs".to_string()];
        let outcome = run_glob(
            &pat("**/*.rs"),
            &files,
            100,
            permissive_privacy(),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(
            outcome.matches,
            vec!["/scope/a/one.rs".to_string(), "/scope/a/two.rs".to_string()]
        );
    }

    #[test]
    fn privacy_blocks_paths_from_output() {
        let privacy = Arc::new(crate::privacy::PrivacySettings {
            privacy_rules: crate::privacy::FilePrivacySettings {
                only_send_to_servers_I_control: Vec::new(),
                blocked: vec!["**/*.pem".to_string()],
            },
            loaded_ts: u64::MAX / 2,
        });
        let files = vec![
            "/scope/keep.rs".to_string(),
            "/scope/secret.pem".to_string(),
        ];
        let outcome =
            run_glob(&pat("**/*"), &files, 100, privacy, &AtomicBool::new(false)).unwrap();
        assert!(outcome.matches.contains(&"/scope/keep.rs".to_string()));
        assert!(
            !outcome.matches.iter().any(|p| p.ends_with(".pem")),
            "privacy-blocked path must be excluded from glob output"
        );
    }

    #[test]
    fn oversized_files_are_not_read() {
        let temp = tempfile::Builder::new()
            .prefix("refact-glob-oversized-")
            .tempdir()
            .unwrap();
        let file = temp.path().join("huge.rs");
        fs::write(&file, vec![b'a'; 8 * 1024 * 1024]).unwrap();
        let files = vec![file.to_string_lossy().to_string()];
        let outcome = run_glob(
            &pat("*.rs"),
            &files,
            100,
            permissive_privacy(),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(outcome.matches.len(), 1, "oversized file matched by path");
    }

    #[test]
    fn truncation_is_reported() {
        let files: Vec<String> = (0..10).map(|i| format!("/scope/file_{i:02}.rs")).collect();
        let outcome = run_glob(
            &pat("**/*.rs"),
            &files,
            3,
            permissive_privacy(),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert_eq!(outcome.matches.len(), 3);
        assert_eq!(outcome.total_matched, 10);
        assert!(outcome.truncated);
        assert_eq!(
            outcome.matches,
            vec![
                "/scope/file_00.rs".to_string(),
                "/scope/file_01.rs".to_string(),
                "/scope/file_02.rs".to_string()
            ]
        );
    }

    #[test]
    fn respects_preset_abort_flag() {
        let files: Vec<String> = (0..2000).map(|i| format!("/s/f{i}.rs")).collect();
        let result = run_glob(
            &pat("**/*.rs"),
            &files,
            100,
            permissive_privacy(),
            &AtomicBool::new(true),
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("aborted"));
    }

    #[test]
    fn parse_max_results_clamps_and_defaults() {
        let default = HashMap::new();
        assert_eq!(parse_max_results(&default).unwrap(), DEFAULT_MAX_RESULTS);
        let high = HashMap::from_iter([("max_results".to_string(), Value::from(100000))]);
        assert_eq!(parse_max_results(&high).unwrap(), CAP_MAX_RESULTS);
        let zero = HashMap::from_iter([("max_results".to_string(), Value::from(0))]);
        assert_eq!(parse_max_results(&zero).unwrap(), 1);
        let bad =
            HashMap::from_iter([("max_results".to_string(), Value::String("lots".to_string()))]);
        assert!(parse_max_results(&bad).is_err());
    }
}
