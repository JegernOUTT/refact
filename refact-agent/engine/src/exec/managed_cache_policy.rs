use std::collections::BTreeMap;
#[cfg(target_os = "linux")]
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::at_commands::at_commands::AtCommandsContext;
use crate::exec::types::ExecSandboxMode;
use crate::exec::ExecSpawnRequest;
use crate::tools::command_checkout::ValidatedCommandCheckout;
use crate::worktrees::service::WorktreeService;
use crate::worktrees::types::{CacheComponentReport, CacheOutcome, CacheReason, WorktreeMeta};
use refact_worktrees::cache::CacheExecutionContext;

const REFUSED: &str = "managed cache execution inputs changed; request command approval again";
const MAX_INPUT_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Copy)]
enum ValidationPurpose<'a> {
    Execution,
    Pending(&'a crate::agents::command_approval::PendingCommandValidation),
}

pub(crate) struct ManagedCacheDecision {
    context: Option<Arc<CacheExecutionContext>>,
    meta: WorktreeMeta,
    identity: Value,
    inputs: String,
    checkout_binding: String,
    binding: String,
    execution: String,
    environment: String,
    target: Option<PathBuf>,
    report: Value,
    checkout: bool,
    executable: bool,
}

#[cfg(test)]
tokio::task_local! {
    pub(crate) static BEFORE_SPAWN: std::cell::RefCell<Option<Box<dyn FnOnce() + Send>>>;
}

fn component(outcome: CacheOutcome, reason: CacheReason) -> CacheComponentReport {
    CacheComponentReport {
        outcome,
        reason,
        key_ref: None,
    }
}

fn environment_binding(env: &std::collections::HashMap<String, String>) -> String {
    let ordered: BTreeMap<_, _> = env.iter().collect();
    crate::file_activity::digest_of(&json!(ordered).to_string())
}

fn target_policy(request: &ExecSpawnRequest, target: &Path) -> Result<(), CacheReason> {
    if request.env.keys().any(|name| {
        matches!(
            name.as_str(),
            "CARGO_TARGET_DIR"
                | "CARGO_BUILD_TARGET_DIR"
                | "CARGO_HOME"
                | "RUSTC_WRAPPER"
                | "RUSTC_WORKSPACE_WRAPPER"
                | "CARGO_BUILD_RUSTC_WRAPPER"
                | "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER"
        ) || name.starts_with("SCCACHE_")
    }) {
        return Err(CacheReason::InputMismatch);
    }
    if [
        "--target-dir",
        "--config",
        "CARGO_TARGET_DIR=",
        "CARGO_BUILD_TARGET_DIR=",
        "RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER",
        "SCCACHE_",
    ]
    .iter()
    .any(|key| request.command.contains(key))
    {
        return Err(CacheReason::InputMismatch);
    }
    if let Some(sandbox) = &request.sandbox {
        if sandbox.mode == ExecSandboxMode::ReadOnly
            || !sandbox.rw_paths.iter().any(|root| target.starts_with(root))
        {
            return Err(CacheReason::UnverifiedIsolation);
        }
    }
    Ok(())
}

