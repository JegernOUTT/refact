use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use refact_chat_api::{PythonCallRecord, PythonCallStatus};
use refact_git::script::{GitContext, GitError, GitOutput, GitRequest};
use refact_python_kernel::protocol::{HostResult, PathDenyRules};
use serde_json::{json, Value};

use crate::ext::hooks_runner::ControlVerdict;
use crate::files_correction::RegisteredWorktreePathMapping;

use super::dispatcher::{NestedDispatcher, NestedOutcome};
use super::engine_fns::{failed_envelope, fit_for_script};

type Raise = (&'static str, String, bool);

fn cancelled() -> Raise {
    ("RuntimeError", "python run cancelled".into(), true)
}

/// What the run card and the hooks see of a call: the checked arguments and a git-like line.
fn preview_args(request: &GitRequest) -> Value {
    let mut args = request.to_kwargs();
    args["command"] = json!(request.command_line());
    args
}

fn comparable(path: &Path) -> PathBuf {
    if cfg!(windows) || cfg!(target_os = "macos") {
        PathBuf::from(path.to_string_lossy().to_lowercase())
    } else {
        path.to_path_buf()
    }
}

/// The run's read rules with every path in its written and its resolved spelling.
struct WrittenRules {
    allow: Vec<PathBuf>,
    prefixes: Vec<PathBuf>,
    files: Vec<PathBuf>,
}

impl WrittenRules {
    fn new(rules: &PathDenyRules, root: &Path) -> Self {
        let spellings = |paths: &[PathBuf]| {
            paths
                .iter()
                .map(|path| root.join(path))
                .flat_map(|path| {
                    let resolved = dunce::canonicalize(&path).unwrap_or_else(|_| path.clone());
                    [comparable(&path), comparable(&resolved)]
                })
                .collect()
        };
        Self {
            allow: spellings(&rules.allow_prefixes),
            prefixes: spellings(&rules.prefixes),
            files: spellings(&rules.files),
        }
    }

    /// Whether the rules deny `path` as written, without following a link in it.
    fn deny(&self, path: &Path) -> bool {
        let path = comparable(path);
        !self.allow.iter().any(|allowed| path.starts_with(allowed))
            && (self.prefixes.iter().any(|prefix| path.starts_with(prefix))
                || self.files.contains(&path))
    }
}

/// `path` and its spellings in registered worktrees, as written.
fn written_aliases(path: &Path, mappings: &[RegisteredWorktreePathMapping]) -> Vec<PathBuf> {
    let mut aliases = vec![path.to_path_buf()];
    for mapping in mappings {
        if let Ok(rest) = path.strip_prefix(&mapping.root) {
            aliases.push(mapping.source_root.join(rest));
        }
        if let Ok(rest) = path.strip_prefix(&mapping.source_root) {
            aliases.push(mapping.root.join(rest));
        }
    }
    aliases
}

impl NestedDispatcher {
    /// `git()`: read-only git data of the repository that holds PROJECT_ROOT, under the hook
    /// name `git`, the mode's deny rules, the nested-call budget and the parallel-call permit.
    pub async fn dispatch_git(
        &self,
        seq: u32,
        kwargs: Value,
        too_large: Option<String>,
        site: (Option<u32>, Option<u32>),
    ) -> NestedOutcome {
        let start = std::time::Instant::now();
        let parsed = GitRequest::parse(&kwargs);
        let preview = match &parsed {
            Ok(request) => preview_args(request),
            Err(_) => kwargs.clone(),
        };
        let mut record = self.new_record(seq, refact_python_catalog::GIT_HELPER, &preview, site);
        self.config.sink.call_started(&record);
        let mut logged_args = preview;
        let outcome = self
            .git_inner(parsed, too_large.as_deref(), &mut record, &mut logged_args)
            .await;
        let (result, privacy, stop_run) = match outcome {
            Ok((value, privacy)) => {
                let ok = value["ok"] == true;
                record.status = if ok {
                    PythonCallStatus::Ok
                } else {
                    PythonCallStatus::Failed
                };
                record.summary = super::envelope::preview(
                    value[if ok { "text" } else { "error" }]
                        .as_str()
                        .unwrap_or(""),
                );
                (HostResult::Ok(value), privacy, false)
            }
            Err((exc_type, message, stopped)) => {
                record.status = if stopped {
                    PythonCallStatus::Cancelled
                } else if exc_type == "PermissionError" {
                    PythonCallStatus::Denied
                } else {
                    PythonCallStatus::Failed
                };
                record.summary = super::envelope::preview(&message);
                (
                    HostResult::Err {
                        exc_type: exc_type.into(),
                        message,
                    },
                    Vec::new(),
                    stopped,
                )
            }
        };
        self.finish_call(
            seq,
            record,
            start,
            &logged_args,
            None,
            result,
            privacy,
            Vec::new(),
            stop_run,
        )
        .await
    }

    async fn git_inner(
        &self,
        parsed: Result<GitRequest, String>,
        too_large: Option<&str>,
        record: &mut PythonCallRecord,
        logged_args: &mut Value,
    ) -> Result<(Value, Vec<Value>), Raise> {
        let c = &self.config;
        if let Some(size) = too_large {
            return Err((
                "TypeError",
                format!("git() arguments are too large to send: {size}; pass smaller values"),
                false,
            ));
        }
        let mut request = parsed.map_err(|error| ("TypeError", error, false))?;
        if !self.charge_nested_call() {
            return Err(("RuntimeError", "nested call budget exhausted".into(), false));
        }
        let rewritten = tokio::select! {
            biased;
            _ = c.cancel.cancelled() => return Err(cancelled()),
            verdict = self.git_pre_hook(&request, &record.call_id) => verdict?,
        };
        if let Some(rewritten) = rewritten {
            request = rewritten;
            *logged_args = preview_args(&request);
            record.args_preview =
                super::approvals::args_preview(&logged_args.to_string(), 1000, 200);
        }
        if let Some(rule) = self
            .engine_deny_rule(refact_python_catalog::GIT_HELPER)
            .await
        {
            return Err(("PermissionError", rule, false));
        }
        let gcx = &c.app.gcx;
        let privacy_error = |error| ("PermissionError", error, false);
        let read_context = crate::files_in_workspace::prepare_file_read_context(gcx.clone()).await;
        let settings = crate::privacy::load_privacy_if_needed(gcx.clone()).await;
        let privacy = crate::privacy::LegacyPrivacyMatcher::new(&settings);
        let snapshot =
            crate::privacy::classification_cache::snapshot(gcx).map_err(privacy_error)?;
        let written = WrittenRules::new(&c.deny_read, &c.project_root);
        let hostfs =
            refact_python_kernel::hostfs::HostFs::new(refact_python_kernel::hostfs::HostFsConfig {
                deny_read: c.deny_read.clone(),
                project_root: c.project_root.clone(),
                ..Default::default()
            });
        let permit = tokio::select! {
            biased;
            _ = c.cancel.cancelled() => return Err(cancelled()),
            permit = self.parallel.clone().acquire_owned() => permit.map_err(|_| ("RuntimeError", "dispatcher closed".to_string(), true))?,
        };
        let stop = Arc::new(AtomicBool::new(false));
        let mut task = tokio::task::spawn_blocking({
            let stop = stop.clone();
            let gcx = gcx.clone();
            let snapshot = snapshot.clone();
            let root = c.project_root.clone();
            let folders = c.workspace_folders.clone();
            move || {
                let _permit = permit;
                snapshot.finish(&gcx).map_err(GitError::Denied)?;
                let may_read = |path: &Path| {
                    hostfs.may_read(path)
                        && crate::files_in_workspace::check_file_privacy_for_read_preparation_with_context(
                            &read_context,
                            path,
                        )
                        .is_ok()
                        && !written.deny(path)
                        && written_aliases(path, &snapshot.mappings)
                            .iter()
                            .all(|alias| privacy.check_for_read_preparation(alias).is_ok())
                        && {
                            let zone = snapshot.classify_for_read_admission(&gcx, path);
                            zone.name != "blocked" && !zone.send_to.is_empty()
                        }
                };
                refact_git::script::run(
                    &request,
                    &GitContext {
                        root: &root,
                        folders: &folders,
                        may_read: &may_read,
                        cancel: &stop,
                    },
                )
            }
        });
        let joined = tokio::select! {
            biased;
            _ = c.cancel.cancelled() => {
                stop.store(true, Ordering::Release);
                let _ = (&mut task).await;
                return Err(cancelled());
            }
            joined = &mut task => joined,
        };
        let output: GitOutput = match joined {
            Err(error) => {
                return Err(("RuntimeError", format!("git() failed: {error}"), false));
            }
            Ok(Err(GitError::Type(message))) => return Err(("TypeError", message, false)),
            Ok(Err(GitError::Denied(message))) => return Err(("PermissionError", message, false)),
            Ok(Err(GitError::Cancelled)) => return Err(cancelled()),
            Ok(Err(GitError::Failed(message))) => {
                snapshot.finish(gcx).map_err(privacy_error)?;
                return Ok((failed_envelope(&message), Vec::new()));
            }
            Ok(Ok(output)) => output,
        };
        #[cfg(test)]
        crate::privacy::classification_cache::observe(gcx, "git-output");
        let value = fit_for_script(
            "git()",
            json!({"ok":true,"data":output.data,"text":output.summary,"files":[],"diffs":[],"images":[],"images_dropped":0,"error":null,"truncated":output.truncated}),
            c.max_nested_result_bytes,
            false,
        );
        if value["ok"] != true || output.paths.is_empty() {
            snapshot.finish(gcx).map_err(privacy_error)?;
            return Ok((value, Vec::new()));
        }
        let derived = c.ccx.lock().await.derived_privacy_zones.clone();
        let records = crate::privacy::records::observed_file_records_with_derived(
            gcx,
            output.paths,
            &derived,
        )
        .map_err(|error| {
            (
                "PermissionError",
                format!(
                    "git(): privacy records of the files it read could not be verified: {error}"
                ),
                false,
            )
        })?;
        let privacy = if records.is_empty() {
            Vec::new()
        } else {
            vec![json!(refact_privacy::PrivacyRecord {
                files: records
            })]
        };
        snapshot.finish(gcx).map_err(privacy_error)?;
        Ok((value, privacy))
    }

    /// `PreToolUse` with tool `git` and the checked arguments; a rewrite is checked again.
    async fn git_pre_hook(
        &self,
        request: &GitRequest,
        call_id: &str,
    ) -> Result<Option<GitRequest>, Raise> {
        match self
            .pre_tool_use(
                refact_python_catalog::GIT_HELPER,
                request.to_kwargs(),
                call_id,
            )
            .await
        {
            ControlVerdict::Allow => Ok(None),
            ControlVerdict::Deny(reason) => Err(("PermissionError", reason, false)),
            ControlVerdict::Rewrite(value) if value.is_object() => {
                GitRequest::parse(&value).map(Some).map_err(|error| {
                    (
                        "PermissionError",
                        format!(
                            "a PreToolUse hook rewrote git() into a call that is refused: {error}"
                        ),
                        false,
                    )
                })
            }
            ControlVerdict::Rewrite(_) => Err((
                "PermissionError",
                "hook rewrite must be a JSON object of git() arguments".into(),
                false,
            )),
        }
    }
}

#[cfg(test)]
#[path = "git_tests.rs"]
mod tests;