async fn accepted_identity(
    ccx: &Arc<Mutex<AtCommandsContext>>,
    meta: &WorktreeMeta,
    checkout: bool,
    purpose: ValidationPurpose<'_>,
) -> Result<Value, String> {
    refuse_nested_task_command(ccx, meta).await?;
    if let ValidationPurpose::Pending(proof) = purpose {
        proof.check(ccx).await?;
    }
    let (app, chat, scope, agent, model, depth) = {
        let context = ccx.lock().await;
        if context
            .abort_flag
            .load(std::sync::atomic::Ordering::Relaxed)
            || context
                .app
                .runtime
                .shutdown_flag
                .load(std::sync::atomic::Ordering::Relaxed)
            || context
                .app
                .agent_executions
                .get(&context.chat_id)
                .is_some_and(|run| run.is_cancelled())
        {
            return Err(REFUSED.into());
        }
        (
            context.app.clone(),
            context.chat_id.clone(),
            context.execution_scope.clone(),
            context.background_agent_id.clone(),
            context.current_model.clone(),
            context.subchat_depth,
        )
    };
    let policy = crate::coordination::privacy_guard::policy_key(
        &app.gcx,
        &crate::caps::BaseModelRecord {
            id: model.clone(),
            ..Default::default()
        },
    )
    .map_err(|_| REFUSED)?;
    let mode = crate::exec::command_policy::chat_mode_for_exec(app.gcx.clone(), &chat).await;
    let boundary = json!({"model":model,"policy":policy,"terminal":app.gcx.terminal_security_config,"mode":mode});
    if checkout {
        return Ok(json!({"checkout": meta,"boundary":boundary}));
    }
    if scope.as_ref().map(|scope| scope.worktree()) != Some(meta) || !meta.enforce {
        return Err(REFUSED.into());
    }
    if let Some(agent) = agent {
        let config = app.agent_executions.saved_config(&agent).ok_or(REFUSED)?;
        if config.worktree.as_ref() != Some(meta)
            || config.chat_id.as_deref() != Some(chat.as_str())
        {
            return Err(REFUSED.into());
        }
        let record = app.agents.get_any(&agent).await.map_err(|_| REFUSED)?;
        if record.child_chat_id.as_deref() != Some(chat.as_str())
            || record.worktree_id.as_deref() != Some(meta.id.as_str())
            || record.worktree_branch != meta.branch
            || record.status.is_terminal()
        {
            return Err(REFUSED.into());
        }
        return Ok(
            json!({"agent":agent,"generation":record.run_generation,"chat":chat,"worktree":config.worktree,"boundary":boundary}),
        );
    }
    if depth != 0 {
        return Err(REFUSED.into());
    }
    let session = app
        .chat
        .sessions
        .read()
        .await
        .get(&chat)
        .cloned()
        .ok_or(REFUSED)?;
    let session = session.lock().await;
    if session.closed || session.thread.worktree.as_ref() != Some(meta) {
        return Err(REFUSED.into());
    }
    let task = session.thread.task_meta.clone();
    let source = session.work_project_root.clone();
    drop(session);
    let admitted = if meta.kind == "task_agent" {
        let task = task.as_ref().ok_or(REFUSED)?;
        if task.role != "agents"
            || Some(task.task_id.as_str()) != meta.task_id.as_deref()
            || task.card_id != meta.card_id
            || task.agent_id.as_deref().is_none_or(str::is_empty)
            || task.planner_chat_id.as_deref().is_none_or(str::is_empty)
            || source
                .as_ref()
                .is_some_and(|source| source != &meta.source_workspace_root)
        {
            return Err(REFUSED.into());
        }
        let (board, _) =
            crate::tools::command_checkout::read_board(&meta.source_workspace_root, &task.task_id)
                .await?;
        let card = board
            .get_card(task.card_id.as_deref().ok_or(REFUSED)?)
            .ok_or(REFUSED)?;
        if card.column != "doing"
            || card.paused_at.is_some()
            || card.assignee != task.agent_id
            || card.agent_chat_id.as_deref() != Some(chat.as_str())
            || card.agent_worktree_name.as_deref() != Some(meta.id.as_str())
            || board.owner_chat_id != task.planner_chat_id
        {
            return Err(REFUSED.into());
        }
        let execution =
            crate::tasks::admission::current_execution(&app, &chat).map_err(|_| REFUSED)?;
        match purpose {
            ValidationPurpose::Execution if !execution.holds_permit() => {
                return Err(REFUSED.into());
            }
            ValidationPurpose::Pending(proof) => proof.check(ccx).await?,
            ValidationPurpose::Execution => {}
        }
        Some(execution.admission_generation())
    } else {
        None
    };
    Ok(json!({"chat":chat,"worktree":meta,"task":task,"admission":admitted,"boundary":boundary}))
}

fn read_input(
    root: &Path,
    path: &Path,
    remaining: &mut u64,
) -> Result<Option<Vec<u8>>, CacheReason> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::{AsRawFd, FromRawFd};
        use std::os::unix::ffi::OsStrExt;
        use std::os::unix::fs::OpenOptionsExt;
        let mut directory = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
            .open("/")
            .map_err(|_| CacheReason::UnsafePath)?;
        if !path.is_absolute() || !path.starts_with(root) {
            return Err(CacheReason::UnsafePath);
        }
        let parts: Vec<_> = path
            .components()
            .filter(|part| !matches!(part, std::path::Component::RootDir))
            .collect();
        for (index, part) in parts.iter().enumerate() {
            let std::path::Component::Normal(name) = part else {
                return Err(CacheReason::UnsafePath);
            };
            let name =
                std::ffi::CString::new(name.as_bytes()).map_err(|_| CacheReason::UnsafePath)?;
            let last = index + 1 == parts.len();
            let flags = libc::O_RDONLY
                | libc::O_NOFOLLOW
                | libc::O_CLOEXEC
                | libc::O_NONBLOCK
                | if last { 0 } else { libc::O_DIRECTORY };
            let fd = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags) };
            if fd < 0 {
                return if std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
                    Ok(None)
                } else {
                    Err(CacheReason::UnsafePath)
                };
            }
            directory = unsafe { std::fs::File::from_raw_fd(fd) };
        }
        let meta = directory.metadata().map_err(|_| CacheReason::IoFailure)?;
        if !meta.is_file() || meta.len() >= *remaining {
            return Err(CacheReason::LimitExceeded);
        }
        let mut bytes = Vec::new();
        directory
            .take(*remaining)
            .read_to_end(&mut bytes)
            .map_err(|_| CacheReason::IoFailure)?;
        if bytes.len() as u64 >= *remaining {
            return Err(CacheReason::LimitExceeded);
        }
        *remaining -= bytes.len() as u64;
        Ok(Some(bytes))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (root, path, remaining);
        Err(CacheReason::UnsupportedPlatform)
    }
}

fn checkout_binding(meta: &WorktreeMeta) -> Result<String, CacheReason> {
    let repo = crate::worktrees::git::discover_repo(&meta.root)
        .map_err(|_| CacheReason::IdentityChanged)?;
    let head = repo
        .head()
        .ok()
        .and_then(|head| head.target())
        .ok_or(CacheReason::IdentityChanged)?;
    #[cfg(target_os = "linux")]
    let physical = {
        use std::os::unix::fs::MetadataExt;
        [meta.root.as_path(), repo.path()]
            .into_iter()
            .map(|path| {
                std::fs::metadata(path)
                    .map(|metadata| (metadata.dev(), metadata.ino()))
                    .map_err(|_| CacheReason::IdentityChanged)
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    #[cfg(not(target_os = "linux"))]
    let physical: Vec<(u64, u64)> = Vec::new();
    Ok(crate::file_activity::digest_of(
        &json!({"meta":meta,"head":head.to_string(),"physical":physical}).to_string(),
    ))
}

fn input_binding(
    meta: &WorktreeMeta,
    request: &ExecSpawnRequest,
    gcx: &Arc<crate::global_context::GlobalContext>,
) -> Result<String, CacheReason> {
    let cwd = request.cwd.as_deref().ok_or(CacheReason::UnsafePath)?;
    if dunce::canonicalize(cwd).ok().as_deref() != Some(cwd)
        || !cwd.starts_with(&meta.root)
        || cwd.ancestors().count() > 48
    {
        return Err(CacheReason::UnsafePath);
    }
    let checkout = checkout_binding(meta)?;
    let mut paths = vec![
        meta.root.join("refact-agent/engine/Cargo.toml"),
        meta.root.join("refact-agent/engine/Cargo.lock"),
        meta.root.join("refact-agent/engine/.cargo/config"),
        meta.root.join("refact-agent/engine/.cargo/config.toml"),
    ];
    let mut external_config = BTreeMap::new();
    for dir in cwd.ancestors().take(48) {
        if !dir.starts_with(&meta.root) {
            for name in [".cargo/config", ".cargo/config.toml"] {
                match std::fs::symlink_metadata(dir.join(name)) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Ok(_) => {
                        external_config.insert(dir.join(name), true);
                    }
                    Err(_) => return Err(CacheReason::InputMismatch),
                }
            }
            continue;
        }
        paths.push(dir.join(".cargo/config"));
        paths.push(dir.join(".cargo/config.toml"));
    }
    if let Some(home) = request.env.get("HOME") {
        for name in [".cargo/config", ".cargo/config.toml"] {
            match std::fs::symlink_metadata(Path::new(home).join(name)) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Ok(_) => {
                    external_config.insert(Path::new(home).join(name), true);
                }
                Err(_) => return Err(CacheReason::InputMismatch),
            }
        }
    }
    paths.sort();
    paths.dedup();
    let mut remaining = MAX_INPUT_BYTES;
    let mut inputs = BTreeMap::new();
    let privacy = gcx
        .privacy_settings
        .read()
        .map_err(|_| CacheReason::InputMismatch)?
        .clone();
    for path in paths {
        crate::privacy::check_file_privacy(
            privacy.clone(),
            &path,
            &crate::privacy::FilePrivacyLevel::AllowToSendAnywhere,
        )
        .map_err(|_| CacheReason::InputMismatch)?;
        let bytes = read_input(&meta.root, &path, &mut remaining)?;
        inputs.insert(
            path,
            bytes.map(|bytes| crate::file_activity::digest_of(&hex::encode(bytes))),
        );
    }
    Ok(crate::file_activity::digest_of(
        &json!({"checkout":checkout,"inputs":inputs,"external_config_present":external_config})
            .to_string(),
    ))
}

const NESTED_REFUSED: &str = "Nested subagent command execution is not supported for this task checkout. Ask the owning card agent to run this command; no command was started.";

async fn refuse_nested_task_command(
    ccx: &Arc<Mutex<AtCommandsContext>>,
    meta: &WorktreeMeta,
) -> Result<(), String> {
    let (app, chat, background, depth) = {
        let context = ccx.lock().await;
        (
            context.app.clone(),
            context.chat_id.clone(),
            context.background_agent_id.clone(),
            context.subchat_depth,
        )
    };
    if meta.enforce
        && meta.kind == "task_agent"
        && (background.is_some()
            || depth != 0
            || app
                .agents
                .find_agent_id_by_child_chat_id(&chat)
                .await
                .is_some())
    {
        return Err(NESTED_REFUSED.into());
    }
    Ok(())
}

pub(crate) async fn ensure_managed_task_execution(
    ccx: &Arc<Mutex<AtCommandsContext>>,
    meta: &WorktreeMeta,
) -> Result<(), String> {
    refuse_nested_task_command(ccx, meta).await?;
    let (app, chat, task) = {
        let context = ccx.lock().await;
        if context
            .abort_flag
            .load(std::sync::atomic::Ordering::Relaxed)
            || context
                .app
                .runtime
                .shutdown_flag
                .load(std::sync::atomic::Ordering::Relaxed)
            || context
                .app
                .agent_executions
                .get(&context.chat_id)
                .is_some_and(|run| run.is_cancelled())
        {
            return Err("command approval cancelled before execution".into());
        }
        (
            context.app.clone(),
            context.chat_id.clone(),
            context.task_meta.clone(),
        )
    };
    let session = app
        .chat
        .sessions
        .read()
        .await
        .get(&chat)
        .cloned()
        .ok_or(REFUSED)?;
    if session.lock().await.thread.task_meta != task {
        return Err(REFUSED.into());
    }
    let execution = crate::tasks::admission::current_execution(&app, &chat)
        .map_err(|_| "managed task execution unavailable before command execution")?;
    if !execution.holds_permit() {
        return Err("managed task execution requires a held current permit".into());
    }
    let target = crate::agents::approval_target::ApprovalTarget::from_context(&app, ccx).await?;
    if target.chat_id != chat
        || target.generation != execution.admission_generation()
        || !target.work_id.starts_with("board:")
    {
        return Err(REFUSED.into());
    }
    accepted_identity(ccx, meta, false, ValidationPurpose::Execution).await?;
    Ok(())
}

pub(crate) async fn prepare_managed_cache(
    ccx: &Arc<Mutex<AtCommandsContext>>,
    checkout: Option<&ValidatedCommandCheckout>,
    request: &mut ExecSpawnRequest,
) -> Result<Option<ManagedCacheDecision>, String> {
    prepare_with_purpose(ccx, checkout, request, ValidationPurpose::Execution).await
}

pub(crate) async fn validate_pending_managed_cache(
    ccx: &Arc<Mutex<AtCommandsContext>>,
    checkout: Option<&ValidatedCommandCheckout>,
    mut request: ExecSpawnRequest,
    reason: &str,
    proof: &crate::agents::command_approval::PendingCommandValidation,
) -> Result<refact_work_core::ExecutionApproval, String> {
    proof.check(ccx).await?;
    let purpose = ValidationPurpose::Pending(proof);
    let cache = prepare_with_purpose(ccx, checkout, &mut request, purpose).await?;
    crate::agents::command_approval::freeze(&mut request)?;
    let mut approval = crate::agents::command_approval::descriptor(&request, reason)?;
    if let Some(checkout) = checkout {
        checkout.bind_approval(&mut approval, &request)?;
    }
    if let Some(cache) = cache {
        cache
            .revalidate_with_purpose(ccx, &request, purpose)
            .await?;
        cache.bind_approval(&mut approval);
    }
    proof.check(ccx).await?;
    Ok(approval)
}

async fn prepare_with_purpose(
    ccx: &Arc<Mutex<AtCommandsContext>>,
    checkout: Option<&ValidatedCommandCheckout>,
    request: &mut ExecSpawnRequest,
    purpose: ValidationPurpose<'_>,
) -> Result<Option<ManagedCacheDecision>, String> {
    let meta = if let Some(checkout) = checkout {
        Some(checkout.metadata().clone())
    } else {
        ccx.lock()
            .await
            .execution_scope
            .as_ref()
            .filter(|scope| scope.is_enforced())
            .map(|scope| scope.worktree().clone())
    };
    let Some(meta) = meta else { return Ok(None) };
    refuse_nested_task_command(ccx, &meta).await?;
    if request
        .cwd
        .as_ref()
        .is_none_or(|cwd| !cwd.starts_with(&meta.root))
    {
        return Ok(None);
    }
    let identity = match accepted_identity(ccx, &meta, checkout.is_some(), purpose).await {
        Ok(identity) => identity,
        Err(_) => return Ok(None),
    };
    let gcx = ccx.lock().await.app.gcx.clone();
    let service =
        match WorktreeService::new_async(gcx.cache_dir.clone(), meta.source_workspace_root.clone())
            .await
        {
            Ok(service)
                if service.worktree_path_for_id(&meta.id).ok().as_ref() == Some(&meta.root) =>
            {
                service
            }
            _ => return Ok(None),
        };
    crate::agents::command_approval::freeze(request)?;
    let environment = environment_binding(&request.env);
    let context = service.cache_execution_context(&meta).await;
    let chat = ccx.lock().await.chat_id.clone();
    let mode = crate::exec::command_policy::chat_mode_for_exec(gcx.clone(), &chat).await;
    let known_mode = matches!(
        mode.as_deref(),
        Some("agent" | "task_agent" | "task_planner" | "explore" | "no_tools" | "chat")
    );
    let writable = known_mode
        && (matches!(
            mode.as_deref(),
            Some("agent" | "task_agent" | "task_planner")
        ) || request
            .sandbox
            .as_ref()
            .is_some_and(|sandbox| sandbox.mode != ExecSandboxMode::ReadOnly));
    let explicitly_unconfined = matches!(
        gcx.terminal_security_config.mode,
        crate::global_context::TerminalSecurityMode::Off
            | crate::global_context::TerminalSecurityMode::ApprovalOnly
            | crate::global_context::TerminalSecurityMode::Audit
    );
    let checked_meta = meta.clone();
    let checkout_binding = tokio::task::spawn_blocking(move || checkout_binding(&checked_meta))
        .await
        .map_err(|_| REFUSED)?
        .map_err(|_| REFUSED)?;
    let check_meta = meta.clone();
    let check_request = request.clone();
    let (context, inputs, target) = tokio::task::spawn_blocking(move || {
        let inputs = context
            .as_ref()
            .map_err(|reason| *reason)
            .and_then(|_| input_binding(&check_meta, &check_request, &gcx));
        let target = context
            .as_ref()
            .map_err(|reason| *reason)
            .and_then(|context| {
                let target = context.cargo_target()?;
                if !writable || (check_request.sandbox.is_none() && !explicitly_unconfined) {
                    return Err(CacheReason::UnverifiedIsolation);
                }
                target_policy(&check_request, &target)?;
                let privacy = gcx
                    .privacy_settings
                    .read()
                    .map_err(|_| CacheReason::InputMismatch)?
                    .clone();
                crate::privacy::check_file_privacy(
                    privacy,
                    &target,
                    &crate::privacy::FilePrivacyLevel::AllowToSendAnywhere,
                )
                .map_err(|_| CacheReason::InputMismatch)?;
                inputs.as_ref().map_err(|reason| *reason)?;
                context.validate_current()?;
                Ok(target)
            });
        (context.ok().map(Arc::new), inputs, target)
    })
    .await
    .map_err(|_| REFUSED)?;
    let report = json!({
        "schema_version":1,
        "cargo_target": match &target {
            Ok(_) => component(CacheOutcome::Applied, CacheReason::CheckoutLocal),
            Err(reason) => component(CacheOutcome::Degraded, *reason),
        },
        "compiler_cache":component(CacheOutcome::Degraded, CacheReason::UnsafeDaemonBoundary),
    });
    let inputs = inputs.unwrap_or_else(|reason| json!(reason).to_string());
    let target = target.ok();
    if let Some(target) = &target {
        request.env.insert(
            "CARGO_TARGET_DIR".into(),
            target.to_str().ok_or(REFUSED)?.into(),
        );
    }
    let binding = crate::file_activity::digest_of(
        &json!({"identity":identity,"inputs":inputs,"checkout":checkout_binding,"target":target,"report":report}).to_string(),
    );
    let execution = crate::agents::command_approval::descriptor(request, "")?.fingerprint;
    Ok(Some(ManagedCacheDecision {
        context,
        meta,
        identity,
        inputs,
        checkout_binding,
        binding,
        execution,
        environment,
        target,
        report,
        checkout: checkout.is_some(),
        executable: matches!(purpose, ValidationPurpose::Execution),
    }))
}

impl ManagedCacheDecision {
    pub(crate) fn bind_approval(&self, approval: &mut refact_work_core::ExecutionApproval) {
        approval.fingerprint = crate::file_activity::digest_of(
            &json!({"execution":approval.fingerprint,"managed_cache":self.binding}).to_string(),
        );
    }

    pub(crate) fn attach_report(&self, extra: &mut serde_json::Map<String, Value>) {
        if let Some(exec) = extra.get_mut("exec").and_then(Value::as_object_mut) {
            exec.insert("cache_report".into(), self.report.clone());
        }
    }

    pub(crate) async fn revalidate(
        &self,
        ccx: &Arc<Mutex<AtCommandsContext>>,
        request: &ExecSpawnRequest,
    ) -> Result<(), String> {
        if !self.executable {
            return Err(REFUSED.into());
        }
        self.revalidate_with_purpose(ccx, request, ValidationPurpose::Execution)
            .await
    }

    async fn revalidate_with_purpose(
        &self,
        ccx: &Arc<Mutex<AtCommandsContext>>,
        request: &ExecSpawnRequest,
        purpose: ValidationPurpose<'_>,
    ) -> Result<(), String> {
        #[cfg(test)]
        let _ = BEFORE_SPAWN.try_with(|hook| {
            if let Some(change) = hook.borrow_mut().take() {
                change();
            }
        });
        if accepted_identity(ccx, &self.meta, self.checkout, purpose).await? != self.identity
            || crate::agents::command_approval::descriptor(request, "")?.fingerprint
                != self.execution
        {
            return Err(REFUSED.into());
        }
        let gcx = ccx.lock().await.app.gcx.clone();
        let mut errors = Vec::new();
        let current_env = crate::integrations::setting_up_integrations::get_vars_for_replacements(
            gcx.clone(),
            &mut errors,
        )
        .await;
        let policy = crate::exec::ExecEnvPolicy::Scrubbed {
            passthrough: gcx.terminal_security_config.env_passthrough.clone(),
        };
        let current_env = refact_exec::build_child_env(&policy, &current_env)
            .into_iter()
            .collect();
        if !errors.is_empty() || environment_binding(&current_env) != self.environment {
            return Err(REFUSED.into());
        }
        let request = request.clone();
        let context = self.context.clone();
        let meta = self.meta.clone();
        let target = self.target.clone();
        let inputs = self.inputs.clone();
        let expected_checkout = self.checkout_binding.clone();
        tokio::task::spawn_blocking(move || {
            if checkout_binding(&meta).map_err(|_| REFUSED)? != expected_checkout {
                return Err(REFUSED.to_string());
            }
            if let Some(context) = &context {
                context.validate_current().map_err(|_| REFUSED)?;
                if let Some(target) = &target {
                    if context.cargo_target().map_err(|_| REFUSED)? != *target {
                        return Err(REFUSED.to_string());
                    }
                }
            }
            let current = context
                .as_ref()
                .map(|_| {
                    input_binding(&meta, &request, &gcx)
                        .unwrap_or_else(|reason| json!(reason).to_string())
                })
                .unwrap_or_else(|| inputs.clone());
            if current != inputs {
                return Err(REFUSED.to_string());
            }
            if let Some(context) = &context {
                context.validate_current().map_err(|_| REFUSED)?;
            }
            Ok(())
        })
        .await
        .map_err(|_| REFUSED)??;
        if accepted_identity(ccx, &self.meta, self.checkout, purpose).await? != self.identity {
            return Err(REFUSED.into());
        }
        Ok(())
    }
}
